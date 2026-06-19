#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct CpuContext {
    #[cfg(target_arch = "x86_64")]
    pub rsp: u64,
    #[cfg(target_arch = "x86_64")]
    pub rbp: u64,
    #[cfg(target_arch = "x86_64")]
    pub rbx: u64,
    #[cfg(target_arch = "x86_64")]
    pub r12: u64,
    #[cfg(target_arch = "x86_64")]
    pub r13: u64,
    #[cfg(target_arch = "x86_64")]
    pub r14: u64,
    #[cfg(target_arch = "x86_64")]
    pub r15: u64,
    #[cfg(target_arch = "x86_64")]
    pub rip: u64,
    #[cfg(target_arch = "x86_64")]
    pub user_rip: u64,
    #[cfg(target_arch = "x86_64")]
    pub user_rsp: u64,
    #[cfg(target_arch = "x86_64")]
    pub kernel_rsp: u64,
    #[cfg(target_arch = "x86_64")]
    pub rflags: u64,
    #[cfg(target_arch = "x86_64")]
    pub cr3: u64,
    #[cfg(target_arch = "x86_64")]
    pub rax: u64,
}

impl CpuContext {
    pub fn new() -> Self {
        Self::default()
    }
}
