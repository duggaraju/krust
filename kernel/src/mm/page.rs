use core::ptr::NonNull;
use x86_64::structures::paging::{PhysFrame, Size4KiB};

pub const PAGE_SIZE: usize = 4096;

pub struct OwnedPage {
    frame: PhysFrame<Size4KiB>,
    pointer: NonNull<[u8; PAGE_SIZE]>,
}

// Each page exclusively owns its frame; access requires a borrow of the owner.
unsafe impl Send for OwnedPage {}

impl OwnedPage {
    pub fn new() -> Option<Self> {
        let frame = super::allocate_frame()?;
        let Some(address) = crate::arch::phys_to_virt_addr(frame.start_address().as_u64()) else {
            super::deallocate_frame(frame);
            return None;
        };
        let Some(pointer) = NonNull::new(address as *mut [u8; PAGE_SIZE]) else {
            super::deallocate_frame(frame);
            return None;
        };
        let mut page = Self { frame, pointer };
        page.bytes_mut().fill(0);
        Some(page)
    }

    pub fn bytes(&self) -> &[u8; PAGE_SIZE] {
        // SAFETY: the direct mapping covers the exclusively owned physical frame.
        unsafe { self.pointer.as_ref() }
    }

    pub fn bytes_mut(&mut self) -> &mut [u8; PAGE_SIZE] {
        // SAFETY: mutable access is exclusive through the page owner.
        unsafe { self.pointer.as_mut() }
    }
}

impl Drop for OwnedPage {
    fn drop(&mut self) {
        super::deallocate_frame(self.frame);
    }
}
