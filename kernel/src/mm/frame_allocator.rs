use bootloader_api::info::{MemoryRegion, MemoryRegionKind};
use x86_64::{
    PhysAddr,
    structures::paging::{FrameAllocator, FrameDeallocator, PageSize, PhysFrame, Size4KiB},
};

#[derive(Debug)]
pub struct BootInfoFrameAllocator {
    memory_regions: &'static [MemoryRegion],
    next: usize,
}

impl BootInfoFrameAllocator {
    pub fn init(memory_regions: &'static [MemoryRegion]) -> Self {
        Self {
            memory_regions,
            next: 0,
        }
    }

    pub fn allocate_frame(&mut self) -> Option<PhysFrame> {
        let frame = self.usable_frames().nth(self.next);
        if frame.is_some() {
            self.next += 1;
        }
        frame
    }

    pub fn deallocate_frame(&mut self, _frame: PhysFrame) {}

    fn usable_frames(&self) -> impl Iterator<Item = PhysFrame> + '_ {
        self.memory_regions
            .iter()
            .filter(|region| region.kind == MemoryRegionKind::Usable)
            .flat_map(|region| {
                let start = PhysAddr::new(region.start)
                    .align_up(Size4KiB::SIZE)
                    .as_u64();
                let end = PhysAddr::new(region.end)
                    .align_down(Size4KiB::SIZE)
                    .as_u64();

                (start..end)
                    .step_by(Size4KiB::SIZE as usize)
                    .map(|addr| PhysFrame::containing_address(PhysAddr::new(addr)))
            })
    }
}

unsafe impl FrameAllocator<Size4KiB> for BootInfoFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        BootInfoFrameAllocator::allocate_frame(self)
    }
}

impl FrameDeallocator<Size4KiB> for BootInfoFrameAllocator {
    unsafe fn deallocate_frame(&mut self, frame: PhysFrame<Size4KiB>) {
        BootInfoFrameAllocator::deallocate_frame(self, frame);
    }
}
