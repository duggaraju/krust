//! Current practical 128-TiB userspace policy for the ARM64 port.
pub const PAGE_SIZE: usize = 4096;
pub const USER_ADDRESS_END: usize = 1 << 47;
