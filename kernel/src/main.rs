#![no_std]
#![no_main]
#![feature(allocator_api)]
#![cfg_attr(feature = "arch-x86_64", feature(abi_x86_interrupt))]

extern crate alloc;

pub mod arch;

pub mod boot_config;
#[path = "../../boot_settings.rs"]
pub mod boot_settings;

#[cfg(feature = "drivers")]
pub mod drivers;
#[cfg(feature = "fs")]
pub mod fs;
pub mod logger;

pub mod mm;
#[cfg(feature = "modules")]
pub mod module;

pub mod process;
#[cfg(feature = "shell")]
pub mod shell;

pub mod syscall;
pub mod time;
use core::panic::PanicInfo;
use log::info;

#[cfg(feature = "boot-bios")]
use bootloader_api::config::Mapping;
#[cfg(feature = "boot-bios")]
use bootloader_api::{BootInfo, BootloaderConfig, entry_point};

#[cfg(feature = "boot-bios")]
pub static BOOTLOADER_CONFIG: BootloaderConfig = {
    let mut config = BootloaderConfig::new_default();
    config.mappings.physical_memory = Some(Mapping::Dynamic);
    config
};

#[cfg(feature = "boot-bios")]
entry_point!(kernel_main, config = &BOOTLOADER_CONFIG);

#[cfg(feature = "boot-bios")]
fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    let framebuffer = core::mem::replace(
        &mut boot_info.framebuffer,
        bootloader_api::info::Optional::None,
    );
    let mut framebuffer = match framebuffer {
        bootloader_api::info::Optional::Some(framebuffer) => Some(framebuffer),
        bootloader_api::info::Optional::None => None,
    };

    let physical_memory_offset = boot_info
        .physical_memory_offset
        .as_ref()
        .copied()
        .expect("bootloader did not provide a physical memory offset");
    let memory_regions = unsafe {
        core::slice::from_raw_parts(
            boot_info.memory_regions.as_ptr(),
            boot_info.memory_regions.len(),
        )
    };

    {
        mm::init_with(physical_memory_offset, memory_regions);
    }

    let boot_config = boot_config::BootConfig::from_boot_info(boot_info);
    let use_framebuffer_console = match boot_config.shell_console() {
        boot_config::ShellConsole::Framebuffer => framebuffer.is_some(),
        boot_config::ShellConsole::Serial => false,
        boot_config::ShellConsole::Auto => framebuffer.is_some(),
    };
    let framebuffer_logger = None;
    let trace_serial = if matches!(
        boot_config.shell_console(),
        boot_config::ShellConsole::Serial
    ) {
        // Serial is the shell console — traces share the same port.
        Some(0x3F8)
    } else if use_framebuffer_console {
        // Framebuffer UI is active: route kernel traces to COM1 and suppress
        // the tty layer's own serial echo so shell output stays on the screen.
        #[cfg(feature = "drivers")]
        crate::drivers::tty::set_serial_echo(false);
        Some(0x3F8)
    } else {
        Some(0x2F8)
    };

    logger::init(framebuffer_logger, trace_serial, boot_config.log_level);

    info!("Starting krust kernel...");
    #[cfg(feature = "arch-x86_64")]
    info!("Boot info version: {:?}", boot_info.api_version);
    info!("Boot log level: {:?}", boot_config.log_level);

    // Initialize architecture (GDT, IDT)
    arch::init();
    time::init();

    // Initialize process management

    process::init();

    // Initialize filesystem
    #[cfg(feature = "fs")]
    fs::init();

    // Initialize syscall interface
    syscall::init(boot_config.syscall_trace());

    // Initialize device drivers
    #[cfg(feature = "drivers")]
    drivers::init(boot_config.virtual_consoles());

    // Wire the ttyS serial device into the tty echo/input layer.
    #[cfg(feature = "drivers")]
    {
        let echo_name = if boot_config.shell_port() == 2 {
            "ttyS1"
        } else {
            "ttyS0"
        };
        if let Some(dev) = crate::drivers::registry::get(echo_name) {
            crate::drivers::tty::set_serial_device(dev);
        }
    }

    // Initialize module subsystem
    #[cfg(feature = "modules")]
    module::init();

    #[cfg(feature = "fs")]
    {
        if let Err(error) = fs::boot::mount_filesystems(&boot_config.settings, boot_info) {
            panic!("boot mounts failed: {error}");
        }
    }

    info!("Kernel initialization complete.");

    {
        let root_cwd_inode = {
            #[cfg(feature = "fs")]
            {
                crate::fs::vfs::root_inode().expect("root inode must exist before init startup")
            }
            #[cfg(not(feature = "fs"))]
            {
                unreachable!("process feature requires fs for init startup")
            }
        };

        let init_path = match boot_config.settings.init.as_deref() {
            Some(path) => Some(alloc::string::String::from(path)),
            None => {
                #[cfg(feature = "fs")]
                {
                    match fs::vfs::lookup_path("/sbin/init") {
                        Ok(_) => Some(alloc::string::String::from("/sbin/init")),
                        Err(fs::vfs::FsError::NotFound) => None,
                        Err(error) => panic!("init: cannot check /sbin/init: {error:?}"),
                    }
                }
                #[cfg(not(feature = "fs"))]
                {
                    None::<alloc::string::String>
                }
            }
        };
        process::register_boot_processes(root_cwd_inode, init_path.is_none());

        process::set_current_pid(crate::process::ROOT_PID);
        process::resume_cpu_accounting();

        // Init framebuffer console output if needed.
        #[cfg(feature = "drivers")]
        if use_framebuffer_console {
            if let Some(framebuffer) = &mut framebuffer {
                let info = framebuffer.info();
                let buffer = framebuffer.buffer_mut();
                let buffer =
                    unsafe { core::slice::from_raw_parts_mut(buffer.as_mut_ptr(), buffer.len()) };
                crate::drivers::video::init_framebuffer_console(buffer, info);
            }
        }

        // Assign the controlling terminal before exec so PID 1 inherits stdio.
        // Framebuffer mode → tty0 (virtual console); serial mode → ttyS0.
        #[cfg(feature = "drivers")]
        {
            let ctty = if use_framebuffer_console {
                crate::process::task::ControllingTerminal::VirtualConsole(0)
            } else {
                crate::process::task::ControllingTerminal::Serial(
                    boot_config.shell_port().saturating_sub(1) as usize,
                )
            };
            process::set_controlling_terminal(crate::process::ROOT_PID, ctty);
        }

        if let Some(path) = init_path {
            let envp = process::with_current_task_mut(|task| {
                task.env
                    .iter()
                    .map(|(key, value)| alloc::format!("{key}={value}"))
                    .collect()
            })
            .expect("PID 1 must be registered");
            info!("init: starting '{}' as PID 1", path);
            if let Err(errno) =
                syscall::impls::execve_with_args(&path, alloc::vec![path.clone()], envp)
            {
                panic!("init: failed to execute '{path}' as PID 1: errno {errno}");
            }
            unreachable!("successful init exec does not return");
        }

        #[cfg(feature = "shell")]
        {
            info!("init: /sbin/init not found; starting kernel shell as PID 1");
            let _reason = shell::run();
            if process::terminate_root_process() {
                info!("PID 1 kernel shell exited; shutting down kernel");
                arch::shutdown(arch::ShutdownStatus::Success);
            }
            arch::shutdown(arch::ShutdownStatus::Success);
        }

        #[cfg(not(feature = "shell"))]
        panic!("init: /sbin/init not found and kernel shell is disabled");
    }
}

