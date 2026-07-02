use super::gdt;
use crate::process;
/// Architecture-specific context switching for x86_64.
///
/// Atomically saves and restores CPU registers for task switching.

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct CpuContext {
    rsp: u64,
    rbp: u64,
    rbx: u64,
    r12: u64,
    r13: u64,
    r14: u64,
    r15: u64,
    rip: u64,
    user_rip: u64,
    user_rsp: u64,
    kernel_rsp: u64,
    rflags: u64,
    cr3: u64,
    rax: u64,
    fs_base: u64,
    gs_base: u64,
}

impl CpuContext {
    pub fn new() -> Self {
        Self::default()
    }

    /// Platform-independent interface for CPU context management.
    ///
    /// These methods abstract away architecture-specific register names and provide
    /// a unified interface for managing CPU state across x86_64 and ARM64.

    /// Get the root of the page table hierarchy (address space root).
    pub fn address_space_root(&self) -> u64 {
        self.cr3
    }

    /// Set the root of the page table hierarchy (address space root).
    pub fn set_address_space_root(&mut self, value: u64) {
        self.cr3 = value;
    }

    /// Get the kernel stack pointer.
    pub fn kernel_stack_pointer(&self) -> u64 {
        self.rsp
    }

    /// Set the kernel stack pointer.
    pub fn set_kernel_stack_pointer(&mut self, value: u64) {
        self.rsp = value;
    }

    /// Get the frame pointer.
    pub fn frame_pointer(&self) -> u64 {
        self.rbp
    }

    /// Set the frame pointer.
    pub fn set_frame_pointer(&mut self, value: u64) {
        self.rbp = value;
    }

    /// Get the kernel entry point (instruction pointer).
    pub fn kernel_entry_point(&self) -> u64 {
        self.rip
    }

    /// Set the kernel entry point (instruction pointer).
    pub fn set_kernel_entry_point(&mut self, value: u64) {
        self.rip = value;
    }

    /// Get CPU control flags (e.g., interrupt enable, direction flag).
    pub fn control_flags(&self) -> u64 {
        self.rflags
    }

    /// Set CPU control flags (e.g., interrupt enable, direction flag).
    pub fn set_control_flags(&mut self, value: u64) {
        self.rflags = value;
    }

    /// Get the return value register (used for syscall returns).
    pub fn return_value(&self) -> u64 {
        self.rax
    }

    /// Set the return value register (used for syscall returns).
    pub fn set_return_value(&mut self, value: u64) {
        self.rax = value;
    }

    /// Get the user-mode entry point.
    pub fn user_entry_point(&self) -> u64 {
        self.user_rip
    }

    /// Set the user-mode entry point.
    pub fn set_user_entry_point(&mut self, value: u64) {
        self.user_rip = value;
    }

    /// Get the user-mode stack pointer.
    pub fn user_stack_pointer(&self) -> u64 {
        self.user_rsp
    }

    /// Set the user-mode stack pointer.
    pub fn set_user_stack_pointer(&mut self, value: u64) {
        self.user_rsp = value;
    }

    /// Get the kernel stack top (separate from kernel_stack_pointer for TSS/IST).
    pub fn kernel_stack_top(&self) -> u64 {
        self.kernel_rsp
    }

    /// Set the kernel stack top (separate from kernel_stack_pointer for TSS/IST).
    pub fn set_kernel_stack_top(&mut self, value: u64) {
        self.kernel_rsp = value;
    }

    /// Get thread-local storage base (FS for true, GS for false).
    pub fn thread_local_base(&self, is_fs: bool) -> u64 {
        if is_fs { self.fs_base } else { self.gs_base }
    }

    /// Set thread-local storage base (FS for true, GS for false).
    pub fn set_thread_local_base(&mut self, value: u64, is_fs: bool) {
        if is_fs {
            self.fs_base = value;
        } else {
            self.gs_base = value;
        }
    }

    /// Save the current CPU context to a memory address.
    ///
    /// Saves all callee-saved registers (RSP, RBP, RBX, R12-R15, RIP) to consecutive
    /// 8-byte slots starting at the given address. Useful for persisting context snapshots.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `addr` points to at least 56 bytes of valid writable memory.
    pub unsafe fn save_to(&self, addr: *mut u64) {
        unsafe {
            addr.add(0).write(self.rsp);
            addr.add(1).write(self.rbp);
            addr.add(2).write(self.rbx);
            addr.add(3).write(self.r12);
            addr.add(4).write(self.r13);
            addr.add(5).write(self.r14);
            addr.add(6).write(self.r15);
            addr.add(7).write(self.rip);
        }
    }

    /// Load CPU context from a memory address.
    ///
    /// Loads all callee-saved registers (RSP, RBP, RBX, R12-R15, RIP) from consecutive
    /// 8-byte slots starting at the given address.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `addr` points to at least 56 bytes of valid readable memory.
    pub unsafe fn load_from(&mut self, addr: *const u64) {
        unsafe {
            self.rsp = addr.add(0).read();
            self.rbp = addr.add(1).read();
            self.rbx = addr.add(2).read();
            self.r12 = addr.add(3).read();
            self.r13 = addr.add(4).read();
            self.r14 = addr.add(5).read();
            self.r15 = addr.add(6).read();
            self.rip = addr.add(7).read();
        }
    }

