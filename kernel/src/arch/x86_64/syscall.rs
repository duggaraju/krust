use x86_64::VirtAddr;
use x86_64::registers::model_specific::{
    Efer, EferFlags, FsBase, KernelGsBase, LStar, SFMask, Star,
};
use x86_64::registers::rflags::RFlags;

#[repr(C)]
struct SyscallCpuLocal {
    kernel_rsp: u64,
    scratch: u64,
}

#[repr(C)]
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
}

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
    sub rsp, 128
    call syscall_dispatch_from_regs
    add rsp, 128
    mov gs:[8], rax

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
"#
);

unsafe extern "C" {
    fn syscall_entry_trampoline();
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

pub fn restore_kernel_gs_base() {
    KernelGsBase::write(VirtAddr::from_ptr(core::ptr::addr_of!(SYSCALL_CPU_LOCAL)));
}