#[panic_handler]
#[cfg(not(test))]
fn panic(info: &PanicInfo) -> ! {
    use log::error;
    error!("Kernel panic: {:#?}", info);

    {
        let current_pid = crate::process::current_pid();
        error!("panic trace: current_pid={}", current_pid);

        let scheduler = crate::process::scheduler::SCHEDULER.lock();
        if let Some(scheduler) = scheduler.as_ref() {
            error!(
                "panic trace: scheduler_current_pid={:?}",
                scheduler.current().map(|task| task.pid)
            );

            if let Some(task) = scheduler.task_by_pid(current_pid) {
                error!(
                    "panic trace: task pid={} ppid={} state={:?} mode={:?} priority={} userland={} exit_code={} exec_path='{}' cwd='{}' argv={:?}",
                    task.pid,
                    task.parent_pid,
                    task.state,
                    task.mode,
                    task.priority,
                    task.userland,
                    task.exit_code,
                    task.exec_path,
                    task.cwd_path,
                    task.argv
                );
                error!("panic trace: context={:#?}", task.context);
                error!(
                    "panic trace: pending_signals={} terminated_by_signal={:?} kernel_stack_top=0x{:x} user_stack_top=0x{:x}",
                    task.pending_signals.len(),
                    task.terminated_by_signal,
                    task.kernel_stack_top,
                    task.user_stack_top
                );
            } else {
                error!("panic trace: no task found for current_pid={}", current_pid);
            }

            for task in scheduler.tasks().take(8) {
                error!(
                    "panic trace: task_list pid={} state={:?} mode={:?} userland={} exec='{}'",
                    task.pid, task.state, task.mode, task.userland, task.exec_path
                );
            }
        } else {
            error!("panic trace: scheduler unavailable");
        }
    }
    arch::shutdown(arch::ShutdownStatus::Failure);
}
