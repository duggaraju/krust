use core::{ptr, slice, str};

use log::{info, warn};
use x86_64::instructions::hlt;

use crate::syscall::dispatch::SyscallArgs;
use crate::syscall::impls;

const MAX_PATH_LEN: usize = 4096;

pub fn sys_read(args: &SyscallArgs) -> isize {
    let fd = args.arg0;
    let ptr = args.arg1 as *mut u8;
    let len = args.arg2;

    if ptr.is_null() {
        warn!("sys_read received a null buffer");
        return -14;
    }

    let buf = unsafe { slice::from_raw_parts_mut(ptr, len) };
    match impls::read(fd, buf) {
        Ok(n) => n as isize,
        Err(errno) => errno,
    }
}

pub fn sys_write(args: &SyscallArgs) -> isize {
    let fd = args.arg0;
    let ptr = args.arg1 as *const u8;
    let len = args.arg2;

    if ptr.is_null() {
        warn!("sys_write received a null buffer");
        return -14;
    }

    let bytes = unsafe { slice::from_raw_parts(ptr, len) };
    if fd == 1 {
        match str::from_utf8(bytes) {
            Ok(message) => {
                info!("{message}");
                len as isize
            }
            Err(_) => {
                warn!("sys_write received non-UTF-8 data");
                -84
            }
        }
    } else {
        match impls::write(fd, bytes) {
            Ok(n) => n as isize,
            Err(errno) => errno,
        }
    }
}

pub fn sys_open(args: &SyscallArgs) -> isize {
    let path_ptr = args.arg0 as *const u8;
    let Some(path) = (unsafe { read_cstr(path_ptr) }) else {
        warn!("sys_open received invalid path pointer");
        return -14;
    };

    match impls::open(path) {
        Ok(fd) => fd as isize,
        Err(errno) => errno,
    }
}

pub fn sys_close(args: &SyscallArgs) -> isize {
    let fd = args.arg0;
    match impls::close(fd) {
        Ok(()) => 0,
        Err(errno) => errno,
    }
}

pub fn sys_stat(args: &SyscallArgs) -> isize {
    let path_ptr = args.arg0 as *const u8;
    let stat_ptr = args.arg1 as *mut impls::Stat;
    if stat_ptr.is_null() {
        return -14;
    }

    let Some(path) = (unsafe { read_cstr(path_ptr) }) else {
        warn!("sys_stat received invalid path pointer");
        return -14;
    };

    match impls::stat(path) {
        Ok(stat) => {
            unsafe { ptr::write(stat_ptr, stat) };
            0
        }
        Err(errno) => errno,
    }
}

pub fn sys_chdir(args: &SyscallArgs) -> isize {
    let path_ptr = args.arg0 as *const u8;
    let Some(path) = (unsafe { read_cstr(path_ptr) }) else {
        warn!("sys_chdir received invalid path pointer");
        return -14;
    };

    match impls::chdir(path) {
        Ok(()) => 0,
        Err(errno) => errno,
    }
}

pub fn sys_times(_args: &SyscallArgs) -> isize {
    impls::times() as isize
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
    crate::arch::shutdown(crate::arch::ShutdownStatus::Success)
}

pub fn sys_getpid(_args: &SyscallArgs) -> isize {
    impls::getpid()
}

pub fn sys_fork(_args: &SyscallArgs) -> isize {
    warn!("sys_fork is unimplemented");
    -38
}

pub fn sys_execve(_args: &SyscallArgs) -> isize {
    warn!("sys_execve is unimplemented");
    -38
}

unsafe fn read_cstr(ptr: *const u8) -> Option<&'static str> {
    if ptr.is_null() {
        return None;
    }

    let mut len = 0usize;
    while len < MAX_PATH_LEN {
        if unsafe { *ptr.add(len) } == 0 {
            let bytes = unsafe { slice::from_raw_parts(ptr, len) };
            return str::from_utf8(bytes).ok();
        }
        len += 1;
    }
    None
}
