#[cfg(feature = "arch-x86_64")]
use bootloader_api::config::ApiVersion;
#[cfg(feature = "arch-x86_64")]
use bootloader_api::info::{FrameBuffer, MemoryRegion};

pub struct KernelBootInfo {
    pub physical_memory_offset: u64,
    #[cfg(feature = "arch-x86_64")]
    pub memory_regions: &'static [MemoryRegion],
    #[cfg(not(feature = "arch-x86_64"))]
    pub memory_regions: &'static [()],
    #[cfg(feature = "arch-x86_64")]
    pub framebuffer: Option<FrameBuffer>,
    #[cfg(not(feature = "arch-x86_64"))]
    pub framebuffer: Option<()>,
    #[cfg(feature = "arch-x86_64")]
    pub api_version: ApiVersion,
}
