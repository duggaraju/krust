#![no_std]
#![allow(dead_code)]

extern crate alloc;

mod process {
    pub mod task {
        pub struct Task;
    }
    pub mod binfmt {
        pub struct LoadPlan;
    }
}
mod mm {
    pub mod address_space {
        pub struct AddressSpace;
    }
}

#[path = "../../kernel/src/arch/aarch64/paging.rs"]
mod paging;
#[path = "../../kernel/src/arch/aarch64/user_layout.rs"]
mod user_layout;
#[path = "../../kernel/src/arch/aarch64/userspace.rs"]
mod userspace;

pub fn mmap_user_page_flags(prot: usize) -> Result<userspace::PageFlags, isize> {
    userspace::mmap_user_page_flags(prot)
}

pub use user_layout::{PAGE_SIZE as USER_PAGE_SIZE, USER_ADDRESS_END};

pub fn range_overlaps_supervisor(root: u64, start: usize, end: usize) -> Option<bool> {
    paging::range_overlaps_supervisor(root, start, end)
}
