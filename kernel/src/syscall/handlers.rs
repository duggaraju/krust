extern crate alloc;

use core::{ptr, slice, str};

use log::{info, warn};
use x86_64::instructions::hlt;

use crate::syscall::dispatch::SyscallArgs;
use crate::syscall::impls;

const MAX_PATH_LEN: usize = 4096;

#[repr(C)]
struct IoVec {
    iov_base: *const u8,
    iov_len: usize,
}

#[repr(C)]
struct TimeSpec {
    tv_sec: i64,
    tv_nsec: i64,
}

#[repr(C)]
struct RLimit {
    rlim_cur: u64,
    rlim_max: u64,
}

#[repr(C)]
struct WinSize {
    ws_row: u16,
    ws_col: u16,
    ws_xpixel: u16,
    ws_ypixel: u16,
}

#[repr(C)]
struct Termios2 {
    c_iflag: u32,
    c_oflag: u32,
    c_cflag: u32,
    c_lflag: u32,
    c_line: u8,
    c_cc: [u8; 19],
    c_ispeed: u32,
    c_ospeed: u32,
}

#[repr(C)]
struct LinuxStat {
    st_dev: u64,
    st_ino: u64,
    st_nlink: u64,
    st_mode: u32,
    st_uid: u32,
    st_gid: u32,
    __pad0: u32,
    st_rdev: u64,
    st_size: i64,
    st_blksize: i64,
    st_blocks: i64,
    st_atime: i64,
    st_atime_nsec: i64,
    st_mtime: i64,
    st_mtime_nsec: i64,
    st_ctime: i64,
    st_ctime_nsec: i64,
    __unused: [i64; 3],
}

const TIOCGWINSZ: usize = 0x5413;
const TCGETS2: usize = 0x802C_542A;
const AT_FDCWD: isize = -100;

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
    if fd == 2 {
        if let Ok(text) = core::str::from_utf8(bytes) {
            info!("stderr: {}", text.trim_end_matches('\0'));
        }
    }
    match impls::write(fd, bytes) {
        Ok(n) => n as isize,
        Err(errno) => errno,
    }
}

pub fn sys_writev(args: &SyscallArgs) -> isize {
    let fd = args.arg0;
    let iov_ptr = args.arg1 as *const IoVec;
    let iov_cnt = args.arg2;
    if iov_ptr.is_null() {
        return -14;
    }
    if iov_cnt == 0 {
        return 0;
    }
    let iovecs = unsafe { slice::from_raw_parts(iov_ptr, iov_cnt) };
    let mut total = 0usize;
    for iov in iovecs {
        if iov.iov_len == 0 {
            continue;
        }
        if iov.iov_base.is_null() {
            return -14;
        }
        let bytes = unsafe { slice::from_raw_parts(iov.iov_base, iov.iov_len) };
        match impls::write(fd, bytes) {
            Ok(written) => total = total.saturating_add(written),
            Err(errno) => {
                if total > 0 {
                    return total as isize;
                }
                return errno;
            }
        }
    }
    total as isize
}

pub fn sys_open(args: &SyscallArgs) -> isize {
    let path_ptr = args.arg0 as *const u8;
    let Some(path) = (unsafe { read_cstr(path_ptr) }) else {
        warn!("sys_open received invalid path pointer");
        return -14;
    };

    match impls::open(path, args.arg1) {
        Ok(fd) => fd as isize,
        Err(errno) => errno,
    }
}

