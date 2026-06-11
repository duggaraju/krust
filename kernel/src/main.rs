#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]

#[cfg(feature = "mm")]
extern crate alloc;

pub mod arch;

pub mod boot_config;
#[cfg(feature = "drivers")]
pub mod drivers;
#[cfg(feature = "fs")]
pub mod fs;
pub mod logger;
#[cfg(feature = "mm")]
pub mod mm;
#[cfg(feature = "modules")]
pub mod module;
#[cfg(feature = "process")]
pub mod process;
#[cfg(feature = "shell")]
pub mod shell;
pub mod syscall;
pub mod time;

use bootloader_api::config::Mapping;
use bootloader_api::info::Optional;
use bootloader_api::{BootInfo, BootloaderConfig, entry_point};
use core::panic::PanicInfo;
use log::info;
use log::debug;

pub static BOOTLOADER_CONFIG: BootloaderConfig = {
    let mut config = BootloaderConfig::new_default();
    config.mappings.physical_memory = Some(Mapping::Dynamic);
    config
};

entry_point!(kernel_main, config = &BOOTLOADER_CONFIG);

fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    // Move framebuffer out so we don't keep borrowing `boot_info`.
    let mut framebuffer = core::mem::replace(&mut boot_info.framebuffer, Optional::None);

    #[cfg(feature = "mm")]
    {
        let physical_memory_offset = boot_info
            .physical_memory_offset
            .as_ref()
            .copied()
            .expect("bootloader did not provide a physical memory offset");
        let memory_regions = unsafe {
            // SAFETY: bootloader memory regions live for kernel lifetime
            core::slice::from_raw_parts(
                boot_info.memory_regions.as_ptr(),
                boot_info.memory_regions.len(),
            )
        };
        mm::init_with(physical_memory_offset, memory_regions);
    }

    let boot_config = boot_config::BootConfig::from_boot_info(boot_info);

    let framebuffer_logger = match boot_config.shell_console() {
        boot_config::ShellConsole::Auto => {
            if let Optional::Some(framebuffer) = &mut framebuffer {
                #[cfg(feature = "drivers")]
                {
                    let info = framebuffer.info();
                    let buffer = framebuffer.buffer_mut();
                    let buffer = unsafe {
                        core::slice::from_raw_parts_mut(buffer.as_mut_ptr(), buffer.len())
                    };
                    Some((buffer, info))
                }
                #[cfg(not(feature = "drivers"))]
                {
                    let _ = framebuffer;
                    None
                }
            } else {
                None
            }
        }
        boot_config::ShellConsole::Framebuffer => {
            if let Optional::Some(framebuffer) = &mut framebuffer {
                #[cfg(feature = "drivers")]
                {
                    let info = framebuffer.info();
                    let buffer = framebuffer.buffer_mut();
                    let buffer = unsafe {
                        core::slice::from_raw_parts_mut(buffer.as_mut_ptr(), buffer.len())
                    };
                    Some((buffer, info))
                }
                #[cfg(not(feature = "drivers"))]
                {
                    let _ = framebuffer;
                    None
                }
            } else {
                None
            }
        }
        boot_config::ShellConsole::Serial => None,
    };

    let trace_serial = if matches!(
        boot_config.shell_console(),
        boot_config::ShellConsole::Serial
    ) {
        // Serial is the shell console — traces share the same port.
        Some(0x3F8)
    } else if framebuffer_logger.is_some() {
        // Framebuffer UI is active: route kernel traces to COM1 and suppress
        // the tty layer's own serial echo so shell output stays on the screen.
        #[cfg(feature = "drivers")]
        crate::drivers::tty::set_serial_echo(false);
        Some(0x3F8)
    } else {
        Some(0x2F8)
    };

    logger::init(framebuffer_logger, trace_serial, boot_config.to_log_level());

    info!("Starting krust kernel...");
    info!("Boot info version: {:?}", boot_info.api_version);
    info!("Boot log level: {:?}", boot_config.log_level);

    // Initialize architecture (GDT, IDT)
    arch::x86_64::init();
    time::init();

    // Initialize process management
    #[cfg(feature = "process")]
    process::init();

    // Initialize filesystem
    #[cfg(feature = "fs")]
    fs::init();

    // Initialize syscall interface
    syscall::init();

    // Initialize device drivers
    #[cfg(feature = "drivers")]
    drivers::init(boot_config.virtual_consoles());

    // Wire the ttyS serial device into the tty echo/input layer.
    #[cfg(feature = "drivers")]
    {
        let echo_name = if boot_config.shell_port() == 2 { "ttyS1" } else { "ttyS0" };
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
        #[cfg(feature = "drivers")]
        let _ = crate::fs::register_mount(
            "/bin",
            "fatfs",
            crate::fs::MountDevice::device_path("/dev/sda"),
        );
        #[cfg(feature = "process")]
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
        #[cfg(feature = "process")]
        let root_cwd_inode = {
            #[cfg(feature = "fs")]
            {
                crate::fs::vfs::root_inode()
                    .expect("root inode must exist before shell startup")
            }
            #[cfg(not(feature = "fs"))]
            {
                unreachable!("process feature requires fs for shell startup")
            }
        };

        #[cfg(feature = "process")]
        process::register_boot_processes(root_cwd_inode);
        #[cfg(feature = "process")]
        process::set_current_pid(crate::process::SHELL_PID);

        // Determine whether the shell console is framebuffer-based.
        let use_framebuffer = match boot_config.shell_console() {
            boot_config::ShellConsole::Framebuffer => true,
            boot_config::ShellConsole::Serial => false,
            boot_config::ShellConsole::Auto => matches!(framebuffer, Optional::Some(_)),
        };

        // Init framebuffer console output if needed.
        #[cfg(feature = "drivers")]
        if use_framebuffer {
            if let Optional::Some(framebuffer) = &mut framebuffer {
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
            let ctty = if use_framebuffer {
                crate::process::task::ControllingTerminal::VirtualConsole(0)
            } else {
                crate::process::task::ControllingTerminal::Serial(
                    boot_config.shell_port().saturating_sub(1) as usize,
                )
            };
            process::set_controlling_terminal(crate::process::SHELL_PID, ctty);
        }

        let reason = shell::run();
        debug!("shell exited: {:?}", reason);

        #[cfg(feature = "process")]
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
    arch::shutdown(arch::ShutdownStatus::Failure);
}
