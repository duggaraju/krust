use super::gdt;
use crate::process;
/// Architecture-specific context switching for x86_64.
///
/// Atomically saves and restores CPU registers for task switching.
use crate::process::context::CpuContext;

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

pub unsafe fn enter_usermode() -> ! {
    let mut user_rip = 0u64;
    let mut user_rsp = 0u64;
    let mut kernel_rsp = 0u64;
    let mut cr3 = 0u64;

    let _ = process::with_current_task_mut(|task| {
        user_rip = task.context.user_rip;
        user_rsp = task.context.user_rsp;
        kernel_rsp = task.kernel_stack_top as u64;
        cr3 = task.context.cr3;
        task.context.kernel_rsp = kernel_rsp;
        gdt::set_tss_rsp0(kernel_rsp);
        super::syscall::set_kernel_stack_top(kernel_rsp);
        super::syscall::set_user_bases(task.fs_base, task.gs_base);
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
