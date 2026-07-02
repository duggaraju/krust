/// Architecture-specific context switching for ARM64.
///
/// Atomically saves and restores CPU registers for task switching.
/// ARM64 uses the standard ARM calling convention (ARM Architecture Reference Manual).

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct CpuContext {
    // General purpose registers: x0-x30 (x31 is SP, handled separately)
    // Callee-saved: x19-x28, sp, lr
    // Caller-saved: x0-x18, x29, x30
    x19: u64,
    x20: u64,
    x21: u64,
    x22: u64,
    x23: u64,
    x24: u64,
    x25: u64,
    x26: u64,
    x27: u64,
    x28: u64,
    sp: u64, // Stack pointer (x31)
    fp: u64, // Frame pointer (x29)
    lr: u64, // Link register (x30)

    // User-mode context
    user_pc: u64,   // ELR_EL1 (Exception Link Register)
    user_sp: u64,   // User stack pointer
    kernel_sp: u64, // Kernel stack pointer (separate from x31)

    // Control registers
    ttbr0_el1: u64, // Translation Table Base Register (page tables)
    spsr_el1: u64,  // Saved Program Status Register
    tpidr_el0: u64, // Thread ID register (user-accessible)
    tpidr_el1: u64, // Thread ID register (kernel)
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
        self.ttbr0_el1
    }

    /// Set the root of the page table hierarchy (address space root).
    pub fn set_address_space_root(&mut self, value: u64) {
        self.ttbr0_el1 = value;
    }

    /// Get the kernel stack pointer.
    pub fn kernel_stack_pointer(&self) -> u64 {
        self.sp
    }

    /// Set the kernel stack pointer.
    pub fn set_kernel_stack_pointer(&mut self, value: u64) {
        self.sp = value;
    }

    /// Get the frame pointer.
    pub fn frame_pointer(&self) -> u64 {
        self.fp
    }

    /// Set the frame pointer.
    pub fn set_frame_pointer(&mut self, value: u64) {
        self.fp = value;
    }

    /// Get CPU control flags (Saved Program Status Register).
    pub fn control_flags(&self) -> u64 {
        self.spsr_el1
    }

    /// Set CPU control flags (Saved Program Status Register).
    pub fn set_control_flags(&mut self, value: u64) {
        self.spsr_el1 = value;
    }

    /// Get the user-mode entry point (ELR_EL1).
    pub fn user_entry_point(&self) -> u64 {
        self.user_pc
    }

    /// Set the user-mode entry point (ELR_EL1).
    pub fn set_user_entry_point(&mut self, value: u64) {
        self.user_pc = value;
    }

    /// Get the user-mode stack pointer.
    pub fn user_stack_pointer(&self) -> u64 {
        self.user_sp
    }

    /// Set the user-mode stack pointer.
    pub fn set_user_stack_pointer(&mut self, value: u64) {
        self.user_sp = value;
    }

    /// Get the kernel stack top.
    pub fn kernel_stack_top(&self) -> u64 {
        self.kernel_sp
    }

    /// Set the kernel stack top.
    pub fn set_kernel_stack_top(&mut self, value: u64) {
        self.kernel_sp = value;
    }

    /// Get user thread ID (TPIDR_EL0).
    pub fn user_thread_id(&self) -> u64 {
        self.tpidr_el0
    }

    /// Set user thread ID (TPIDR_EL0).
    pub fn set_user_thread_id(&mut self, value: u64) {
        self.tpidr_el0 = value;
    }

    /// Get kernel thread ID (TPIDR_EL1).
    pub fn kernel_thread_id(&self) -> u64 {
        self.tpidr_el1
    }

    /// Set kernel thread ID (TPIDR_EL1).
    pub fn set_kernel_thread_id(&mut self, value: u64) {
        self.tpidr_el1 = value;
    }

    /// Save the current CPU context to a memory address.
    ///
    /// Saves all callee-saved registers (x19-x28, sp, fp, lr) to consecutive
    /// 8-byte slots starting at the given address.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `addr` points to at least 176 bytes of valid writable memory.
    pub unsafe fn save_to(&self, addr: *mut u64) {
        unsafe {
            addr.add(0).write(self.x19);
            addr.add(1).write(self.x20);
            addr.add(2).write(self.x21);
            addr.add(3).write(self.x22);
            addr.add(4).write(self.x23);
            addr.add(5).write(self.x24);
            addr.add(6).write(self.x25);
            addr.add(7).write(self.x26);
            addr.add(8).write(self.x27);
            addr.add(9).write(self.x28);
            addr.add(10).write(self.sp);
            addr.add(11).write(self.fp);
            addr.add(12).write(self.lr);
            addr.add(13).write(self.user_pc);
            addr.add(14).write(self.user_sp);
            addr.add(15).write(self.kernel_sp);
            addr.add(16).write(self.ttbr0_el1);
            addr.add(17).write(self.spsr_el1);
            addr.add(18).write(self.tpidr_el0);
            addr.add(19).write(self.tpidr_el1);
        }
    }

    /// Load CPU context from a memory address.
    ///
    /// Loads all callee-saved registers from consecutive 8-byte slots starting
    /// at the given address.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `addr` points to at least 176 bytes of valid readable memory.
    pub unsafe fn load_from(&mut self, addr: *const u64) {
        unsafe {
            self.x19 = addr.add(0).read();
            self.x20 = addr.add(1).read();
            self.x21 = addr.add(2).read();
            self.x22 = addr.add(3).read();
            self.x23 = addr.add(4).read();
            self.x24 = addr.add(5).read();
            self.x25 = addr.add(6).read();
            self.x26 = addr.add(7).read();
            self.x27 = addr.add(8).read();
            self.x28 = addr.add(9).read();
            self.sp = addr.add(10).read();
            self.fp = addr.add(11).read();
            self.lr = addr.add(12).read();
            self.user_pc = addr.add(13).read();
            self.user_sp = addr.add(14).read();
            self.kernel_sp = addr.add(15).read();
            self.ttbr0_el1 = addr.add(16).read();
            self.spsr_el1 = addr.add(17).read();
            self.tpidr_el0 = addr.add(18).read();
            self.tpidr_el1 = addr.add(19).read();
        }
    }

    /// Save this context to the kernel stack.
    ///
    /// Decrements the stack pointer, then saves all registers.
    /// Returns the new SP value.
    pub fn save_to_stack(&mut self) -> u64 {
        let mut sp = self.kernel_stack_pointer();
        sp -= 160; // 20 * 8 bytes
        unsafe {
            self.save_to(sp as *mut u64);
        }
        self.set_kernel_stack_pointer(sp);
        sp
    }

    /// Load this context from the kernel stack.
    ///
    /// Loads all registers from the current SP, then increments it.
    pub fn load_from_stack(&mut self) {
        let sp = self.kernel_stack_pointer();
        unsafe {
            self.load_from(sp as *const u64);
        }
        self.set_kernel_stack_pointer(sp + 160); // 20 * 8 bytes
    }
}

