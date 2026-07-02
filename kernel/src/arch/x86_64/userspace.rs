extern crate alloc;

use alloc::string::String;
use core::ptr;

pub type PageFlags = x86_64::structures::paging::PageTableFlags;

const ARCH_SET_GS: usize = 0x1001;
const ARCH_SET_FS: usize = 0x1002;
const ARCH_GET_FS: usize = 0x1003;
const ARCH_GET_GS: usize = 0x1004;

const PAGE_SIZE: usize = 4096;
const USER_BRK_MAX: usize = 0x0000_7fff_0000_0000;
const USER_STACK_TOP: usize = 0x0000_7fff_ffff_f000;
const USER_STACK_SIZE: usize = 8 * 4096;
const USER_STACK_RESERVE: usize = 256 * 4096;
const USER_STACK_MAP_SIZE: usize = USER_STACK_SIZE + USER_STACK_RESERVE + PAGE_SIZE;

const AT_NULL: usize = 0;
const AT_PHDR: usize = 3;
const AT_PHENT: usize = 4;
const AT_PHNUM: usize = 5;
const AT_PAGESZ: usize = 6;
const AT_BASE: usize = 7;
const AT_FLAGS: usize = 8;
const AT_ENTRY: usize = 9;
const AT_UID: usize = 11;
const AT_EUID: usize = 12;
const AT_GID: usize = 13;
const AT_EGID: usize = 14;
const AT_PLATFORM: usize = 15;
const AT_HWCAP: usize = 16;
const AT_CLKTCK: usize = 17;
const AT_SECURE: usize = 23;
const AT_RANDOM: usize = 25;
const AT_HWCAP2: usize = 26;
const AT_EXECFN: usize = 31;
const CLKTCK: usize = 100;

pub fn writable_user_page_flags() -> PageFlags {
    PageFlags::WRITABLE
}

pub fn empty_user_page_flags() -> PageFlags {
    PageFlags::empty()
}

pub fn user_brk_max() -> usize {
    USER_BRK_MAX
}

pub fn user_mmap_base() -> usize {
    0x0000_6000_0000_0000
}

pub fn sync_current_task_user_bases() {
    let _ = crate::process::with_current_task_mut(|task| {
        super::syscall::set_user_bases(
            task.context.thread_local_base(true),
            task.context.thread_local_base(false),
        );
    });
}

pub fn activate_task_runtime_state(task: &crate::process::task::Task) {
    super::gdt::set_tss_rsp0(task.kernel_stack_top as u64);
    super::syscall::set_kernel_stack_top(task.kernel_stack_top as u64);
    super::syscall::set_user_bases(
        task.context.thread_local_base(true),
        task.context.thread_local_base(false),
    );
}

pub fn arch_prctl(code: usize, addr: usize) -> Result<usize, isize> {
    crate::process::with_current_task_mut(|task| match code {
        ARCH_SET_FS => {
            task.context.set_thread_local_base(addr as u64, true);
            super::syscall::set_user_bases(
                task.context.thread_local_base(true),
                task.context.thread_local_base(false),
            );
            Ok(0)
        }
        ARCH_SET_GS => {
            task.context.set_thread_local_base(addr as u64, false);
            super::syscall::set_user_bases(
                task.context.thread_local_base(true),
                task.context.thread_local_base(false),
            );
            Ok(0)
        }
        ARCH_GET_FS => {
            if addr == 0 {
                return Err(-14isize);
            }
            unsafe {
                ptr::write(addr as *mut u64, task.context.thread_local_base(true));
            }
            Ok(0)
        }
        ARCH_GET_GS => {
            if addr == 0 {
                return Err(-14isize);
            }
            unsafe {
                ptr::write(addr as *mut u64, task.context.thread_local_base(false));
            }
            Ok(0)
        }
        _ => Err(-22isize),
    })
    .ok_or(-1isize)?
}

