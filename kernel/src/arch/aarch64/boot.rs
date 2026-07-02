/// ARM64 UEFI boot handler and framebuffer setup.
///
/// This module provides the UEFI entry point for ARM64 systems, queries the
/// Graphics Output Protocol (GOP) for framebuffer access, and transitions to
/// the kernel main function.
use uefi::prelude::*;

/// Boot information passed from UEFI to kernel.
#[derive(Debug, Clone)]
pub struct Arm64BootInfo {
    pub framebuffer_base: *mut u8,
    pub framebuffer_size: usize,
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    pub pixel_format: PixelFormat,
    pub memory_map: MemoryMapInfo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Rgb,
    Bgr,
    Bitmask,
    BltOnly,
}

#[derive(Debug, Clone)]
pub struct MemoryMapInfo {
    pub total_memory: usize,
}

/// UEFI entry point for ARM64.
///
/// This function is called by the UEFI firmware. It initializes the GOP,
/// queries memory information, and transitions to the kernel.
///
/// The `#[entry]` macro automatically provides initialization.
#[entry]
fn efi_main() -> Status {
    // TODO: Implement UEFI boot handler
    // - Locate Graphics Output Protocol (GOP)
    // - Query framebuffer information
    // - Populate Arm64BootInfo struct
    // - Exit boot services
    // - Call kernel_main

    // For now, return success (UEFI will handle shutdown if needed)
    Status::SUCCESS
}
