#![no_std]
#![no_main]

use core::arch::asm;
use core::ptr::{addr_of, addr_of_mut};
use krust_guest::{argument, call, exit, say};

krust_guest::entry!();
static mut GLOBAL_VALUE: i32 = 19;

fn fork_registers(number: usize, flags: usize, child_tid: usize) -> isize {
    let (result, r12, r13, r14, r15): (isize, usize, usize, usize, usize);
    unsafe {
        asm!("syscall",
            inlateout("rax") number as isize => result,
            in("rdi") flags,
            in("rsi") 0usize,
            in("rdx") 0usize,
            in("r10") child_tid,
            in("r8") 0usize,
            inout("r12") 0x1234_5678usize => r12,
            inout("r13") 0x2345_6789usize => r13,
            inout("r14") 0x3456_7890usize => r14,
            inout("r15") 0x4567_8901usize => r15,
            lateout("rcx") _, lateout("r11") _,
            options(nostack));
    }
    assert_eq!(
        (r12, r13, r14, r15),
        (0x1234_5678, 0x2345_6789, 0x3456_7890, 0x4567_8901)
    );
    result
}

#[repr(C)]
struct WaitResult {
    status: i32,
    guard: u32,
}

fn read_proc(suffix: &[u8], buf: &mut [u8]) -> usize {
    read_proc_pid(call!(39, 0, 0, 0, 0) as usize, suffix, buf)
}

fn read_proc_pid(mut pid: usize, suffix: &[u8], buf: &mut [u8]) -> usize {
    let mut digits = [0u8; 20];
    let mut first = digits.len();
    loop {
        first -= 1;
        digits[first] = b'0' + (pid % 10) as u8;
        pid /= 10;
        if pid == 0 {
            break;
        }
    }
    let mut path = [0u8; 64];
    path[..6].copy_from_slice(b"/proc/");
    let end = 6 + digits.len() - first;
    path[6..end].copy_from_slice(&digits[first..]);
    path[end..end + suffix.len()].copy_from_slice(suffix);
    let fd = call!(2, path.as_ptr(), 0, 0, 0);
    assert!(fd >= 0);
    let count = call!(0, fd, buf.as_mut_ptr(), buf.len(), 0);
    assert!(count >= 0);
    assert_eq!(call!(3, fd, 0, 0, 0), 0);
    count as usize
}

fn check_proc_stat() {
    let mut buf = [0u8; 1024];
    let count = read_proc(b"/stat", &mut buf);
    let text = core::str::from_utf8(&buf[..count]).unwrap();
    assert!(text.ends_with('\n'));
    let (pid, rest) = text.split_once(" (").unwrap();
    assert_eq!(pid.parse::<isize>().unwrap(), call!(39, 0, 0, 0, 0));
    let (_, rest) = rest.rsplit_once(") ").unwrap();
    let mut fields = rest.split_whitespace();
    assert_eq!(fields.next(), Some("R"));
    let mut count = 0;
    for field in fields {
        let value = field.parse::<i64>().unwrap();
        if count == 0 {
            assert_eq!(value, call!(110, 0, 0, 0, 0) as i64);
        }
        if count == 16 {
            assert_eq!(value, 1);
        }
        count += 1;
    }
    assert_eq!(count, 49);
}

fn cpu_usage() -> [u64; 4] {
    cpu_usage_for(call!(39, 0, 0, 0, 0) as usize)
}

fn cpu_usage_for(pid: usize) -> [u64; 4] {
    let mut buf = [0u8; 1024];
    let count = read_proc_pid(pid, b"/stat", &mut buf);
    let text = core::str::from_utf8(&buf[..count]).unwrap();
    let (_, rest) = text.rsplit_once(") ").unwrap();
    let mut values = rest.split_whitespace().skip(11);
    core::array::from_fn(|_| values.next().unwrap().parse().unwrap())
}

fn wall_ns() -> u64 {
    let mut time = [0u64; 2];
    assert_eq!(call!(228, 1, time.as_mut_ptr(), 0, 0), 0);
    time[0] * 1_000_000_000 + time[1]
}

fn user_workload() {
    let start = wall_ns();
    for _ in 0..100_000 {
        for _ in 0..50_000 {
            core::hint::spin_loop();
        }
        if wall_ns() - start >= 80_000_000 {
            return;
        }
    }
    panic!("user workload clock did not advance");
}

fn check_cpu_accounting() {
    let before = cpu_usage();
    let wall_start = wall_ns();
    user_workload();
    let after_user = cpu_usage();
    let wall_elapsed = wall_ns() - wall_start;
    assert!(
        after_user[0] >= before[0] + 2,
        "user CPU time did not advance"
    );
    assert!(
        (after_user[0] - before[0]) * 10_000_000 <= wall_elapsed + 50_000_000,
        "CPU accounting clock exceeds elapsed wall time"
    );
    for _ in 0..128 {
        for _ in 0..32 {
            assert!(call!(39, 0, 0, 0, 0) > 0);
        }
        let current = cpu_usage();
        assert!(current[0] >= after_user[0]);
        if current[1] >= after_user[1] + 2 {
            assert!(
                current[1] - after_user[1] > current[0] - after_user[0],
                "syscall workload must be charged primarily as kernel time"
            );
            say("KRUST_CPU_ACCOUNTING_OK\n");
            return;
        }
    }
    panic!("kernel CPU time did not advance");
}

