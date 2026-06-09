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
#[cfg(feature = "syscall")]
pub mod syscall;
pub mod tty;

use bootloader_api::config::Mapping;
use bootloader_api::info::Optional;
use bootloader_api::{BootInfo, BootloaderConfig, entry_point};
use core::panic::PanicInfo;
use log::{error, info};
use x86_64::instructions::port::{PortGeneric, ReadWriteAccess};

pub static BOOTLOADER_CONFIG: BootloaderConfig = {
    let mut config = BootloaderConfig::new_default();
    config.mappings.physical_memory = Some(Mapping::Dynamic);
    config
};

entry_point!(kernel_main, config = &BOOTLOADER_CONFIG);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum QemuExitCode {
    Success = 0x10,
    Failed = 0x11,
}

pub fn exit_qemu(exit_code: QemuExitCode) -> ! {
    use x86_64::instructions::port::Port;

    unsafe {
        let mut port: PortGeneric<u32, ReadWriteAccess> = Port::new(0xf4);
        info!("Exiting QEMU with code: {:?}", exit_code);
        port.write(exit_code as u32);
    }

    loop {
        x86_64::instructions::hlt();
    }
}

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
    tty::set_port(boot_config.shell_port());

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
        Some(0x3F8)
    } else if framebuffer_logger.is_some() {
        None
    } else {
        Some(0x2F8)
    };

    logger::init(framebuffer_logger, trace_serial, boot_config.to_log_level());

    info!("Starting krust kernel...");
    info!("Boot info version: {:?}", boot_info.api_version);
    info!("Boot log level: {:?}", boot_config.log_level);

    // Initialize architecture (GDT, IDT)
    arch::x86_64::init();

    // Initialize process management
    #[cfg(feature = "process")]
    process::init();

    // Initialize filesystem
    #[cfg(feature = "fs")]
    fs::init();

    // Initialize syscall interface
    #[cfg(feature = "syscall")]
    syscall::init();

    // Initialize device drivers
    #[cfg(feature = "drivers")]
    drivers::init();

    // Initialize module subsystem
    #[cfg(feature = "modules")]
    module::init();

    info!("Kernel initialization complete.");

    // Start the kernel shell (never returns)
    #[cfg(feature = "shell")]
    {
        shell::run(crate::shell::ConsoleTarget::Serial);
    }

    // If shell is disabled, just show VGA output and exit
    #[cfg(not(feature = "shell"))]
    {
        #[cfg(feature = "drivers")]
        drivers::video::print_something();
        exit_qemu(QemuExitCode::Success);
    }
}

#[panic_handler]
#[cfg(not(test))]
fn panic(info: &PanicInfo) -> ! {
    error!("Kernel panic: {:#?}", info);
    exit_qemu(QemuExitCode::Failed);
}
