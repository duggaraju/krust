use log::{error, info};

use crate::syscall::{
    handlers,
    numbers::{
        SYS_ARCH_PRCTL, SYS_BRK, SYS_CHDIR, SYS_CLOCK_GETTIME, SYS_CLOSE, SYS_EXECVE, SYS_EXIT,
        SYS_EXIT_GROUP, SYS_FCNTL, SYS_FORK, SYS_FSTAT, SYS_GETCWD, SYS_GETDENTS64, SYS_GETGID,
        SYS_GETPID,
        SYS_GETRANDOM, SYS_GETUID, SYS_IOCTL, SYS_LSEEK, SYS_LSTAT, SYS_MMAP, SYS_MPROTECT,
        SYS_MUNMAP, SYS_NANOSLEEP, SYS_NEWFSTATAT, SYS_OPEN, SYS_OPENAT, SYS_PRLIMIT64, SYS_READ,
        SYS_READLINKAT, SYS_RSEQ, SYS_RT_SIGACTION, SYS_RT_SIGPROCMASK, SYS_SCHED_YIELD,
        SYS_SET_ROBUST_LIST, SYS_SET_TID_ADDRESS, SYS_SETGID, SYS_SETUID, SYS_SHUTDOWN, SYS_STAT,
        SYS_TIME, SYS_TIMES, SYS_WAIT4, SYS_WRITE, SYS_WRITEV,
    },
};

pub type SyscallHandler = fn(&SyscallArgs) -> isize;

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

    let handler: Option<SyscallHandler> = match args.number {
        SYS_READ => Some(handlers::sys_read),
        SYS_WRITE => Some(handlers::sys_write),
        SYS_WRITEV => Some(handlers::sys_writev),
        SYS_IOCTL => Some(handlers::sys_ioctl),
        SYS_OPEN => Some(handlers::sys_open),
        SYS_CLOSE => Some(handlers::sys_close),
        SYS_STAT => Some(handlers::sys_stat),
        SYS_FSTAT => Some(handlers::sys_fstat),
        SYS_GETDENTS64 => Some(handlers::sys_getdents64),
        SYS_NEWFSTATAT => Some(handlers::sys_newfstatat),
        SYS_GETCWD => Some(handlers::sys_getcwd),
        SYS_CHDIR => Some(handlers::sys_chdir),
        SYS_GETUID => Some(handlers::sys_getuid),
        SYS_GETGID => Some(handlers::sys_getgid),
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
            info!(
                "syscall exit: pid={} nr={} ({}) ret={}",
                crate::process::current_pid(),
                args.number,
                syscall_name(args.number),
                result
            );
            if result < 0 {
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

fn syscall_name(number: usize) -> &'static str {
    match number {
        SYS_READ => "read",
        SYS_WRITE => "write",
        SYS_WRITEV => "writev",
        SYS_IOCTL => "ioctl",
        SYS_OPEN => "open",
        SYS_CLOSE => "close",
        SYS_STAT => "stat",
        SYS_FSTAT => "fstat",
        SYS_GETDENTS64 => "getdents64",
        SYS_NEWFSTATAT => "newfstatat",
        SYS_GETCWD => "getcwd",
        SYS_CHDIR => "chdir",
        SYS_GETUID => "getuid",
        SYS_GETGID => "getgid",
        SYS_SETUID => "setuid",
        SYS_SETGID => "setgid",
        SYS_TIMES => "times",
        SYS_TIME => "time",
        SYS_GETPID => "getpid",
        SYS_FORK => "fork",
        SYS_EXECVE => "execve",
        SYS_EXIT => "exit",
        SYS_EXIT_GROUP => "exit_group",
        SYS_SHUTDOWN => "shutdown",
        SYS_WAIT4 => "wait4",
        SYS_SCHED_YIELD => "sched_yield",
        SYS_MMAP => "mmap",
        SYS_MUNMAP => "munmap",
        SYS_MPROTECT => "mprotect",
        SYS_BRK => "brk",
        SYS_ARCH_PRCTL => "arch_prctl",
        SYS_SET_TID_ADDRESS => "set_tid_address",
        SYS_CLOCK_GETTIME => "clock_gettime",
        SYS_READLINKAT => "readlinkat",
        SYS_SET_ROBUST_LIST => "set_robust_list",
        SYS_PRLIMIT64 => "prlimit64",
        SYS_GETRANDOM => "getrandom",
        SYS_RSEQ => "rseq",
        SYS_LSTAT => "lstat",
        SYS_LSEEK => "lseek",
        SYS_FCNTL => "fcntl",
        SYS_OPENAT => "openat",
        SYS_RT_SIGACTION => "rt_sigaction",
        SYS_RT_SIGPROCMASK => "rt_sigprocmask",
        SYS_NANOSLEEP => "nanosleep",
        _ => "unknown",
    }
}
