#![no_std]
#![allow(dead_code)]

#[path = "../../kernel/src/arch/aarch64/paging.rs"]
mod paging;
#[path = "../../kernel/src/arch/aarch64/user_layout.rs"]
mod user_layout;

pub use user_layout::{PAGE_SIZE as USER_PAGE_SIZE, USER_ADDRESS_END};

pub fn range_overlaps_supervisor(root: u64, start: usize, end: usize) -> Option<bool> {
    paging::range_overlaps_supervisor(root, start, end)
}
