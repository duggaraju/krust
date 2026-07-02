pub mod address_space;
pub mod frame_allocator;
pub mod heap;
pub mod stats;

use bootloader_api::info::MemoryRegion;
use log::info;
use spin::{Mutex, Once};
use x86_64::{
    VirtAddr,
    registers::control::Cr3,
    structures::paging::{PhysFrame, Size4KiB},
};

use self::frame_allocator::BootInfoFrameAllocator;

static FRAME_ALLOCATOR: Mutex<Option<BootInfoFrameAllocator>> = Mutex::new(None);
static KERNEL_ROOT_FRAME: Once<PhysFrame<Size4KiB>> = Once::new();

/// Initialize memory management from pre-extracted boot info fields.
/// This avoids borrow conflicts with BootInfo in main.
pub fn init_with(physical_memory_offset: u64, memory_regions: &'static [MemoryRegion]) {
    crate::arch::set_physical_memory_offset(physical_memory_offset);
    let phys_offset = VirtAddr::new(physical_memory_offset);

    let mut mapper = unsafe { crate::arch::current::paging::init_mapper(phys_offset.as_u64()) };
    let mut frame_allocator = BootInfoFrameAllocator::init(memory_regions);

    stats::init(memory_regions);
    heap::init(&mut mapper, &mut frame_allocator);
    *FRAME_ALLOCATOR.lock() = Some(frame_allocator);
    let (root_frame, _) = Cr3::read();
    KERNEL_ROOT_FRAME.call_once(|| root_frame);
    info!("memory management initialized");
}

pub fn kernel_root_frame() -> Option<PhysFrame<Size4KiB>> {
    KERNEL_ROOT_FRAME.get().copied()
}

pub fn with_frame_allocator<R>(f: impl FnOnce(&mut BootInfoFrameAllocator) -> R) -> Option<R> {
    let mut allocator = FRAME_ALLOCATOR.lock();
    let allocator = allocator.as_mut()?;
    Some(f(allocator))
}

pub fn allocate_frame() -> Option<PhysFrame<Size4KiB>> {
    with_frame_allocator(|allocator| allocator.allocate_frame()).flatten()
}

pub fn deallocate_frame(frame: PhysFrame<Size4KiB>) {
    let _ = with_frame_allocator(|allocator| allocator.deallocate_frame(frame));
}
