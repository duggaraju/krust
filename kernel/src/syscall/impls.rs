extern crate alloc;

use crate::fs::vfs::{self, DirCursor, DirEntry, File, FileType, FsError, Inode, SeekFrom};
use crate::process;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

pub const STAT_TYPE_REGULAR: u32 = 1;
pub const STAT_TYPE_DIRECTORY: u32 = 2;
pub const STAT_TYPE_CHAR_DEVICE: u32 = 3;
pub const STAT_TYPE_BLOCK_DEVICE: u32 = 4;
pub const STAT_TYPE_SYMLINK: u32 = 5;

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct Stat {
    pub ino: u64,
    pub file_type: u32,
    pub uid: u32,
    pub gid: u32,
    pub size: u64,
    pub mode: u32,
}

pub fn open(path: &str, _flags: usize) -> Result<usize, isize> {
    let absolute = resolve_absolute_from_current_cwd(path).map_err(errno_from_fs)?;
    let file = vfs::open_path(absolute.as_str()).map_err(errno_from_fs)?;
    process::open_file(file)
}

pub fn close(fd: usize) -> Result<(), isize> {
    process::close_fd(fd)
}

pub fn read(fd: usize, buf: &mut [u8]) -> Result<usize, isize> {
    process::read_fd(fd, buf)
}

pub fn write(fd: usize, buf: &[u8]) -> Result<usize, isize> {
    process::write_fd(fd, buf)
}

pub fn readdir(
    fd: usize,
    cursor: &mut DirCursor,
    name_buf: &mut [u8],
    visit: &mut dyn for<'a> FnMut(DirEntry<'a>) -> bool,
) -> Result<usize, isize> {
    let fd_desc = process::fd_descriptor(fd)?;
    let file = vfs::open_by_descriptor(fd_desc).map_err(errno_from_fs)?;
    file.readdir(cursor, name_buf, visit).map_err(errno_from_fs)
}

pub fn getdents64(fd: usize, out: &mut [u8]) -> Result<usize, isize> {
    if out.is_empty() {
        return Err(-22);
    }

    process::with_fd(fd, |file| {
        if file.file_type() != FileType::Directory {
            return Err(-20);
        }

        let mut cursor = DirCursor {
            offset: file.position() as u64,
        };
        let mut name_buf = [0u8; 256];
        let mut written = 0usize;

        loop {
            let prev_offset = cursor.offset;
            let mut captured: Option<(u64, FileType, String)> = None;
            let mut visit = |entry: DirEntry<'_>| {
                captured = Some((entry.ino, entry.file_type, String::from(entry.name)));
                false
            };

            let emitted = file
                .readdir(&mut cursor, &mut name_buf, &mut visit)
                .map_err(errno_from_fs)?;
            if emitted == 0 {
                break;
            }

            let Some((ino, file_type, name)) = captured else {
                break;
            };
            let reclen = align_up_8(19 + name.len() + 1);
            if reclen > out.len().saturating_sub(written) {
                cursor.offset = prev_offset;
                break;
            }

            let entry = &mut out[written..written + reclen];
            entry.fill(0);
            entry[0..8].copy_from_slice(&ino.to_ne_bytes());
            entry[8..16].copy_from_slice(&cursor.offset.to_ne_bytes());
            entry[16..18].copy_from_slice(&(reclen as u16).to_ne_bytes());
            entry[18] = dirent_type(file_type);
            entry[19..19 + name.len()].copy_from_slice(name.as_bytes());
            written += reclen;
        }

        file.seek(SeekFrom::Start(cursor.offset as usize))
            .map_err(errno_from_fs)?;
        Ok(written)
    })
}

pub fn stat(path: &str) -> Result<Stat, isize> {
    let inode = resolve_path_from_current_cwd(path).map_err(errno_from_fs)?;
    Ok(Stat {
        ino: inode.ino(),
        file_type: file_type_to_u32(inode.file_type()),
        uid: inode.uid(),
        gid: inode.gid(),
        size: inode.size() as u64,
        mode: u32::from(inode.mode()),
    })
}

pub fn fstat(fd: usize) -> Result<Stat, isize> {
    let fd_desc = process::fd_descriptor(fd)?;
    let file = vfs::open_by_descriptor(fd_desc).map_err(errno_from_fs)?;
    let inode = file.inode();
    Ok(Stat {
        ino: inode.ino(),
        file_type: file_type_to_u32(inode.file_type()),
        uid: inode.uid(),
        gid: inode.gid(),
        size: inode.size() as u64,
        mode: u32::from(inode.mode()),
    })
}

