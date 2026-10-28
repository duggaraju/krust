#![no_std]
#![no_main]

use core::ptr::{addr_of, addr_of_mut};
use krust_guest::{argument, call, exit, say};

krust_guest::entry!();

const SENTINEL: usize = 0x0123_4567_89ab_cdef;
static mut PROTECTED_SENTINEL: usize = SENTINEL;

fn map(address: usize, len: usize, flags: usize, fd: usize) -> isize {
    call!(9, address, len, 3, flags, fd, 0)
}

struct State {
    next: usize,
    previous: usize,
    boundary: usize,
}

impl State {
    fn healthy(&mut self) {
        let result = map(0, 4096, 0x22, usize::MAX);
        assert!(result > 0);
        assert_eq!(result as usize, self.next);
        unsafe {
            assert_eq!(addr_of!(PROTECTED_SENTINEL).read_volatile(), SENTINEL);
            for pointer in [self.previous, self.boundary] {
                if pointer != 0 {
                    assert_eq!((pointer as *const usize).read_volatile(), SENTINEL);
                }
            }
            self.previous = result as usize;
            let page = self.previous as *mut usize;
            assert_eq!(page.read_volatile(), 0);
            assert_eq!(page.add(511).read_volatile(), 0);
            page.write_volatile(SENTINEL);
        }
        self.next += 4096;
    }

    fn reject(&mut self, address: usize, len: usize, flags: usize, fd: usize, error: isize) {
        assert_eq!(map(address, len, flags, fd), error);
        self.healthy();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn guest_main(stack: *const usize) -> ! {
    let end = 1usize << 47;
    assert_eq!(unsafe { stack.read() }, 2);
    let text = unsafe { argument(stack, 1) };
    assert!(text.starts_with(b"0x") && (3..=14).contains(&text.len()));
    let mut kernel = 0usize;
    for &byte in &text[2..] {
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            _ => panic!("invalid kernel address"),
        };
        kernel = (kernel << 4) | usize::from(digit);
    }
    assert!(kernel > 4096 && kernel < end && kernel & 4095 == 0);
    let mut state = State {
        next: 0x6000_0000_0000,
        previous: 0,
        boundary: 0,
    };
    state.healthy();
    state.reject(kernel, 4096, 0x22, usize::MAX, -12);
    let data_page = addr_of_mut!(PROTECTED_SENTINEL) as usize & !4095;
    assert!(data_page < kernel);
    state.reject(data_page, kernel - data_page + 4096, 0x22, usize::MAX, -12);
    say("KRUST_MMAP_KERNEL_OK\n");
    // This reserved stack-top page is above the live initial stack; collision
    // policy is deliberately outside this preflight regression.
    assert_eq!(
        map(end - 4096, 4096, 0x22, usize::MAX),
        (end - 4096) as isize
    );
    state.boundary = end - 4096;
    unsafe {
        let page = state.boundary as *mut usize;
        assert_eq!(page.read_volatile(), 0);
        assert_eq!(page.add(511).read_volatile(), 0);
        page.write_volatile(SENTINEL);
    }
    state.healthy();
    state.reject(0, 0, 0x22, usize::MAX, -22);
    for (address, len) in [
        (0, usize::MAX),
        (0, usize::MAX - 1),
        (0, usize::MAX - 4094),
        (0, usize::MAX - 4095),
        (0, end),
        (usize::MAX, 4096),
        (usize::MAX - 4095, 8192),
        (end, 4096),
        (end + 4096, 4096),
        (0xffff_8000_0000_0000, 4096),
        (end - 1, 1),
        (end - 4096, 4097),
        (end - 8192, 8193),
        (4096, usize::MAX - 4095),
    ] {
        state.reject(address, len, 0x22, usize::MAX, -12);
    }
    state.reject(0, 0, 0x32, usize::MAX, -12);
    state.reject(0, 4096, 0x2, usize::MAX, -12);
    state.reject(0, 4096, 0x22, 0, -12);
    let hinted = map(0x5000_0000_0001, 4097, 0x22, u32::MAX as usize);
    assert_eq!(hinted, 0x5000_0000_1000);
    unsafe {
        let page = hinted as *mut usize;
        assert_eq!(page.read_volatile(), 0);
        assert_eq!(page.add(1023).read_volatile(), 0);
        page.write_volatile(SENTINEL);
        page.add(1023).write_volatile(SENTINEL);
    }
    state.healthy();
    assert_eq!(map(0, 1, 0x22, usize::MAX), state.next as isize);
    state.next += 4096;
    state.healthy();
    say("KRUST_MMAP_OK\n");
    exit(0)
}
