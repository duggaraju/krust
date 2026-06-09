use core::{slice, str};

use log::{info, warn};
use x86_64::instructions::hlt;

use crate::syscall::dispatch::SyscallArgs;

pub fn sys_read(_args: &SyscallArgs) -> isize {
    warn!("sys_read is unimplemented");
    -1
}

pub fn sys_write(args: &SyscallArgs) -> isize {
    let fd = args.arg0;
    let ptr = args.arg1 as *const u8;
    let len = args.arg2;

    if fd != 1 {
        warn!("sys_write only supports stdout (fd=1), got fd={fd}");
        return -1;
    }

    if ptr.is_null() {
        warn!("sys_write received a null buffer");
        return -1;
    }

    let bytes = unsafe { slice::from_raw_parts(ptr, len) };
    match str::from_utf8(bytes) {
        Ok(message) => {
            info!("{message}");
            len as isize
        }
        Err(_) => {
            warn!("sys_write received non-UTF-8 data");
            -1
        }
    }
}

pub fn sys_open(_args: &SyscallArgs) -> isize {
    warn!("sys_open is unimplemented");
    -1
}

pub fn sys_close(_args: &SyscallArgs) -> isize {
    warn!("sys_close is unimplemented");
    -1
}

pub fn sys_exit(args: &SyscallArgs) -> isize {
    let status = args.arg0 as isize;
    info!("sys_exit called with status {}", status);

    loop {
        hlt();
    }
}

pub fn sys_shutdown(_args: &SyscallArgs) -> isize {
    info!("sys_shutdown called");
    crate::exit_qemu(crate::QemuExitCode::Success)
}

pub fn sys_getpid(_args: &SyscallArgs) -> isize {
    1
}

pub fn sys_fork(_args: &SyscallArgs) -> isize {
    warn!("sys_fork is unimplemented");
    -1
}

pub fn sys_execve(_args: &SyscallArgs) -> isize {
    warn!("sys_execve is unimplemented");
    -1
}