pub fn getcwd() -> Result<String, isize> {
    crate::process::current_cwd_path().ok_or(-2)
}

pub fn chdir(path: &str) -> Result<(), isize> {
    let inode = resolve_path_from_current_cwd(path).map_err(errno_from_fs)?;
    if inode.file_type() != FileType::Directory {
        return Err(-20);
    }
    let cwd_path = resolve_absolute_from_current_cwd(path).map_err(errno_from_fs)?;
    process::set_current_cwd(inode, cwd_path)?;
    Ok(())
}

pub fn mkdir(path: &str) -> Result<(), isize> {
    create_from_cwd_from_current_cwd(path, FileType::Directory)
        .map(|_| ())
        .map_err(errno_from_fs)
}

pub fn write_file(path: &str, data: &[u8]) -> Result<usize, isize> {
    let absolute = resolve_absolute_from_current_cwd(path).map_err(errno_from_fs)?;
    let inode = match vfs::lookup_path(absolute.as_str()) {
        Ok(inode) => inode,
        Err(FsError::NotFound) => {
            vfs::create_path(absolute.as_str(), FileType::Regular).map_err(errno_from_fs)?
        }
        Err(err) => return Err(errno_from_fs(err)),
    };

    inode.write(0, data).map_err(errno_from_fs)
}

pub fn read_file(path: &str, max_bytes: usize) -> Result<Vec<u8>, isize> {
    let fd = open(path, 0)?;
    let result = (|| {
        let mut out = Vec::new();
        let mut buf = [0u8; 256];
        while out.len() < max_bytes {
            let remaining = max_bytes - out.len();
            let chunk_len = core::cmp::min(remaining, buf.len());
            let n = read(fd, &mut buf[..chunk_len])?;
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
        }
        Ok(out)
    })();
    let _ = close(fd);
    result
}

pub fn read_file_string(path: &str, max_bytes: usize) -> Result<String, isize> {
    let bytes = read_file(path, max_bytes)?;
    String::from_utf8(bytes).map_err(|_| -84)
}

pub fn getpid() -> isize {
    process::current_pid() as isize
}