unsafe fn check_initial_stack(stack: *const usize) {
    assert_eq!(stack as usize % 16, 0);
    let argc = unsafe { stack.read() };
    assert_eq!(unsafe { stack.add(argc + 1).read() }, 0);
    let mut cursor = unsafe { stack.add(argc + 2) };
    let mut envc = 0;
    while unsafe { cursor.read() } != 0 {
        assert!(envc < 256);
        envc += 1;
        cursor = unsafe { cursor.add(1) };
    }
    cursor = unsafe { cursor.add(1) };
    let mut values = [None; 32];
    let mut terminated = false;
    for _ in 0..64 {
        let key = unsafe { cursor.read() };
        let value = unsafe { cursor.add(1).read() };
        if key == 0 {
            assert_eq!(value, 0);
            terminated = true;
            break;
        }
        if key < values.len() {
            assert!(values[key].replace(value).is_none());
        }
        cursor = unsafe { cursor.add(2) };
    }
    assert!(terminated, "unterminated auxiliary vector");
    assert_eq!(values[4], Some(56), "AT_PHENT");
    assert!(values[5].is_some_and(|count| count > 0), "AT_PHNUM");
    assert_eq!(values[6], Some(4096), "AT_PAGESZ");
    assert!(values[9].is_some_and(|entry| entry >= 0x400000), "AT_ENTRY");
    assert_eq!(values[23], Some(0), "AT_SECURE");
    assert!(values[25].is_some_and(|pointer| pointer != 0), "AT_RANDOM");
    assert!(values[31].is_some_and(|pointer| pointer != 0), "AT_EXECFN");
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn guest_main(stack: *const usize, rtld_fini: usize) -> ! {
    assert_eq!(rtld_fini, 0, "static executable finalizer");
    unsafe { check_initial_stack(stack) };
    assert_eq!(call!(334, 0, 0, 0, 0), -38, "unsupported rseq");
    assert_eq!(call!(111, 0, 0, 0, 0), call!(121, 0, 0, 0, 0));
    for flags in [0, 17 | 0x100, 17 | 0x10000, 17 | 0x80000, 17 | 0x100000] {
        assert_eq!(call!(56, flags, 0, 0, 0), -95);
    }
    assert_eq!(call!(56, 17, 1, 0, 0), -95);
    assert_eq!(call!(56, 0x0120_0011, 0, 0, 1), -14);
    for dirfd in [(-100isize) as usize, (-100i32) as u32 as usize] {
        let mut stat = [0u64; 32];
        assert_eq!(call!(262, dirfd, c".".as_ptr(), stat.as_mut_ptr(), 0), 0);
        let fd = call!(257, dirfd, c".".as_ptr(), 0, 0);
        assert!(fd >= 0);
        assert_eq!(call!(3, fd, 0, 0, 0), 0);
        assert_eq!(call!(258, dirfd, c"".as_ptr(), 0o755, 0), -2);
    }
    check_proc_stat();
    let argc = unsafe { stack.read() };
    let name = if argc > 0 {
        unsafe { argument(stack, 0) }
    } else {
        b""
    };
    if matches!(name, b"replacement" | b"replacement-empty") {
        assert!(cpu_usage()[0] >= 2, "exec must preserve CPU usage");
        let mut cmdline = [0u8; 64];
        let count = read_proc(b"/cmdline", &mut cmdline);
        let expected = if name == b"replacement" {
            b"replacement\0\0argument\0".as_slice()
        } else {
            b"replacement-empty\0\0argument\0".as_slice()
        };
        assert_eq!(&cmdline[..count], expected);
        assert_eq!(argc, 3);
        assert_eq!(unsafe { argument(stack, 1) }, b"");
        assert_eq!(unsafe { argument(stack, 2) }, b"argument");
        assert_eq!(unsafe { stack.add(4).read() }, 0);
        let env = unsafe { stack.add(argc + 2) };
        if name == b"replacement-empty" {
            assert_eq!(unsafe { env.read() }, 0);
        } else {
            let pointer = unsafe { env.read() };
            assert_ne!(pointer, 0);
            assert_eq!(
                unsafe { core::ffi::CStr::from_ptr(pointer as *const i8) }.to_bytes(),
                b"TOKEN=kept"
            );
            assert_eq!(unsafe { env.add(1).read() }, 0);
        }
        exit(42);
    }
    check_cpu_accounting();
    let path = c"/bin/proctest".as_ptr();
    let bad_string = [1usize, 0];
    let args = [
        c"replacement".as_ptr() as usize,
        c"".as_ptr() as usize,
        c"argument".as_ptr() as usize,
        0,
    ];
    let env = [c"TOKEN=kept".as_ptr() as usize, 0];
    assert_eq!(call!(59, 1, 0, 0, 0), -14);
    assert_eq!(call!(59, path, 1, 0, 0), -14);
    assert_eq!(call!(59, path, bad_string.as_ptr(), 0, 0), -14);
    assert_eq!(call!(59, path, args.as_ptr(), 1, 0), -14);
    assert_eq!(
        call!(59, c"/bin/missing".as_ptr(), args.as_ptr(), env.as_ptr(), 0),
        -2
    );
    let mapped = call!(9, 0, 128 * 1024, 3, 0x22, usize::MAX, 0);
    assert!(mapped > 0);
    let memory = mapped as *mut u8;
    for index in 0..128 * 1024 {
        unsafe {
            memory.add(index).write_volatile(b'x');
        }
    }
    let huge = [memory as usize, 0];
    assert_eq!(call!(59, path, huge.as_ptr(), 0, 0), -7);
    let cross = unsafe { memory.add(4092).cast::<usize>() };
    for (index, pointer) in args.into_iter().enumerate() {
        unsafe {
            cross.add(index).write_unaligned(pointer);
        }
    }
    let mut result = WaitResult {
        status: -1,
        guard: 0xfeed_cafe,
    };
    assert_eq!(
        call!(61, usize::MAX, addr_of_mut!(result.status), 1, 0),
        -10
    );
    assert_eq!(
        call!(61, usize::MAX, addr_of_mut!(result.status), 2, 0),
        -10
    );
    assert_eq!(
        call!(61, usize::MAX, addr_of_mut!(result.status), 4, 0),
        -22
    );
    for iteration in 0..3 {
        let before_fork = cpu_usage();
        let mut local_value = 23i32;
        unsafe {
            addr_of_mut!(GLOBAL_VALUE).write_volatile(19);
            memory.add(12000).write_volatile(31);
            cross.write_unaligned(if iteration == 2 {
                c"replacement-empty".as_ptr() as usize
            } else {
                args[0]
            });
        }
        let child_tid = unsafe { memory.add(8190).cast::<i32>() };
        unsafe { child_tid.write_unaligned(-123) };
        let child = if iteration == 0 {
            fork_registers(57, 0, 0)
        } else {
            fork_registers(
                56,
                if iteration == 1 { 0x0120_0011 } else { 17 },
                child_tid as usize,
            )
        };
        assert!(child >= 0);
        unsafe {
            assert_eq!(addr_of!(local_value).read_volatile(), 23);
            assert_eq!(addr_of!(GLOBAL_VALUE).read_volatile(), 19);
            assert_eq!(memory.add(12000).read_volatile(), 31);
        }
        if child == 0 {
            assert_eq!(
                unsafe { child_tid.read_unaligned() },
                if iteration == 1 {
                    call!(39, 0, 0, 0, 0) as i32
                } else {
                    -123
                }
            );
            let fresh = cpu_usage();
            assert!(fresh[0] < before_fork[0], "fork must reset CPU usage");
            assert_eq!(&fresh[2..], &[0, 0]);
            user_workload();
            unsafe {
                addr_of_mut!(local_value).write_volatile(99);
                addr_of_mut!(GLOBAL_VALUE).write_volatile(98);
                memory.add(12000).write_volatile(97);
            }
            call!(
                59,
                path,
                cross,
                if iteration == 2 {
                    0
                } else {
                    env.as_ptr() as usize
                },
                0
            );
            panic!("successful exec returned");
        }
        assert_eq!(unsafe { child_tid.read_unaligned() }, -123);
        let after_child = cpu_usage();
        assert!(
            after_child[0] <= before_fork[0] + 2,
            "switched-out time charged to parent"
        );
        let zombie_usage = cpu_usage_for(child as usize);
        user_workload();
        assert_eq!(
            cpu_usage_for(child as usize),
            zombie_usage,
            "zombie usage must be frozen"
        );
        assert_eq!(call!(61, child, 1, 0, 0), -14);
        assert_eq!(call!(61, child, addr_of_mut!(result.status), 2, 0), child);
        assert_eq!((result.status, result.guard), (42 << 8, 0xfeed_cafe));
        let after_wait = cpu_usage();
        assert!(
            after_wait[2] >= before_fork[2] + 2,
            "reaped child user time missing"
        );
        assert!(
            after_wait[3] >= before_fork[3],
            "child kernel time regressed"
        );
        assert_eq!(call!(61, child, addr_of_mut!(result.status), 3, 0), -10);
    }
    let child = fork_registers(57, 0, 0);
    assert!(child >= 0);
    if child == 0 {
        unsafe {
            asm!("mov qword ptr [{address}], 1", address = in(reg) 0usize);
        }
        panic!("null write returned");
    }
    assert_eq!(call!(61, child, addr_of_mut!(result.status), 0, 0), child);
    assert_eq!((result.status, result.guard), (139 << 8, 0xfeed_cafe));
    assert!(call!(39, 0, 0, 0, 0) > 0);
    say("KRUST_PROCESS_OK\n");
    exit(0)
}
