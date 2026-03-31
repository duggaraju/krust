
#![no_std]
#![no_main]

mod vga_buffer;

use core::panic::PanicInfo;
use bootloader_api::{BootInfo, entry_point, info};
use bootloader_api::info::Optional;
use bootloader_boot_config::LevelFilter;
use bootloader_x86_64_common::init_logger;
use log::{error, info};
use x86_64::instructions::port::{PortGeneric, ReadWriteAccess};

entry_point!(kernel_main);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum QemuExitCode {
    Success = 0x10,
    Failed = 0x11,
}

pub fn exit_qemu(exit_code: QemuExitCode) -> ! {
    use x86_64::instructions::{nop, port::Port};

    unsafe {
        let mut port: PortGeneric<u32, ReadWriteAccess> = Port::new(0xf4);
        info!("Exiting QEMU with code: {:?}", exit_code);
        port.write(exit_code as u32);
    }

    loop {
        info!("Halting CPU...");
        x86_64::instructions::hlt();
    }
}

fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    if let Optional::Some(ref mut framebuffer) = boot_info.framebuffer {
        // Initialize the framebuffer
        // Fallback to a simple logger
        let info = framebuffer.info();
        init_logger( framebuffer.buffer_mut(), info, LevelFilter::Info, true, true);
    }
    info!("Starting kernel...");
    info!("Boot info version: {:?} ", boot_info.api_version);
    vga_buffer::print_something();
    exit_qemu(QemuExitCode::Success);
}

#[panic_handler]
#[cfg(not(test))]
fn panic(info: &PanicInfo) -> ! {
    error!("Kernel panic: {:#?}", info);
    exit_qemu(QemuExitCode::Failed);
}