/// Switch CPU context from one task to another.
///
/// Saves all callee-saved registers from the current task into `old`,
/// then loads registers for the new task from `new` and returns.
///
/// # Safety
///
/// This function performs a context switch. The `old` and `new` pointers must be valid
/// and properly aligned. This must only be called from the kernel context.
pub unsafe fn switch(old: *mut CpuContext, new: *const CpuContext) {
    unsafe {
        core::arch::asm!(
            // Save current context
            "stp x19, x20, [{old}]",
            "stp x21, x22, [{old}, #16]",
            "stp x23, x24, [{old}, #32]",
            "stp x25, x26, [{old}, #48]",
            "stp x27, x28, [{old}, #64]",
            "stp sp, x29, [{old}, #80]",
            "str x30, [{old}, #96]",

            // Load new context
            "ldp x19, x20, [{new}]",
            "ldp x21, x22, [{new}, #16]",
            "ldp x23, x24, [{new}, #32]",
            "ldp x25, x26, [{new}, #48]",
            "ldp x27, x28, [{new}, #64]",
            "ldp sp, x29, [{new}, #80]",
            "ldr x30, [{new}, #96]",
            "ret",

            old = in(reg) old,
            new = in(reg) new,
            options(noreturn, preserves_flags)
        );
    }
}

pub unsafe fn enter_usermode() -> ! {
    // Stub for now: ARM64 usermode entry via ERET instruction
    loop {
        core::hint::spin_loop();
    }
}
