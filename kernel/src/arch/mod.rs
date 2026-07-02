extern crate alloc;

pub mod boot;

#[cfg(target_arch = "x86_64")]
pub mod x86_64;

#[cfg(target_arch = "aarch64")]
pub mod aarch64;

#[cfg(target_arch = "aarch64")]
pub use self::aarch64 as current;
#[cfg(target_arch = "x86_64")]
pub use self::x86_64 as current;

#[cfg(target_arch = "aarch64")]
pub use self::aarch64::context::CpuContext;
#[cfg(target_arch = "x86_64")]
pub use self::x86_64::context::CpuContext;

#[cfg(target_arch = "aarch64")]
pub use self::aarch64::userspace::PageFlags as ArchPageFlags;
#[cfg(target_arch = "x86_64")]
pub use self::x86_64::userspace::PageFlags as ArchPageFlags;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownStatus {
    Success,
    Failure,
}

pub fn shutdown(status: ShutdownStatus) -> ! {
    #[cfg(target_arch = "x86_64")]
    {
        current::shutdown(status)
    }

    #[cfg(not(target_arch = "x86_64"))]
    loop {
        let _ = status;
        core::hint::spin_loop();
    }
}

pub fn init() {
    current::init();
}

pub trait ArchInterrupts {
    fn enable();
    fn disable();
    fn halt_loop() -> !;
}

pub trait ArchPaging {
    type PageTable;
    type Mapper;

    fn init();
}

pub trait ArchContext {
    type ContextFrame;

    unsafe fn switch(current: *mut Self::ContextFrame, next: *const Self::ContextFrame);
}

pub fn writable_user_page_flags() -> ArchPageFlags {
    current::userspace::writable_user_page_flags()
}

pub fn empty_user_page_flags() -> ArchPageFlags {
    current::userspace::empty_user_page_flags()
}

pub fn user_brk_max() -> usize {
    current::userspace::user_brk_max()
}

pub fn user_mmap_base() -> usize {
    current::userspace::user_mmap_base()
}

pub fn build_initial_user_stack(
    address_space: &mut crate::mm::address_space::AddressSpace,
    argv: &[alloc::string::String],
    envp: &[alloc::string::String],
    exec_path: &str,
    plan: &crate::process::binfmt::LoadPlan,
) -> Result<usize, isize> {
    current::userspace::build_initial_user_stack(address_space, argv, envp, exec_path, plan)
}

pub fn arch_prctl(code: usize, addr: usize) -> Result<usize, isize> {
    current::userspace::arch_prctl(code, addr)
}

pub fn sync_current_task_user_bases() {
    current::userspace::sync_current_task_user_bases();
}

pub fn activate_task_runtime_state(task: &crate::process::task::Task) {
    current::userspace::activate_task_runtime_state(task);
}

pub unsafe fn enter_usermode() -> ! {
    unsafe { current::context::enter_usermode() }
}

pub unsafe fn switch_context(old: *mut CpuContext, new: *const CpuContext) {
    unsafe { current::context::switch(old, new) }
}

pub fn restore_kernel_syscall_state() {
    current::syscall::restore_kernel_gs_base();
}

pub fn set_physical_memory_offset(physical_memory_offset: u64) {
    current::paging::set_physical_memory_offset(physical_memory_offset);
}

pub fn physical_memory_offset() -> Option<u64> {
    current::paging::physical_memory_offset()
}

pub fn phys_to_virt_addr(physical_address: u64) -> Option<u64> {
    current::paging::phys_to_virt_addr(physical_address)
}

pub fn virt_to_phys_addr(virtual_address: u64) -> Option<u64> {
    current::paging::virt_to_phys_addr(virtual_address)
}

#[cfg(target_arch = "x86_64")]
pub unsafe fn capture_current_context(ctx: &mut CpuContext) {
    unsafe { current::context::capture_current(ctx as *mut CpuContext) }
}

#[cfg(target_arch = "aarch64")]
pub unsafe fn capture_current_context(_ctx: &mut CpuContext) {}
