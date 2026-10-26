//! Four-level x86_64 userspace layout (exclusive lower-canonical-half end).
pub const PAGE_SIZE: usize = 4096;
pub const USER_ADDRESS_END: usize = 1 << 47;
pub const USER_STACK_TOP: usize = USER_ADDRESS_END - PAGE_SIZE;
