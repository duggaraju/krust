#![no_std]
#![no_main]
#![cfg_attr(feature = "arch-x86_64", feature(abi_x86_interrupt))]

extern crate alloc;

pub mod arch;

pub mod boot_config;

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
        let _ = crate::fs::register_mount("/", "ramfs", crate::fs::MountDevice::None);
        // Keep ramfs as bootstrap root, then overlay FAT as the runtime root.
        #[cfg(feature = "drivers")]
        let _ = crate::fs::register_mount(
            "/bin",
            "fatfs",
            crate::fs::MountDevice::device_path("/dev/sda"),
        );

        let _ = crate::fs::register_mount("/proc", "proc", crate::fs::MountDevice::None);
        #[cfg(feature = "drivers")]
        let _ = crate::fs::register_mount("/dev", "dev", crate::fs::MountDevice::None);

        if let Err(err) = fs::mount_registered_filesystems() {
            log::error!("failed to mount registered filesystems: {:?}", err);
        }
    }

    info!("Kernel initialization complete.");

    // Start the kernel shell.
    #[cfg(feature = "shell")]
    {
        let root_cwd_inode = {
            #[cfg(feature = "fs")]
            {
                crate::fs::vfs::root_inode().expect("root inode must exist before shell startup")
            }
            #[cfg(not(feature = "fs"))]
            {
                unreachable!("process feature requires fs for shell startup")
            }
        };

        process::register_boot_processes(root_cwd_inode);

        process::set_current_pid(crate::process::SHELL_PID);

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

        // Assign the controlling terminal for the shell process.
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
            process::set_controlling_terminal(crate::process::SHELL_PID, ctty);
        }

        let _reason = shell::run();

        {
            process::mark_shell_exited();
            if process::terminate_root_process() {
                info!("root process terminated; shutting down kernel");
                arch::shutdown(arch::ShutdownStatus::Success);
            }
        }

        arch::shutdown(arch::ShutdownStatus::Success);
    }

    // If shell is disabled, just show VGA output and exit
    #[cfg(not(feature = "shell"))]
    {
        #[cfg(feature = "drivers")]
        drivers::video::print_something();
        arch::shutdown(arch::ShutdownStatus::Success);
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
