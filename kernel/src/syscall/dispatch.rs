use core::sync::atomic::{AtomicBool, Ordering};
use log::{error, info};

use crate::syscall::{
    handlers,
    numbers::{
        SYS_ARCH_PRCTL, SYS_BRK, SYS_CHDIR, SYS_CLOCK_GETTIME, SYS_CLOSE, SYS_EXECVE, SYS_EXIT,
        SYS_EXIT_GROUP, SYS_FCNTL, SYS_FORK, SYS_FSTAT, SYS_GETCWD, SYS_GETDENTS64, SYS_GETEGID,
        SYS_GETEUID, SYS_GETGID, SYS_GETPGID, SYS_GETPID, SYS_GETPPID, SYS_GETRANDOM, SYS_GETUID,
        SYS_IOCTL, SYS_KILL, SYS_LSEEK, SYS_LSTAT, SYS_MMAP, SYS_MPROTECT, SYS_MUNMAP,
        SYS_NANOSLEEP, SYS_NEWFSTATAT, SYS_OPEN, SYS_OPENAT, SYS_POLL, SYS_PRLIMIT64, SYS_READ,
        SYS_READLINKAT, SYS_RSEQ, SYS_RT_SIGACTION, SYS_RT_SIGPROCMASK, SYS_SCHED_YIELD,
        SYS_SET_ROBUST_LIST, SYS_SET_TID_ADDRESS, SYS_SETGID, SYS_SETPGID, SYS_SETUID,
        SYS_SHUTDOWN, SYS_STAT, SYS_TIME, SYS_TIMES, SYS_UNAME, SYS_WAIT4, SYS_WRITE, SYS_WRITEV,
        Syscall,
    },
};

pub type SyscallHandler = fn(&SyscallArgs) -> isize;

static SYSCALL_TRACE_ENABLED: AtomicBool = AtomicBool::new(false);

pub fn set_trace_enabled(enabled: bool) {
    SYSCALL_TRACE_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn trace_enabled() -> bool {
    SYSCALL_TRACE_ENABLED.load(Ordering::Relaxed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyscallArgs {
    pub number: usize,
    pub arg0: usize,
    pub arg1: usize,
    pub arg2: usize,
    pub arg3: usize,
    pub arg4: usize,
    pub arg5: usize,
}

pub fn dispatch(args: &SyscallArgs) -> isize {
    if trace_enabled() {
        info!(
            "syscall entry: pid={} nr={} ({}) args=[{:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x}]",
            crate::process::current_pid(),
            args.number,
            syscall_name(args.number),
            args.arg0,
            args.arg1,
            args.arg2,
            args.arg3,
            args.arg4,
            args.arg5,
        );

        if let Some(syscall) = Syscall::from_number(args.number) {
            info!(
                "syscall meta: name={} argc={}",
                syscall.name(),
                syscall.arg_count()
            );
        }
    }

    let handler: Option<SyscallHandler> = match args.number {
        SYS_READ => Some(handlers::sys_read),
        SYS_WRITE => Some(handlers::sys_write),
        SYS_WRITEV => Some(handlers::sys_writev),
        SYS_IOCTL => Some(handlers::sys_ioctl),
        SYS_OPEN => Some(handlers::sys_open),
        SYS_CLOSE => Some(handlers::sys_close),
        SYS_POLL => Some(handlers::sys_poll),
        SYS_STAT => Some(handlers::sys_stat),
        SYS_FSTAT => Some(handlers::sys_fstat),
        SYS_KILL => Some(handlers::sys_kill),
        SYS_UNAME => Some(handlers::sys_uname),
        SYS_GETDENTS64 => Some(handlers::sys_getdents64),
        SYS_NEWFSTATAT => Some(handlers::sys_newfstatat),
        SYS_GETCWD => Some(handlers::sys_getcwd),
        SYS_CHDIR => Some(handlers::sys_chdir),
        SYS_GETUID => Some(handlers::sys_getuid),
        SYS_GETEUID => Some(handlers::sys_geteuid),
        SYS_GETGID => Some(handlers::sys_getgid),
        SYS_GETEGID => Some(handlers::sys_getegid),
        SYS_SETPGID => Some(handlers::sys_setpgid),
        SYS_GETPPID => Some(handlers::sys_getppid),
        SYS_GETPGID => Some(handlers::sys_getpgid),
        SYS_SETUID => Some(handlers::sys_setuid),
        SYS_SETGID => Some(handlers::sys_setgid),
        SYS_TIMES => Some(handlers::sys_times),
        SYS_TIME => Some(handlers::sys_time),
        SYS_GETPID => Some(handlers::sys_getpid),
        SYS_FORK => Some(handlers::sys_fork),
        SYS_EXECVE => Some(handlers::sys_execve),
        SYS_EXIT | SYS_EXIT_GROUP => Some(handlers::sys_exit),
        SYS_SHUTDOWN => Some(handlers::sys_shutdown),
        SYS_MMAP => Some(handlers::sys_mmap),
        SYS_MUNMAP => Some(handlers::sys_munmap),
        SYS_MPROTECT => Some(handlers::sys_mprotect),
        SYS_BRK => Some(handlers::sys_brk),
        SYS_ARCH_PRCTL => Some(handlers::sys_arch_prctl),
        SYS_SET_TID_ADDRESS => Some(handlers::sys_set_tid_address),
        SYS_CLOCK_GETTIME => Some(handlers::sys_clock_gettime),
        SYS_READLINKAT => Some(handlers::sys_readlinkat),
        SYS_SET_ROBUST_LIST => Some(handlers::sys_set_robust_list),
        SYS_PRLIMIT64 => Some(handlers::sys_prlimit64),
        SYS_GETRANDOM => Some(handlers::sys_getrandom),
        SYS_RSEQ => Some(handlers::sys_rseq),
        SYS_WAIT4 => Some(handlers::sys_wait4),
        SYS_SCHED_YIELD => Some(handlers::sys_sched_yield),
        SYS_LSTAT => Some(handlers::sys_stat),
        SYS_LSEEK => Some(handlers::sys_lseek),
        SYS_FCNTL => Some(handlers::sys_fcntl),
        SYS_OPENAT => Some(handlers::sys_openat),
        SYS_RT_SIGACTION => Some(handlers::sys_rt_sigaction),
        SYS_RT_SIGPROCMASK => Some(handlers::sys_rt_sigprocmask),
        SYS_NANOSLEEP => Some(handlers::sys_nanosleep),
        _ => None,
    };

    match handler {
        Some(handler) => {
            let result = handler(args);
            if trace_enabled() {
                info!(
                    "syscall exit: pid={} nr={} ({}) ret={}",
                    crate::process::current_pid(),
                    args.number,
                    syscall_name(args.number),
                    result
                );
            }
            if result < 0 && trace_enabled() {
                error!(
                    "syscall failed: nr={} ({}) errno={}",
                    args.number,
                    syscall_name(args.number),
                    result,
                );
            }
            result
        }
        None => {
            error!(
                "unimplemented syscall: nr={} ({})",
                args.number,
                syscall_name(args.number),
            );
            -38
        }
    }
}

pub fn kernel_syscall_invoke(args: &SyscallArgs) -> isize {
    let previous = crate::process::current_task_mode();
    let _ = crate::process::set_current_task_mode(crate::process::task::TaskMode::Kernel);
    let result = dispatch(&args);
    if let Some(mode) = previous {
        let _ = crate::process::set_current_task_mode(mode);
    }
    return result;
}

fn syscall_name(number: usize) -> &'static str {
    Syscall::from_number(number)
        .map(|syscall| syscall.name())
        .unwrap_or("unknown")
}