pub fn brk(addr: usize) -> Result<usize, isize> {
    const PAGE_SIZE: usize = 4096;

    process::with_current_task_mut(|task| {
        let current = task.brk_end;
        if task.brk_start == 0 {
            return Err(-12isize);
        }
        if addr == 0 {
            return Ok(current);
        }
        if addr < task.brk_start || addr > crate::arch::user_brk_max() {
            return Ok(current);
        }

        if addr > task.brk_mapped_end {
            let mapped_target = (addr + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
            let map_size = mapped_target.saturating_sub(task.brk_mapped_end);
            task.address_space
                .map_zeroed_region(
                    task.brk_mapped_end,
                    map_size,
                    crate::arch::writable_user_page_flags(),
                )
                .map_err(|_| -12isize)?;
            task.brk_mapped_end = mapped_target;
        }

        task.brk_end = addr;
        Ok(task.brk_end)
    })
    .ok_or(-1isize)?
}

/// Returns the current brk end (mmap can use this as next available address).
pub fn brk_query() -> Option<usize> {
    process::with_current_task_mut(|task| task.brk_end)
}

/// Forcibly advance brk_end to `end` without mapping pages (caller already mapped them).
pub fn brk_set_end(end: usize) -> Result<(), isize> {
    process::with_current_task_mut(|task| {
        task.brk_end = end;
        Ok(())
    })
    .ok_or(-1isize)?
}

/// Map `len` anonymous zero-filled bytes starting at `addr` into the current address space.
pub fn mmap_anon(addr: usize, len: usize, writable: bool) -> Result<(), isize> {
    let mut flags = crate::arch::empty_user_page_flags();
    if writable {
        flags |= crate::arch::writable_user_page_flags();
    }
    process::with_current_task_mut(|task| {
        task.address_space
            .map_zeroed_region(addr, len, flags)
            .map(|_| ())
            .map_err(|_| -12isize)
    })
    .ok_or(-12isize)?
}

/// Seek on an open file descriptor.
pub fn lseek(fd: usize, offset: i64, whence: usize) -> Result<i64, isize> {
    let seek_from = match whence {
        0 => SeekFrom::Start(offset as usize),
        1 => SeekFrom::Current(offset as isize),
        2 => SeekFrom::End(offset as isize),
        _ => return Err(-22),
    };
    process::with_fd(fd, |file| {
        file.seek(seek_from)
            .map(|pos| pos as i64)
            .map_err(errno_from_fs)
    })
}

pub fn arch_prctl(code: usize, addr: usize) -> Result<usize, isize> {
    crate::arch::arch_prctl(code, addr)
}

pub fn set_tid_address(_clear_child_tid: usize) -> Result<usize, isize> {
    Ok(process::current_pid() as usize)
}

pub fn set_robust_list(_head: usize, _len: usize) -> Result<usize, isize> {
    Err(-38)
}

pub fn rseq(
    _rseq_ptr: usize,
    _rseq_len: usize,
    _flags: usize,
    _sig: usize,
) -> Result<usize, isize> {
    Ok(0)
}

pub fn fork() -> Result<u64, isize> {
    let rsp = current_kernel_rsp();
    let on_task_stack = crate::process::with_current_task_mut(|task| {
        let (base, top) = task.kernel_stack_range();
        let rsp = rsp as usize;
        rsp >= base && rsp <= top
    })
    .unwrap_or(false);
    if !on_task_stack {
        log::warn!(
            "fork: kernel caller rsp=0x{:x} is outside task kernel stack; refusing resumable fork",
            rsp
        );
        return Err(-38);
    }

    let _ = crate::process::with_current_task_mut(|task| {
        unsafe {
            crate::arch::capture_current_context(&mut task.context);
        }
        task.context
            .set_kernel_stack_top(task.kernel_stack_top as u64);
        task.context
            .set_address_space_root(task.address_space.root_paddr());
    });

    let child_pid = crate::process::fork_current_task()?;

    if let Some(plan) = crate::process::yield_to_task(child_pid) {
        unsafe {
            crate::arch::restore_kernel_syscall_state();
            crate::arch::switch_context(plan.old_ctx, plan.new_ctx);
        }
    }

    if crate::process::current_pid() == child_pid {
        Ok(0)
    } else {
        Ok(child_pid)
    }
}

pub fn fork_execve(path: String, argv: Vec<String>) -> Result<u64, isize> {
    let child_pid = crate::process::fork_current_task()?;

    let configured = crate::process::with_task_mut(child_pid, |task| {
        task.exec_path = path.clone();
        task.set_argv(argv.clone());
        task.context
            .set_kernel_entry_point(fork_execve_child_entry as *const () as usize as u64);
        task.context
            .set_kernel_stack_pointer(task.kernel_stack_top as u64);
        task.context.set_frame_pointer(task.kernel_stack_top as u64);
        task.context
            .set_kernel_stack_top(task.kernel_stack_top as u64);
        task.userland = false;
        task.mode = process::task::TaskMode::Kernel;
    });
    if configured.is_none() {
        return Err(-1);
    }

    if let Some(plan) = crate::process::yield_to_task(child_pid) {
        unsafe {
            crate::arch::restore_kernel_syscall_state();
            crate::arch::switch_context(plan.old_ctx, plan.new_ctx);
        }
    }

    Ok(child_pid)
}

fn fork_execve_child_entry() {
    let Some((exec_path, argv)) = crate::process::with_current_task_mut(|task| {
        (
            core::mem::take(&mut task.exec_path),
            core::mem::take(&mut task.argv),
        )
    }) else {
        let _ = crate::syscall::handlers::sys_exit(&crate::syscall::dispatch::SyscallArgs {
            number: crate::syscall::numbers::SYS_EXIT,
            arg0: 127,
            arg1: 0,
            arg2: 0,
            arg3: 0,
            arg4: 0,
            arg5: 0,
        });
        loop {
            x86_64::instructions::hlt();
        }
    };

    let envp = crate::process::with_current_task_mut(|task| {
        task.env
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
    })
    .unwrap_or_default();

    if let Err(_errno) = execve_with_args(exec_path.as_str(), argv, envp) {
        let _ = crate::syscall::handlers::sys_exit(&crate::syscall::dispatch::SyscallArgs {
            number: crate::syscall::numbers::SYS_EXIT,
            arg0: 127,
            arg1: 0,
            arg2: 0,
            arg3: 0,
            arg4: 0,
            arg5: 0,
        });
    }

    loop {
        x86_64::instructions::hlt();
    }
}

#[inline]
fn current_kernel_rsp() -> u64 {
    let rsp: u64;
    unsafe {
        core::arch::asm!("mov {}, rsp", out(reg) rsp, options(nostack, nomem, preserves_flags));
    }
    rsp
}

pub fn execve(path: &str) -> Result<(), isize> {
    execve_with_args(path, vec![String::from(path)], Vec::new())
}

pub fn execve_with_args(path: &str, argv: Vec<String>, envp: Vec<String>) -> Result<(), isize> {
    // Normalize the caller-supplied path. This is kept as the user-visible path
    // (argv[0] default, AT_EXECFN) so that e.g. busybox applets see their own name.
    let original_path = resolve_absolute_from_current_cwd(path).map_err(errno_from_fs)?;

    // Resolve any symlinks to find the real binary to load.
    // If resolution fails (filesystem not ready, etc.) fall back to the original path.
    let mut request_path =
        vfs::resolve_symlink_path(original_path.as_str()).unwrap_or_else(|_| original_path.clone());

    if request_path != original_path {
        log::info!(
            "execve: symlink resolved '{}' → '{}'",
            original_path,
            request_path
        );
    }

    let mut request = crate::process::binfmt::ExecRequest {
        path: request_path.clone(),
        argv,
        envp,
    };

    loop {
        let action = crate::process::binfmt::probe_path(&request).map_err(|err| match err {
            crate::process::binfmt::BinfmtError::Io(errno) => errno,
            crate::process::binfmt::BinfmtError::NotExecutableFormat => -8,
            crate::process::binfmt::BinfmtError::AlreadyRegistered
            | crate::process::binfmt::BinfmtError::NotFound => -38,
            crate::process::binfmt::BinfmtError::InvalidFormat => -8,
        })?;

        match action {
            crate::process::binfmt::BinaryFormatAction::Redirect(mut next) => {
                // The redirect target (e.g. a shebang interpreter) may itself be a symlink.
                let resolved_next = vfs::resolve_symlink_path(next.path.as_str())
                    .unwrap_or_else(|_| next.path.clone());
                if resolved_next != next.path {
                    log::info!(
                        "execve: redirect target symlink '{}' → '{}'",
                        next.path,
                        resolved_next,
                    );
                    next.path = resolved_next;
                }
                log::info!(
                    "execve: redirecting '{}' → '{}' with argv={:?}",
                    request.path,
                    next.path,
                    next.argv,
                );
                request_path = next.path.clone();
                request = next;
            }
            crate::process::binfmt::BinaryFormatAction::Load(plan) => {
                if plan.interpreter.is_some() {
                    return Err(-8);
                }

                log::info!(
                    "execve: loading '{}' format='{}' entry=0x{:x} segments={}",
                    request_path,
                    plan.format,
                    plan.entry_point,
                    plan.segments.len(),
                );

                let inode =
                    resolve_path_from_current_cwd(request_path.as_str()).map_err(errno_from_fs)?;
                let inode_size = inode.size();
                let mut image_space =
                    crate::mm::address_space::AddressSpace::kernel().map_err(|_| -5isize)?;
                for segment in &plan.segments {
                    log::info!(
                        "execve: map segment vaddr=0x{:x} file_off=0x{:x} file_size=0x{:x} mem_size=0x{:x} flags=0x{:x}",
                        segment.virtual_address,
                        segment.file_offset,
                        segment.file_size,
                        segment.memory_size,
                        segment.flags.bits()
                    );
                    if segment.file_offset.saturating_add(segment.file_size) > inode_size {
                        return Err(-8);
                    }
                    image_space
                        .map_segment(segment, |offset, buf| {
                            inode
                                .read(offset, buf)
                                .map_err(|_| crate::mm::address_space::MmError::IoFailed)
                        })
                        .map_err(|_| -5isize)?;
                }
                log::info!("execve: segment mapping complete for '{}'", request_path);

                log::info!(
                    "execve: building user stack argv={} envp={}",
                    request.argv.len(),
                    request.envp.len()
                );

                // Ensure we always have at least a minimal environment for glibc compatibility
                let mut envp_vec = request.envp.clone();
                if envp_vec.is_empty() {
                    // Add minimal PATH environment for glibc
                    envp_vec.push(String::from("PATH=/bin"));
                    log::info!("execve: added minimal PATH environment");
                }

                let stack_top = build_user_stack(
                    &mut image_space,
                    &request.argv,
                    &envp_vec,
                    original_path.as_str(), // AT_EXECFN: original name passed to execve
                    &plan,
                )?;
                log::info!("execve: user stack ready sp=0x{:x}", stack_top);

                process::with_current_task_mut(|task| {
                    let old_space = core::mem::replace(&mut task.address_space, image_space);
                    task.set_argv(request.argv.clone());
                    task.exec_path = original_path.clone(); // visible name, not resolved symlink
                    task.prepare_user_exec(plan.entry_point, stack_top);
                    let brk_start = plan
                        .segments
                        .iter()
                        .map(|segment| {
                            segment
                                .virtual_address
                                .saturating_add(segment.memory_size)
                        })
                        .max()
                        .map(|value| (value + 4095) & !4095)
                        .unwrap_or(0);
                    task.initialize_brk(brk_start);
                    task.set_mmap_start(crate::arch::user_mmap_base());
                    task.context.set_address_space_root(task.address_space.root_paddr());
                    task.address_space.activate();
                    task.context.set_kernel_stack_top(task.kernel_stack_top as u64);
                    log::info!(
                        "execve: task prepared pid={} entry=0x{:x} user_sp=0x{:x} cr3=0x{:x} brk_start=0x{:x}",
                        task.pid,
                        plan.entry_point,
                        stack_top,
                        task.context.address_space_root(),
                        brk_start
                    );
                    drop(old_space);
                })
                .ok_or(-1isize)?;

                log::info!(
                    "execve: entering usermode pid={} entry=0x{:x} stack=0x{:x}",
                    crate::process::current_pid(),
                    plan.entry_point,
                    stack_top
                );

                // Set task mode to User before entering usermode
                let _ = process::set_current_task_mode(process::task::TaskMode::User);

                unsafe {
                    crate::arch::enter_usermode();
                }
            }
        }
    }
}

pub fn times() -> u64 {
    crate::time::uptime_ticks()
}

pub fn errno_from_fs(err: FsError) -> isize {
    match err {
        FsError::NotFound => -2,
        FsError::PermissionDenied => -13,
        FsError::NotADirectory => -20,
        FsError::IsADirectory => -21,
        FsError::AlreadyExists => -17,
        FsError::IoError => -5,
        FsError::Busy => -16,
        FsError::NotSupported => -95,
    }
}

fn build_user_stack(
    address_space: &mut crate::mm::address_space::AddressSpace,
    argv: &[String],
    envp: &[String],
    exec_path: &str,
    plan: &crate::process::binfmt::LoadPlan,
) -> Result<usize, isize> {
    crate::arch::build_initial_user_stack(address_space, argv, envp, exec_path, plan)
}

fn file_type_to_u32(file_type: FileType) -> u32 {
    match file_type {
        FileType::Regular => STAT_TYPE_REGULAR,
        FileType::Directory => STAT_TYPE_DIRECTORY,
        FileType::CharDevice => STAT_TYPE_CHAR_DEVICE,
        FileType::BlockDevice => STAT_TYPE_BLOCK_DEVICE,
        FileType::Symlink => STAT_TYPE_SYMLINK,
    }
}

fn dirent_type(file_type: FileType) -> u8 {
    match file_type {
        FileType::Regular => 8,     // DT_REG
        FileType::Directory => 4,   // DT_DIR
        FileType::CharDevice => 2,  // DT_CHR
        FileType::BlockDevice => 6, // DT_BLK
        FileType::Symlink => 10,    // DT_LNK
    }
}

fn align_up_8(value: usize) -> usize {
    (value + 7) & !7
}

fn resolve_path_from_current_cwd(path: &str) -> Result<Arc<dyn Inode>, FsError> {
    let absolute = resolve_absolute_from_current_cwd(path)?;
    vfs::lookup_path(absolute.as_str())
}

fn resolve_absolute_from_current_cwd(path: &str) -> Result<String, FsError> {
    let path = path.trim();
    if path.is_empty() {
        return Err(FsError::NotFound);
    }
    if path.starts_with('/') {
        return vfs::normalize_absolute_path(path);
    }
    let cwd_path = process::current_cwd_path().unwrap_or_else(|| String::from("/"));
    let joined = if cwd_path == "/" {
        format!("/{}", path)
    } else {
        format!("{}/{}", cwd_path.trim_end_matches('/'), path)
    };
    vfs::normalize_absolute_path(joined.as_str())
}

fn resolve_parent_from_current_cwd(
    path: &str,
) -> Result<(alloc::sync::Arc<dyn vfs::Inode>, String), FsError> {
    let absolute = resolve_absolute_from_current_cwd(path)?;
    if absolute == "/" {
        return Err(FsError::NotFound);
    }
    let (parent_path, name) = absolute
        .rsplit_once('/')
        .unwrap_or(("/", absolute.as_str()));
    let parent = if parent_path.is_empty() || parent_path == "/" {
        vfs::lookup_path("/")?
    } else {
        vfs::lookup_path(parent_path)?
    };
    Ok((parent, String::from(name)))
}

fn create_from_cwd_from_current_cwd(
    path: &str,
    file_type: FileType,
) -> Result<Arc<dyn Inode>, FsError> {
    let (parent, name) = resolve_parent_from_current_cwd(path)?;
    parent.create(name.as_str(), file_type)
}
