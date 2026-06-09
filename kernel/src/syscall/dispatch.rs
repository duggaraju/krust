use log::warn;

use crate::syscall::{
    handlers,
    numbers::{
        SYS_BRK, SYS_CHDIR, SYS_CLOSE, SYS_EXECVE, SYS_EXIT, SYS_FORK, SYS_GETPID, SYS_MMAP,
        SYS_OPEN, SYS_READ, SYS_SHUTDOWN, SYS_STAT, SYS_TIMES, SYS_WAIT4, SYS_WRITE,
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
    let handler: Option<SyscallHandler> = match args.number {
        SYS_READ => Some(handlers::sys_read),
        SYS_WRITE => Some(handlers::sys_write),
        SYS_OPEN => Some(handlers::sys_open),
        SYS_CLOSE => Some(handlers::sys_close),
        SYS_STAT => Some(handlers::sys_stat),
        SYS_CHDIR => Some(handlers::sys_chdir),
        SYS_TIMES => Some(handlers::sys_times),
        SYS_GETPID => Some(handlers::sys_getpid),
        SYS_FORK => Some(handlers::sys_fork),
        SYS_EXECVE => Some(handlers::sys_execve),
        SYS_EXIT => Some(handlers::sys_exit),
        SYS_SHUTDOWN => Some(handlers::sys_shutdown),
        SYS_MMAP | SYS_BRK | SYS_WAIT4 => None,
        _ => None,
    };

    match handler {
        Some(handler) => handler(args),
        None => {
            warn!("unimplemented syscall {}", args.number);
            -1
        }
    }
}