pub fn build_initial_user_stack(
    address_space: &mut crate::mm::address_space::AddressSpace,
    argv: &[String],
    envp: &[String],
    exec_path: &str,
    plan: &crate::process::binfmt::LoadPlan,
) -> Result<usize, isize> {
    let stack_base = USER_STACK_TOP - (USER_STACK_SIZE + USER_STACK_RESERVE);
    address_space
        .map_zeroed_region(stack_base, USER_STACK_MAP_SIZE, writable_user_page_flags())
        .map_err(|_| -5isize)?;

    let mut sp = USER_STACK_TOP - USER_STACK_RESERVE;

    let env_ptr_scan_start = sp;
    for value in envp.iter().rev() {
        let bytes = value.as_bytes();
        sp = sp.checked_sub(bytes.len() + 1).ok_or(-7isize)?;
        address_space.write_bytes(sp, bytes).map_err(|_| -5isize)?;
        address_space
            .write_bytes(sp + bytes.len(), &[0])
            .map_err(|_| -5isize)?;
    }

    let argv_ptr_scan_start = sp;
    for value in argv.iter().rev() {
        let bytes = value.as_bytes();
        sp = sp.checked_sub(bytes.len() + 1).ok_or(-7isize)?;
        address_space.write_bytes(sp, bytes).map_err(|_| -5isize)?;
        address_space
            .write_bytes(sp + bytes.len(), &[0])
            .map_err(|_| -5isize)?;
    }

    let platform = b"x86_64";
    sp -= platform.len() + 1;
    address_space
        .write_bytes(sp, platform)
        .map_err(|_| -5isize)?;
    address_space
        .write_bytes(sp + platform.len(), &[0])
        .map_err(|_| -5isize)?;
    let platform_addr = sp;

    let exec_bytes = exec_path.as_bytes();
    sp -= exec_bytes.len() + 1;
    address_space
        .write_bytes(sp, exec_bytes)
        .map_err(|_| -5isize)?;
    address_space
        .write_bytes(sp + exec_bytes.len(), &[0])
        .map_err(|_| -5isize)?;
    let execfn_addr = sp;

    sp -= 16;
    let random_addr = sp;
    let random_seed = [0x5Au8; 16];
    address_space
        .write_bytes(random_addr, &random_seed)
        .map_err(|_| -5isize)?;

    sp &= !0xfusize;

    let push_u64 = |space: &mut crate::mm::address_space::AddressSpace,
                    sp: &mut usize,
                    value: usize|
     -> Result<(), isize> {
        *sp -= 8;
        let bytes = (value as u64).to_ne_bytes();
        space.write_bytes(*sp, &bytes).map_err(|_| -5isize)
    };

    let push_auxv = |space: &mut crate::mm::address_space::AddressSpace,
                     sp: &mut usize,
                     key: usize,
                     value: usize|
     -> Result<(), isize> {
        push_u64(space, sp, value)?;
        push_u64(space, sp, key)
    };

    push_auxv(address_space, &mut sp, AT_NULL, 0)?;
    push_auxv(address_space, &mut sp, AT_EXECFN, execfn_addr)?;
    push_auxv(address_space, &mut sp, AT_HWCAP2, 0)?;
    push_auxv(address_space, &mut sp, AT_RANDOM, random_addr)?;
    push_auxv(address_space, &mut sp, AT_SECURE, 0)?;
    push_auxv(address_space, &mut sp, AT_CLKTCK, CLKTCK)?;
    push_auxv(address_space, &mut sp, AT_HWCAP, 0)?;
    push_auxv(address_space, &mut sp, AT_PLATFORM, platform_addr)?;
    push_auxv(address_space, &mut sp, AT_EGID, 0)?;
    push_auxv(address_space, &mut sp, AT_GID, 0)?;
    push_auxv(address_space, &mut sp, AT_EUID, 0)?;
    push_auxv(address_space, &mut sp, AT_UID, 0)?;
    push_auxv(address_space, &mut sp, AT_ENTRY, plan.entry_point)?;
    push_auxv(address_space, &mut sp, AT_FLAGS, 0)?;
    push_auxv(address_space, &mut sp, AT_BASE, 0)?;
    push_auxv(address_space, &mut sp, AT_PAGESZ, PAGE_SIZE)?;
    push_auxv(address_space, &mut sp, AT_PHNUM, plan.phnum)?;
    push_auxv(address_space, &mut sp, AT_PHENT, plan.phent_size)?;
    push_auxv(address_space, &mut sp, AT_PHDR, plan.phdr_addr)?;

    let table_qwords = 1usize + argv.len() + 1 + envp.len() + 1;
    let desired_sp_mod_16 = if table_qwords % 2 == 0 { 0 } else { 8 };
    while (sp % 16) != desired_sp_mod_16 {
        push_u64(address_space, &mut sp, 0)?;
    }

    push_u64(address_space, &mut sp, 0)?;
    let mut env_scan_sp = env_ptr_scan_start;
    for value in envp.iter().rev() {
        env_scan_sp = env_scan_sp
            .checked_sub(value.as_bytes().len() + 1)
            .ok_or(-7isize)?;
        push_u64(address_space, &mut sp, env_scan_sp)?;
    }
    push_u64(address_space, &mut sp, 0)?;
    let mut argv_scan_sp = argv_ptr_scan_start;
    for value in argv.iter().rev() {
        argv_scan_sp = argv_scan_sp
            .checked_sub(value.as_bytes().len() + 1)
            .ok_or(-7isize)?;
        push_u64(address_space, &mut sp, argv_scan_sp)?;
    }
    push_u64(address_space, &mut sp, argv.len())?;

    Ok(sp)
}