pub fn sys_ioctl(args: &SyscallArgs) -> isize {
    let request = args.arg1;
    let argp = args.arg2;
    match request {
        TIOCGWINSZ => {
            if argp == 0 {
                return -14;
            }
            unsafe {
                ptr::write(
                    argp as *mut WinSize,
                    WinSize {
                        ws_row: 25,
                        ws_col: 80,
                        ws_xpixel: 0,
                        ws_ypixel: 0,
                    },
                );
            }
            0
        }
        TCGETS2 => {
            if argp == 0 {
                return -14;
            }
            unsafe {
                ptr::write(
                    argp as *mut Termios2,
                    Termios2 {
                        c_iflag: 0,
                        c_oflag: 0,
                        c_cflag: 0,
                        c_lflag: 0,
                        c_line: 0,
                        c_cc: [0; 19],
                        c_ispeed: 0,
                        c_ospeed: 0,
                    },
                );
            }
            0
        }
        _ => -25,
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
    let stat_ptr = args.arg1 as *mut LinuxStat;

    if stat_ptr.is_null() {
        return -14;
    }

    let Some(path) = (unsafe { read_cstr(path_ptr) }) else {
        warn!("sys_stat received invalid path pointer");
        return -14;
    };

    match impls::stat(path) {
        Ok(stat) => {
            unsafe { ptr::write(stat_ptr, to_linux_stat(stat)) };
            0
        }
        Err(errno) => errno,
    }
}

pub fn sys_fstat(args: &SyscallArgs) -> isize {
    let fd = args.arg0;
    let stat_ptr = args.arg1 as *mut LinuxStat;
    if stat_ptr.is_null() {
        return -14;
    }
    match impls::fstat(fd) {
        Ok(stat) => {
            unsafe { ptr::write(stat_ptr, to_linux_stat(stat)) };
            0
        }
        Err(errno) => errno,
    }
}

pub fn sys_getdents64(args: &SyscallArgs) -> isize {
    let fd = args.arg0;
    let dirp = args.arg1 as *mut u8;
    let count = args.arg2;
    if dirp.is_null() {
        return -14;
    }
    if count == 0 {
        return -22;
    }
    let buf = unsafe { slice::from_raw_parts_mut(dirp, count) };
    match impls::getdents64(fd, buf) {
        Ok(n) => n as isize,
        Err(errno) => errno,
    }
}

fn to_linux_stat(st: impls::Stat) -> LinuxStat {
    let mode = match st.file_type {
        impls::STAT_TYPE_DIRECTORY => 0o040755,
        impls::STAT_TYPE_CHAR_DEVICE => 0o020666,
        impls::STAT_TYPE_BLOCK_DEVICE => 0o060660,
        impls::STAT_TYPE_SYMLINK => 0o120777,
        _ => 0o100755,
    };
    LinuxStat {
        st_dev: 0,
        st_ino: st.ino,
        st_nlink: 1,
        st_mode: mode,
        st_uid: st.uid,
        st_gid: st.gid,
        __pad0: 0,
        st_rdev: 0,
        st_size: st.size as i64,
        st_blksize: 4096,
        st_blocks: (st.size as i64 + 511) / 512,
        st_atime: 0,
        st_atime_nsec: 0,
        st_mtime: 0,
        st_mtime_nsec: 0,
        st_ctime: 0,
        st_ctime_nsec: 0,
        __unused: [0; 3],
    }
}

pub fn sys_newfstatat(args: &SyscallArgs) -> isize {
    let dirfd = args.arg0 as isize;
    let path_ptr = args.arg1 as *const u8;
    let stat_ptr = args.arg2 as *mut LinuxStat;
    let _flags = args.arg3;
    if stat_ptr.is_null() {
        return -14;
    }
    let Some(path) = (unsafe { read_cstr(path_ptr) }) else {
        return -14;
    };
    if dirfd != AT_FDCWD && !path.starts_with('/') {
        return -95;
    }

    match impls::stat(path) {
        Ok(st) => {
            unsafe { ptr::write(stat_ptr, to_linux_stat(st)) };
            0
        }
        Err(errno) => errno,
    }
}

pub fn sys_getcwd(args: &SyscallArgs) -> isize {
    let buf_ptr = args.arg0 as *mut u8;
    let size = args.arg1;
    if buf_ptr.is_null() {
        return -14;
    }
    if size == 0 {
        return -34;
    }

    match impls::getcwd() {
        Ok(path) => {
            let bytes = path.as_bytes();
            let needed = bytes.len().saturating_add(1);
            if needed > size {
                return -34;
            }

            unsafe {
                ptr::copy_nonoverlapping(bytes.as_ptr(), buf_ptr, bytes.len());
                *buf_ptr.add(bytes.len()) = 0;
            }
            bytes.len() as isize
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

pub fn sys_time(args: &SyscallArgs) -> isize {
    let secs = (crate::time::uptime_ticks() / 100) as i64;
    let tloc = args.arg0 as *mut i64;
    if !tloc.is_null() {
        unsafe {
            ptr::write(tloc, secs);
        }
    }
    secs as isize
}

pub fn sys_getuid(_args: &SyscallArgs) -> isize {
    crate::process::current_uid() as isize
}

pub fn sys_getgid(_args: &SyscallArgs) -> isize {
    crate::process::current_gid() as isize
}

pub fn sys_setuid(args: &SyscallArgs) -> isize {
    let uid = args.arg0 as u32;
    if crate::process::with_current_task_mut(|task| task.set_uid(uid)).is_none() {
        return -1;
    }
    0
}

pub fn sys_setgid(args: &SyscallArgs) -> isize {
    let gid = args.arg0 as u32;
    if crate::process::with_current_task_mut(|task| task.set_gid(gid)).is_none() {
        return -1;
    }
    0
}

pub fn sys_exit(args: &SyscallArgs) -> isize {
    let status = args.arg0 as isize;
    info!("sys_exit called with status {}", status);
    let pid = crate::process::current_pid();
    let _ = crate::process::close_all_fds(pid);
    let switch_plan = {
        let mut scheduler = crate::process::scheduler::SCHEDULER.lock();
        let Some(scheduler) = scheduler.as_mut() else {
            loop {
                hlt();
            }
        };
        let parent_pid = scheduler.task_by_pid(pid).map(|task| task.parent_pid);
        if let Some(task) = scheduler.task_by_pid_mut(pid) {
            task.set_exit_code(status as i32);
        }
        let _ = scheduler.set_task_state(pid, crate::process::task::TaskState::Zombie);
        parent_pid
            .and_then(|ppid| scheduler.switch_to_pid(ppid))
            .or_else(|| scheduler.preempt_and_switch())
    };

    if let Some(plan) = switch_plan {
        unsafe {
            #[cfg(target_arch = "x86_64")]
            crate::arch::x86_64::syscall::restore_kernel_gs_base();
            #[cfg(target_arch = "x86_64")]
            crate::arch::x86_64::context::switch(plan.old_ctx, plan.new_ctx);
        }
    }

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
    match crate::process::fork_current_task() {
        Ok(pid) => pid as isize,
        Err(errno) => errno,
    }
}

pub fn sys_execve(args: &SyscallArgs) -> isize {
    let path_ptr = args.arg0 as *const u8;
    let Some(path) = (unsafe { read_cstr(path_ptr) }) else {
        warn!("sys_execve received invalid path pointer");
        return -14;
    };

    match impls::execve(path) {
        Ok(()) => 0,
        Err(errno) => errno,
    }
}

pub fn sys_wait4(args: &SyscallArgs) -> isize {
    let pid = if args.arg0 == usize::MAX {
        None
    } else {
        Some(args.arg0 as u64)
    };
    let status_ptr = args.arg1 as *mut isize;

    match crate::process::wait_for_child(pid) {
        Ok((child_pid, status)) => {
            if !status_ptr.is_null() {
                unsafe {
                    ptr::write(status_ptr, status as isize);
                }
            }
            child_pid as isize
        }
        Err(errno) => errno,
    }
}

pub fn sys_sched_yield(_args: &SyscallArgs) -> isize {
    let _ = crate::process::sched_yield_current_task();
    0
}

pub fn sys_brk(args: &SyscallArgs) -> isize {
    match impls::brk(args.arg0) {
        Ok(addr) => addr as isize,
        Err(errno) => errno,
    }
}

pub fn sys_arch_prctl(args: &SyscallArgs) -> isize {
    match impls::arch_prctl(args.arg0, args.arg1) {
        Ok(value) => value as isize,
        Err(errno) => errno,
    }
}

pub fn sys_mprotect(_args: &SyscallArgs) -> isize {
    0
}

pub fn sys_mmap(args: &SyscallArgs) -> isize {
    let addr = args.arg0;
    let len = args.arg1;
    let prot = args.arg2;
    let flags = args.arg3;
    let fd_raw = args.arg4;
    let _offset = args.arg5;

    // MAP_ANONYMOUS (0x20) with fd=-1 only.
    let map_anon = (flags & 0x20) != 0;
    let map_fixed = (flags & 0x10) != 0; // MAP_FIXED
    let anon_fd = fd_raw == usize::MAX || fd_raw == u32::MAX as usize;
    if !map_anon || !anon_fd || map_fixed {
        return -12; // ENOMEM (unsupported mode)
    }

    if len == 0 {
        return -22; // EINVAL
    }

    let aligned_len = (len + 4095) & !4095;
    let writable = (prot & 0x2) != 0;

    let result = crate::process::with_current_task_mut(|task| {
        let base = if addr != 0 {
            addr
        } else {
            if task.mmap_end == 0 {
                task.mmap_start
            } else {
                task.mmap_end
            }
        };
        let base = (base + 4095) & !4095;
        let new_end = base.checked_add(aligned_len).ok_or(-12isize)?;
        task.address_space
            .map_zeroed_region(
                base,
                aligned_len,
                if writable {
                    x86_64::structures::paging::PageTableFlags::WRITABLE
                } else {
                    x86_64::structures::paging::PageTableFlags::empty()
                },
            )
            .map_err(|_| -12isize)?;
        if addr == 0 {
            task.mmap_end = new_end;
        }
        Ok::<usize, isize>(base)
    });

    match result {
        Some(Ok(base)) => base as isize,
        _ => -12,
    }
}

pub fn sys_munmap(_args: &SyscallArgs) -> isize {
    -38
}

pub fn sys_lseek(args: &SyscallArgs) -> isize {
    let fd = args.arg0;
    let offset = args.arg1 as i64;
    let whence = args.arg2;
    match impls::lseek(fd, offset, whence) {
        Ok(pos) => pos as isize,
        Err(errno) => errno,
    }
}

pub fn sys_fcntl(args: &SyscallArgs) -> isize {
    let fd = args.arg0;
    let cmd = args.arg1;
    // F_GETFD=1, F_SETFD=2, F_GETFL=3, F_SETFL=4
    match cmd {
        1 => 0, // F_GETFD: no close-on-exec flag
        2 => 0, // F_SETFD: ignore
        3 => {
            // F_GETFL: return O_RDWR for known fds
            if fd <= 2 {
                2
            } else {
                0
            }
        }
        4 => 0,   // F_SETFL: ignore
        _ => -22, // EINVAL
    }
}

pub fn sys_openat(args: &SyscallArgs) -> isize {
    let _dirfd = args.arg0 as isize;
    let path_ptr = args.arg1 as *const u8;
    let flags = args.arg2;
    let _mode = args.arg3;

    let Some(path) = (unsafe { read_cstr(path_ptr) }) else {
        return -14;
    };

    match impls::open(&path, flags) {
        Ok(fd) => fd as isize,
        Err(errno) => errno,
    }
}

pub fn sys_rt_sigaction(_args: &SyscallArgs) -> isize {
    -38
}

pub fn sys_rt_sigprocmask(_args: &SyscallArgs) -> isize {
    -38
}

pub fn sys_nanosleep(args: &SyscallArgs) -> isize {
    let _req = args.arg0 as *const u8;
    let _rem = args.arg1 as *mut u8;
    -38
}

pub fn sys_clock_gettime(args: &SyscallArgs) -> isize {
    let tp = args.arg1 as *mut TimeSpec;
    if tp.is_null() {
        return -14;
    }
    let uptime_ms = crate::process::scheduler::UPTIME_MS.lock();
    let secs = (*uptime_ms / 1000) as i64;
    let nsecs = ((*uptime_ms % 1000) * 1_000_000) as i64;
    unsafe {
        ptr::write(
            tp,
            TimeSpec {
                tv_sec: secs,
                tv_nsec: nsecs,
            },
        );
    }
    0
}

pub fn sys_readlinkat(args: &SyscallArgs) -> isize {
    let _dirfd = args.arg0;
    let path_ptr = args.arg1 as *const u8;
    let buf_ptr = args.arg2 as *mut u8;
    let buf_size = args.arg3;
    if buf_ptr.is_null() {
        return -14;
    }
    if buf_size == 0 {
        return -22;
    }
    let Some(path) = (unsafe { read_cstr(path_ptr) }) else {
        return -14;
    };

    if path == "/proc/self/exe" {
        let exec_path = crate::process::with_current_task_mut(|task| task.exec_path.clone())
            .unwrap_or_default();
        if exec_path.is_empty() {
            return -2;
        }
        let bytes = exec_path.as_bytes();
        let n = core::cmp::min(bytes.len(), buf_size);
        unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), buf_ptr, n) };
        return n as isize;
    }

    -95
}

pub fn sys_prlimit64(args: &SyscallArgs) -> isize {
    let _pid = args.arg0;
    let resource = args.arg1;
    let old_limit = args.arg3 as *mut RLimit;
    if !old_limit.is_null() {
        let (soft, hard) = match resource {
            3 => (8 * 1024 * 1024u64, 8 * 1024 * 1024u64), // RLIMIT_STACK
            _ => (u64::MAX, u64::MAX),
        };
        unsafe {
            ptr::write(
                old_limit,
                RLimit {
                    rlim_cur: soft,
                    rlim_max: hard,
                },
            );
        }
    }
    0
}

pub fn sys_getrandom(args: &SyscallArgs) -> isize {
    let buf_ptr = args.arg0 as *mut u8;
    let len = args.arg1;
    if buf_ptr.is_null() {
        return -14;
    }
    let buf = unsafe { slice::from_raw_parts_mut(buf_ptr, len) };
    for (idx, byte) in buf.iter_mut().enumerate() {
        let ticks = crate::time::uptime_ticks() as usize;
        *byte = ((ticks.wrapping_add(idx * 131)) & 0xff) as u8;
    }
    len as isize
}

pub fn sys_set_tid_address(args: &SyscallArgs) -> isize {
    match impls::set_tid_address(args.arg0) {
        Ok(value) => value as isize,
        Err(errno) => errno,
    }
}

pub fn sys_set_robust_list(args: &SyscallArgs) -> isize {
    match impls::set_robust_list(args.arg0, args.arg1) {
        Ok(value) => value as isize,
        Err(errno) => errno,
    }
}

pub fn sys_rseq(args: &SyscallArgs) -> isize {
    match impls::rseq(args.arg0, args.arg1, args.arg2, args.arg3) {
        Ok(value) => value as isize,
        Err(errno) => errno,
    }
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
