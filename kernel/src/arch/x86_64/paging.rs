use bootloader_api::BootInfo;

pub use x86_64::structures::paging::{
    FrameAllocator, Mapper, OffsetPageTable, Page, PageSize, PageTable, PageTableFlags, PhysFrame,
    Size4KiB, Translate,
};

pub fn init(boot_info: &BootInfo) {
    let _ = boot_info;
}
