#![no_std]

use core::arch::asm;
use core::fmt::{self, Write};
use core::panic::PanicInfo;

pub mod cloexec;

#[macro_export]
macro_rules! call {
    ($nr:expr, $a:expr, $b:expr, $c:expr, $d:expr) => {
        $crate::call!($nr, $a, $b, $c, $d, 0, 0)
    };
    ($nr:expr, $a:expr, $b:expr, $c:expr, $d:expr, $e:expr, $f:expr) => {
        unsafe {
            $crate::syscall($nr, [
                ($a) as usize, ($b) as usize, ($c) as usize,
                ($d) as usize, ($e) as usize, ($f) as usize,
            ])
        }
    };
}

#[macro_export]
macro_rules! entry {
    () => {
        core::arch::global_asm!(
            ".section .text._start,\"ax\"",
            ".global _start",
            "_start:",
            "mov rsi, rdx",
            "mov rdi, rsp",
            "and rsp, -16",
            "call guest_main",
            "ud2",
        );
    };
}

/// # Safety
/// Pointer arguments must satisfy the syscall's contract, except when explicitly
/// testing invalid pointers that the kernel must reject.
#[inline]
pub unsafe fn syscall(number: usize, args: [usize; 6]) -> isize {
    let result: isize;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") number as isize => result,
            in("rdi") args[0], in("rsi") args[1], in("rdx") args[2],
            in("r10") args[3], in("r8") args[4], in("r9") args[5],
            lateout("rcx") _, lateout("r11") _,
            options(nostack),
        );
    }
    result
}

pub fn say(text: &str) {
    let mut bytes = text.as_bytes();
    while !bytes.is_empty() {
        let written = unsafe { syscall(1, [1, bytes.as_ptr() as usize, bytes.len(), 0, 0, 0]) };
        if written <= 0 {
            exit(1);
        }
        bytes = &bytes[written as usize..];
    }
}

pub fn exit(status: usize) -> ! {
    unsafe {
        syscall(60, [status, 0, 0, 0, 0, 0]);
    }
    loop {
        core::hint::spin_loop();
    }
}

struct Console;
impl Write for Console {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        say(text);
        Ok(())
    }
}

#[panic_handler]
fn panic(info: &PanicInfo<'_>) -> ! {
    let _ = writeln!(Console, "KRUST_GUEST_FAIL {info}");
    exit(1)
}

/// # Safety
/// `stack` must be the initial ELF stack supplied by the kernel.
pub unsafe fn argument(stack: *const usize, index: usize) -> &'static [u8] {
    unsafe {
        assert!(index < stack.read());
        core::ffi::CStr::from_ptr(stack.add(index + 1).read() as *const i8).to_bytes()
    }
}

// Volatile loops prevent the compiler from lowering these implementations back
// into calls to the very runtime intrinsics they provide.
#[unsafe(no_mangle)]
unsafe extern "C" fn memcpy(dst: *mut u8, src: *const u8, len: usize) -> *mut u8 {
    for i in 0..len {
        unsafe {
            dst.add(i).write_volatile(src.add(i).read_volatile());
        }
    }
    dst
}

#[unsafe(no_mangle)]
unsafe extern "C" fn memmove(dst: *mut u8, src: *const u8, len: usize) -> *mut u8 {
    if (dst as usize) <= src as usize {
        unsafe { memcpy(dst, src, len) }
    } else {
        for i in (0..len).rev() {
            unsafe {
                dst.add(i).write_volatile(src.add(i).read_volatile());
            }
        }
        dst
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn memset(dst: *mut u8, value: i32, len: usize) -> *mut u8 {
    for i in 0..len {
        unsafe {
            dst.add(i).write_volatile(value as u8);
        }
    }
    dst
}

#[unsafe(no_mangle)]
unsafe extern "C" fn memcmp(a: *const u8, b: *const u8, len: usize) -> i32 {
    for i in 0..len {
        let (a, b) = unsafe { (a.add(i).read_volatile(), b.add(i).read_volatile()) };
        if a != b {
            return i32::from(a) - i32::from(b);
        }
    }
    0
}

#[unsafe(no_mangle)]
unsafe extern "C" fn bcmp(a: *const u8, b: *const u8, len: usize) -> i32 {
    unsafe { memcmp(a, b, len) }
}
