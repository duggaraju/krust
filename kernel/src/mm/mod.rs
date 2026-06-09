pub mod address;
pub mod frame_allocator;
pub mod heap;

use bootloader_api::info::MemoryRegion;
use log::info;
use x86_64::{
    VirtAddr,
    registers::control::Cr3,
    structures::paging::{OffsetPageTable, PageTable},
};

use self::frame_allocator::BootInfoFrameAllocator;

/// Initialize memory management from pre-extracted boot info fields.
/// This avoids borrow conflicts with BootInfo in main.
pub fn init_with(physical_memory_offset: u64, memory_regions: &'static [MemoryRegion]) {
    let phys_offset = VirtAddr::new(physical_memory_offset);

    let mut mapper = unsafe { init_mapper(phys_offset) };
    let mut frame_allocator = BootInfoFrameAllocator::init(memory_regions);

    heap::init(&mut mapper, &mut frame_allocator);
    info!("memory management initialized");
}

unsafe fn init_mapper(physical_memory_offset: VirtAddr) -> OffsetPageTable<'static> {
    let level_4_table = unsafe { active_level_4_table(physical_memory_offset) };
    unsafe { OffsetPageTable::new(level_4_table, physical_memory_offset) }
}

unsafe fn active_level_4_table(physical_memory_offset: VirtAddr) -> &'static mut PageTable {
    let (level_4_table_frame, _) = Cr3::read();
    let physical_address = level_4_table_frame.start_address();
    let virtual_address = physical_memory_offset + physical_address.as_u64();
    let page_table_ptr: *mut PageTable = virtual_address.as_mut_ptr();

    // SAFETY: the bootloader maps all physical memory at `physical_memory_offset`,
    // so the active level 4 frame is accessible through this virtual address.
    unsafe { &mut *page_table_ptr }
}