    /// Save this context to the kernel stack.
    ///
    /// Decrements the stack pointer, then saves all callee-saved registers.
    /// Returns the new RSP value.
    pub fn save_to_stack(&mut self) -> u64 {
        let mut rsp = self.kernel_stack_pointer();
        rsp -= 56; // 7 * 8 bytes
        unsafe {
            self.save_to(rsp as *mut u64);
        }
        self.set_kernel_stack_pointer(rsp);
        rsp
    }

    /// Load this context from the kernel stack.
    ///
    /// Loads all callee-saved registers from the current RSP, then increments it.
    pub fn load_from_stack(&mut self) {
        let rsp = self.kernel_stack_pointer();
        unsafe {
            self.load_from(rsp as *const u64);
        }
        self.set_kernel_stack_pointer(rsp + 56); // 7 * 8 bytes
    }
}

/// Switch CPU context from one task to another.
///
/// Saves all callee-saved registers from the current task into `old`,
/// then loads registers for the new task from `new` and jumps to its RIP.
///
/// # Safety
///
/// This function performs an unconditional context switch. It returns only when
/// this task is later resumed by another switch operation. The `old` and `new`
/// pointers must be valid and properly aligned.
pub unsafe fn switch(old: *mut CpuContext, new: *const CpuContext) {
    unsafe {
        core::arch::asm!(
            // Save old context
            "mov [{old} + 0],   rsp",
            "mov [{old} + 8],   rbp",
            "mov [{old} + 16],  rbx",
            "mov [{old} + 24],  r12",
            "mov [{old} + 32],  r13",
            "mov [{old} + 40],  r14",
            "mov [{old} + 48],  r15",
            // Store resume RIP so this function returns when switched back in.
            "lea rax, [rip + 2f]",
            "mov [{old} + 56],  rax",

            // Load new context
            "mov rsp, [{new} + 0]",
            "mov rbp, [{new} + 8]",
            "mov rbx, [{new} + 16]",
            "mov r12, [{new} + 24]",
            "mov r13, [{new} + 32]",
            "mov r14, [{new} + 40]",
            "mov r15, [{new} + 48]",

            // Jump to new task's rip
            "jmp qword ptr [{new} + 56]",
            "2:",

            old = in(reg) old,
            new = in(reg) new,
            out("rax") _,
            options(nostack)
        );
    }
}

/// Capture a resumable kernel continuation into an existing CPU context.
///
/// Saves the callee-saved register set plus a resume RIP that can be used
/// by the scheduler's normal context switch path.
///
/// # Safety
///
/// `ctx` must be a valid, aligned pointer to writable `CpuContext` storage.
pub unsafe fn capture_current(ctx: *mut CpuContext) {
    unsafe {
        core::arch::asm!(
            "mov [{ctx} + 0],   rsp",
            "mov [{ctx} + 8],   rbp",
            "mov [{ctx} + 16],  rbx",
            "mov [{ctx} + 24],  r12",
            "mov [{ctx} + 32],  r13",
            "mov [{ctx} + 40],  r14",
            "mov [{ctx} + 48],  r15",
            "lea rax, [rip + 2f]",
            "mov [{ctx} + 56],  rax",
            "2:",
            ctx = in(reg) ctx,
            out("rax") _,
            options(nostack)
        );
    }
}

pub unsafe fn enter_usermode() -> ! {
    let mut user_rip = 0u64;
    let mut user_rsp = 0u64;
    let mut kernel_rsp = 0u64;
    let mut cr3 = 0u64;

    let _ = process::with_current_task_mut(|task| {
        user_rip = task.context.user_entry_point();
        user_rsp = task.context.user_stack_pointer();
        kernel_rsp = task.kernel_stack_top as u64;
        cr3 = task.context.address_space_root();
        task.context.set_kernel_stack_top(kernel_rsp);
        gdt::set_tss_rsp0(kernel_rsp);
        super::syscall::set_kernel_stack_top(kernel_rsp);
        super::syscall::set_user_bases(
            task.context.thread_local_base(true),
            task.context.thread_local_base(false),
        );
        task.activate_address_space();
    });

    unsafe {
        core::arch::asm!(
            "mov cr3, {cr3}",
            "push {user_ss}",
            "push {user_rsp}",
            "push {rflags}",
            "push {user_cs}",
            "push {user_rip}",
            "iretq",
            cr3 = in(reg) cr3,
            user_ss = in(reg) (gdt::user_data_selector().0 as u64 | 3),
            user_cs = in(reg) (gdt::user_code_selector().0 as u64 | 3),
            user_rsp = in(reg) user_rsp,
            user_rip = in(reg) user_rip,
            rflags = in(reg) 0x202u64,
            options(noreturn)
        );
    }
}
