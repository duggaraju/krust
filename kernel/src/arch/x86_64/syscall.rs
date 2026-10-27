use x86_64::VirtAddr;
use x86_64::registers::model_specific::{
    Efer, EferFlags, FsBase, GsBase, KernelGsBase, LStar, SFMask, Star,
};
use x86_64::registers::rflags::RFlags;

#[repr(C)]
struct SyscallCpuLocal {
    kernel_rsp: u64,
    scratch: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct SavedRegs {
    rdi: u64,
    rsi: u64,
    rdx: u64,
    r10: u64,
    r8: u64,
    r9: u64,
    rcx: u64,
    r11: u64,
    rbx: u64,
    rbp: u64,
    r12: u64,
    r13: u64,
    r14: u64,
    r15: u64,
    user_rsp: u64,
}

const _: () = assert!(core::mem::size_of::<SavedRegs>() == 120);

static mut SYSCALL_CPU_LOCAL: SyscallCpuLocal = SyscallCpuLocal {
    kernel_rsp: 0,
    scratch: 0,
};

core::arch::global_asm!(
    r#"
    .global syscall_entry_trampoline
syscall_entry_trampoline:
    swapgs
    mov gs:[8], rax
    mov rax, rsp
    mov rsp, gs:[0]
    push rax

    push r15
    push r14
    push r13
    push r12
    push rbp
    push rbx
    push r11
    push rcx
    push r9
    push r8
    push r10
    push rdx
    push rsi
    push rdi

    mov rdi, gs:[8]
    mov rsi, rsp
    sub rsp, 136
    call syscall_dispatch_from_regs
    add rsp, 136
    mov gs:[8], rax

.Lsyscall_return:
    pop rdi
    pop rsi
    pop rdx
    pop r10
    pop r8
    pop r9
    pop rcx
    pop r11
    pop rbx
    pop rbp
    pop r12
    pop r13
    pop r14
    pop r15

    pop rax
    mov rsp, rax
    mov rax, gs:[8]
    swapgs
    sysretq

    .global fork_child_trampoline
fork_child_trampoline:
    sub rsp, 8
    call fork_child_prepare_return
    mov rsp, rax
    mov qword ptr gs:[8], 0
    jmp .Lsyscall_return
"#
);

unsafe extern "C" {
    fn syscall_entry_trampoline();
    fn fork_child_trampoline();
}

#[unsafe(no_mangle)]
extern "C" fn fork_child_prepare_return() -> u64 {
    let (frame, gs_base) = crate::process::with_current_task_mut(|task| {
        task.mode = crate::process::task::TaskMode::User;
        (
            task.kernel_stack_top - core::mem::size_of::<SavedRegs>(),
            task.context.thread_local_base(false),
        )
    })
    .expect("fork child must be registered");
    GsBase::write(VirtAddr::from_ptr(core::ptr::addr_of!(SYSCALL_CPU_LOCAL)));
    KernelGsBase::write(VirtAddr::new(gs_base));
    frame as u64
}

pub fn fork_from_syscall() -> Result<u64, isize> {
    let frame = crate::process::with_current_task_mut(|task| {
        if !task.userland {
            return Err(-38isize);
        }
        let address = task.kernel_stack_top - core::mem::size_of::<SavedRegs>();
        // SYSCALL saves this fixed frame at the top of the current task's stack.
        Ok(unsafe { (address as *const SavedRegs).read() })
    })
    .ok_or(-1isize)??;
    let child = crate::process::fork_current_task()?;
    crate::process::with_task_mut(child, |task| {
        let address = task.kernel_stack_top - core::mem::size_of::<SavedRegs>();
        unsafe {
            (address as *mut SavedRegs).write(frame);
        }
        task.context.set_kernel_stack_pointer(address as u64);
        task.context.set_frame_pointer(0);
        task.context
            .set_kernel_stack_top(task.kernel_stack_top as u64);
        task.context
            .set_kernel_entry_point(fork_child_trampoline as *const () as u64);
        task.context.set_user_entry_point(frame.rcx);
        task.context.set_user_stack_pointer(frame.user_rsp);
    })
    .ok_or(-1isize)?;
    if let Some(plan) = crate::process::yield_to_task(child) {
        unsafe {
            crate::arch::restore_kernel_syscall_state();
            crate::arch::switch_context(plan.old_ctx, plan.new_ctx);
        }
    }
    Ok(child)
}

#[unsafe(no_mangle)]
extern "C" fn syscall_dispatch_from_regs(number: u64, regs: *const SavedRegs) -> isize {
    let regs = unsafe { &*regs };
    log::debug!(
        "syscall_dispatch: pid={} nr={} rcx=0x{:x} r11=0x{:x}",
        crate::process::current_pid(),
        number,
        regs.rcx,
        regs.r11
    );
    if regs.rcx < 0x1000 {
        log::warn!(
            "syscall return RIP suspicious: pid={} nr={} rcx=0x{:x} r11=0x{:x}",
            crate::process::current_pid(),
            number,
            regs.rcx,
            regs.r11
        );
    }
    let args = crate::syscall::dispatch::SyscallArgs {
        number: number as usize,
        arg0: regs.rdi as usize,
        arg1: regs.rsi as usize,
        arg2: regs.rdx as usize,
        arg3: regs.r10 as usize,
        arg4: regs.r8 as usize,
        arg5: regs.r9 as usize,
    };

    let previous = crate::process::current_task_mode();
    let _ = crate::process::set_current_task_mode(crate::process::task::TaskMode::Kernel);
    let result = crate::syscall::dispatch::dispatch(&args);
    if matches!(previous, Some(crate::process::task::TaskMode::User)) {
        let _ = crate::process::set_current_task_mode(crate::process::task::TaskMode::User);
    }

    log::debug!(
        "syscall_dispatch returning: pid={} nr={} result=0x{:x}",
        crate::process::current_pid(),
        number,
        result as u64
    );
    if let Some(gs_base) =
        crate::process::with_current_task_mut(|task| task.context.thread_local_base(false))
    {
        // A faulting child can resume this syscall from an unswapped interrupt entry.
        GsBase::write(VirtAddr::from_ptr(core::ptr::addr_of!(SYSCALL_CPU_LOCAL)));
        KernelGsBase::write(VirtAddr::new(gs_base));
    }
    result
}

pub fn init() {
    let (kernel_cs, kernel_ss) = (
        super::gdt::kernel_code_selector(),
        super::gdt::kernel_data_selector(),
    );
    let (user_cs, user_ss) = (
        super::gdt::user_code_selector(),
        super::gdt::user_data_selector(),
    );

    let _ = Star::write(user_cs, user_ss, kernel_cs, kernel_ss);
    LStar::write(VirtAddr::from_ptr(syscall_entry_trampoline as *const ()));
    SFMask::write(RFlags::INTERRUPT_FLAG);
    unsafe {
        Efer::update(|flags| {
            flags.insert(EferFlags::SYSTEM_CALL_EXTENSIONS);
        });
    }
    KernelGsBase::write(VirtAddr::from_ptr(core::ptr::addr_of!(SYSCALL_CPU_LOCAL)));
}

pub fn set_kernel_stack_top(rsp: u64) {
    unsafe {
        SYSCALL_CPU_LOCAL.kernel_rsp = rsp;
    }
}

pub fn set_user_bases(fs_base: u64, _gs_base: u64) {
    FsBase::write(VirtAddr::new(fs_base));
}

pub fn prepare_usermode_entry(gs_base: u64) {
    // Exec entered through SYSCALL has already swapped GS, unlike a kernel
    // fork_execve caller. Normalize both bases before the non-returning IRET.
    GsBase::write(VirtAddr::new(gs_base));
    restore_kernel_gs_base();
}

pub fn restore_kernel_gs_base() {
    KernelGsBase::write(VirtAddr::from_ptr(core::ptr::addr_of!(SYSCALL_CPU_LOCAL)));
}
