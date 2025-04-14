
#![no_std]
#![no_main]

use core::panic::PanicInfo;
use bootloader_api::{entry_point, BootInfo};
use bootloader_api::info::Optional;
use bootloader_boot_config::LevelFilter;
use bootloader_x86_64_common::init_logger;

entry_point!(kernel_main);


fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    if let Optional::Some(ref mut framebuffer) = boot_info.framebuffer {
        // Initialize the framebuffer
        // Fallback to a simple logger
        let info = framebuffer.info();
        init_logger( framebuffer.buffer_mut(), info, LevelFilter::Info, true, true);
    }
    log::info!("Starting kernel...");
    log::info!("Boot info version: {:?} ", boot_info.api_version);
    loop {}
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    log::error!("Kernel panic: {:#?}", info);
    loop {}
}

