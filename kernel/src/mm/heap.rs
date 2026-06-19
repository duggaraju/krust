#[allow(unused_extern_crates)]
extern crate alloc;

use linked_list_allocator::LockedHeap;
use x86_64::{
    VirtAddr,
    structures::paging::{FrameAllocator, Mapper, Page, PageTableFlags, Size4KiB},
};

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

pub const HEAP_START: usize = 0x_4444_4444_0000;
pub const HEAP_SIZE: usize = 2 * 1024 * 1024;

pub fn init(
    mapper: &mut impl Mapper<Size4KiB>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
) {
    let page_range = {
        let heap_start = VirtAddr::new(HEAP_START as u64);
        let heap_end = heap_start + (HEAP_SIZE as u64 - 1);
        let start_page = Page::containing_address(heap_start);
        let end_page = Page::containing_address(heap_end);

        Page::range_inclusive(start_page, end_page)
    };

    for page in page_range {
        let frame = frame_allocator
            .allocate_frame()
            .expect("frame allocator ran out of usable memory while mapping the heap");
        let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE;

        unsafe {
            // SAFETY: each heap page is mapped exactly once to a fresh unused frame
            // returned by the frame allocator.
            mapper
                .map_to(page, frame, flags, frame_allocator)
                .expect("failed to map kernel heap page")
                .flush();
        }
    }

    unsafe {
        // SAFETY: the heap virtual range has just been mapped and is reserved for
        // allocator use for the remainder of the kernel lifetime.
        ALLOCATOR.lock().init(HEAP_START as *mut u8, HEAP_SIZE);
    }
}
