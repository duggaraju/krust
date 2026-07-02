/// ARM64-specific user-mode page flags and userspace utilities.
use crate::process::task::Task;

/// ARM64 page table descriptor format (stage 1, EL0/EL1)
/// Attributes: valid, access flags, shareability, memory type, etc.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageFlags(u64);

impl PageFlags {
    /// Valid page table entry
    const VALID: u64 = 0x1;

    /// Page vs Table descriptor
    const PAGE_DESCRIPTOR: u64 = 0x3;

    /// Unprivileged (user-accessible)
    const UNPRIVILEGED: u64 = 1 << 6;

    /// Read-write permission
    const WRITABLE: u64 = 0 << 7;
    const READABLE: u64 = 0x1 << 7;

    /// Memory attribute index for normal memory
    const MAIR_IDX: u64 = 0x4 << 2;

    /// Shareability: inner shareable
    const INNER_SHAREABLE: u64 = 0x3 << 8;

    pub fn new() -> Self {
        PageFlags(0)
    }

    pub fn writable(mut self) -> Self {
        self.0 |= Self::VALID
            | Self::PAGE_DESCRIPTOR
            | Self::WRITABLE
            | Self::MAIR_IDX
            | Self::INNER_SHAREABLE;
        self
    }

    pub fn readable(mut self) -> Self {
        self.0 |= Self::VALID
            | Self::PAGE_DESCRIPTOR
            | Self::READABLE
            | Self::MAIR_IDX
            | Self::INNER_SHAREABLE;
        self
    }

    pub fn user_accessible(mut self) -> Self {
        self.0 |= Self::UNPRIVILEGED;
        self
    }

    pub fn as_u64(&self) -> u64 {
        self.0
    }
}

impl Default for PageFlags {
    fn default() -> Self {
        PageFlags::new()
    }
}

pub fn writable_user_page_flags() -> PageFlags {
    PageFlags::new().writable().user_accessible()
}

pub fn empty_user_page_flags() -> PageFlags {
    PageFlags::new()
}

/// Maximum user-addressable memory (typical for 64-bit systems)
pub fn user_brk_max() -> usize {
    0x0000_7FFF_FFFF_0000 // 128 TiB (practical limit)
}

/// Base address for user mmap allocations
pub fn user_mmap_base() -> usize {
    0x0000_1000_0000_0000 // 1 TiB start
}

pub fn build_initial_user_stack(
    _address_space: &mut crate::mm::address_space::AddressSpace,
    _argv: &[alloc::string::String],
    _envp: &[alloc::string::String],
    _exec_path: &str,
    _plan: &crate::process::binfmt::LoadPlan,
) -> Result<usize, isize> {
    // TODO: Implement ARM64-specific user stack setup (argc, argv, environ, auxv, etc.)
    // For now, return a stub value
    Ok(0x0000_7FFF_FFFF_0000)
}

pub fn arch_prctl(_code: usize, _addr: usize) -> Result<usize, isize> {
    // ARM64 doesn't have arch_prctl syscall; this is x86_64-specific
    // Return ENOSYS for compatibility
    Err(-38) // ENOSYS
}

pub fn sync_current_task_user_bases() {
    // Synchronize kernel view of user thread ID registers
    // Not typically needed on ARM64 as registers are synchronized on context switch
}

pub fn activate_task_runtime_state(task: &Task) {
    let _ = task;
    // Activate task-specific runtime state (thread ID registers, etc.)
    // TODO: Implement if needed for future task-local storage
}
