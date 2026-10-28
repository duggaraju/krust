#![no_std]
#![no_main]

use core::arch::asm;
use krust_guest::{argument, call, exit, say};

krust_guest::entry!();

fn raw_map(address: usize, prot: usize) -> isize {
    call!(9, address, 4096, prot, 0x22, usize::MAX, 0)
}
fn map(prot: usize) -> *mut u8 {
    let result = raw_map(0, prot);
    assert!(result > 0);
    result as *mut u8
}
unsafe fn execute(code: *const u8) {
    let mut scratch = 0u8;
    // Zero-filled executable-only pages end in ADD [RAX],AL; provide writable
    // storage and let execution reach a RET on the following page.
    unsafe {
        asm!("call {code}", code = in(reg) code,
            inlateout("rax") (&mut scratch as *mut u8) => _,
            clobber_abi("C"));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn guest_main(stack: *const usize) -> ! {
    assert_eq!(unsafe { stack.read() }, 2);
    let mode = unsafe { argument(stack, 1) };
    if mode == b"ok" {
        let mut next = 0x6000_0000_0000usize;
        for prot in 0..8 {
            let page = map(prot);
            assert_eq!(page as usize, next);
            next += 4096;
            unsafe {
                if prot != 0 {
                    assert_eq!(page.read_volatile(), 0);
                    assert_eq!(page.add(4095).read_volatile(), 0);
                }
                if prot & 2 != 0 {
                    page.write_volatile(0xc3);
                    assert_eq!(page.read_volatile(), 0xc3);
                }
                if prot & 4 != 0 {
                    if prot & 2 != 0 {
                        execute(page);
                    } else {
                        let tail = map(7);
                        assert_eq!(tail as usize, next);
                        next += 4096;
                        tail.write_volatile(0xc3);
                        execute(page.add(4094));
                    }
                }
            }
        }
        for bit in 3..64 {
            assert_eq!(raw_map(0, 3 | (1usize << bit)), -22);
        }
        assert_eq!(map(3) as usize, next);
        say("KRUST_PROTECTION_OK\n");
        exit(0);
    }
    let (prot, access) = match mode {
        b"ro-write" => (1, 1),
        b"exec-write" => (4, 1),
        b"rx-write" => (5, 1),
        b"nx-read" => (1, 2),
        b"nx-write" => (2, 2),
        b"nx-rw" => (3, 2),
        b"none-read" => (0, 0),
        b"none-write" => (0, 1),
        b"none-exec" => (0, 2),
        _ => panic!("unknown protection case"),
    };
    let page = map(prot);
    if prot & 2 != 0 {
        unsafe {
            page.write_volatile(0xc3);
        }
    }
    say("KRUST_PROTECTION_ARMED ");
    say(core::str::from_utf8(mode).expect("ASCII protection case"));
    say("\n");
    // Deliberately faulting instructions must not invoke Rust pointer operations
    // whose validity contracts forbid trapping.
    unsafe {
        match access {
            0 => asm!("mov al, byte ptr [{page}]", page = in(reg) page, lateout("al") _),
            1 => asm!("mov byte ptr [{page}], 0xc3", page = in(reg) page),
            _ => execute(page),
        }
    }
    panic!("forbidden access returned")
}
