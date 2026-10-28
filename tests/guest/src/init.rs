#![no_std]
#![no_main]

use krust_guest::{argument, call, exit, say};

krust_guest::entry!();

fn cpu_ticks() -> usize {
    let fd = call!(2, c"/proc/1/stat".as_ptr(), 0, 0, 0);
    assert!(fd >= 0);
    let mut bytes = [0; 1024];
    let count = call!(0, fd, bytes.as_mut_ptr(), bytes.len(), 0);
    assert!(count > 0);
    assert_eq!(call!(3, fd, 0, 0, 0), 0);
    let text = core::str::from_utf8(&bytes[..count as usize]).unwrap();
    let fields = text.rsplit_once(") ").unwrap().1;
    fields
        .split_whitespace()
        .skip(11)
        .take(2)
        .map(|field| field.parse::<usize>().unwrap())
        .sum()
}

fn terminal_probe() -> ! {
    for path in [c"/dev/ttyS1", c"/dev/tty0"] {
        let fd = call!(2, path.as_ptr(), 2, 0, 0);
        assert!(fd >= 0);
        let mut original = [0u8; 44];
        assert_eq!(call!(16, fd, 0x802c542a, original.as_mut_ptr(), 0), 0);
        let flags = u32::from_ne_bytes(original[12..16].try_into().unwrap());
        assert_eq!(flags & 10, 10);
        assert_eq!(original[23], 1); // VMIN
        assert_eq!(call!(16, usize::MAX, 0x5401, original.as_mut_ptr(), 0), -9);
        assert_eq!(call!(16, fd, 0x5401, 0, 0), -14);
        assert_eq!(call!(16, fd, 0x5402, 0, 0), -14);
        assert_eq!(call!(16, fd, 0x5401, 0x4000, 0), -14);
        assert_eq!(call!(16, fd, 0x5402, 0x4000, 0), -14);
        static READ_ONLY: [u8; 44] = [0; 44];
        assert_eq!(call!(16, fd, 0x5401, READ_ONLY.as_ptr(), 0), -14);
        let regular = call!(2, c"/bin/ttytest".as_ptr(), 0, 0, 0);
        assert!(regular >= 0);
        assert_eq!(call!(16, regular, 0x5401, original.as_mut_ptr(), 0), -25);
        assert_eq!(call!(3, regular, 0, 0, 0), 0);

        let mut raw = original;
        raw[12..16].copy_from_slice(&(flags & !11).to_ne_bytes());
        for request in [0x5402, 0x5403, 0x5404, 0x402c542b, 0x402c542c, 0x402c542d] {
            assert_eq!(call!(16, fd, request, raw.as_ptr(), 0), 0);
            let mut observed = [0; 44];
            assert_eq!(call!(16, fd, 0x802c542a, observed.as_mut_ptr(), 0), 0);
            assert_eq!(observed, raw);
        }
        let mut legacy = [0xa5; 44];
        assert_eq!(call!(16, fd, 0x5401, legacy.as_mut_ptr(), 0), 0);
        assert_eq!(&legacy[..36], &raw[..36]);
        assert_eq!(&legacy[36..], &[0xa5; 8]);
        let alias = call!(2, path.as_ptr(), 2, 0, 0);
        assert!(alias >= 0);
        let mut observed = [0; 44];
        assert_eq!(call!(16, alias, 0x802c542a, observed.as_mut_ptr(), 0), 0);
        assert_eq!(observed, raw);
        assert_eq!(call!(3, alias, 0, 0, 0), 0);
        let cpu_before = cpu_ticks();
        let start = call!(100, 0, 0, 0, 0);
        say("KRUST_TTY_RAW_READY\n");
        let mut buffer = [0u8; 16];
        assert_eq!(call!(0, fd, buffer.as_mut_ptr(), 1, 0), 1);
        let elapsed = (call!(100, 0, 0, 0, 0) - start) as usize;
        let cpu_elapsed = cpu_ticks() - cpu_before;
        assert_eq!(buffer[0], b'A');
        assert!(elapsed >= 50, "read did not wait for delayed input");
        assert!(cpu_elapsed <= elapsed / 4 + 2, "idle read consumed CPU");
        assert_eq!(call!(16, fd, 0x5403, original.as_ptr(), 0), 0);
        say("KRUST_TTY_COOKED_READY\n");
        assert_eq!(call!(0, fd, buffer.as_mut_ptr(), buffer.len(), 0), 4);
        assert_eq!(&buffer[..4], b"abc\n");
        let mut quiet = original;
        quiet[12..16].copy_from_slice(&(flags & !8).to_ne_bytes());
        assert_eq!(call!(16, fd, 0x5402, quiet.as_ptr(), 0), 0);
        say("KRUST_TTY_QUIET_READY\n");
        assert_eq!(call!(0, fd, buffer.as_mut_ptr(), buffer.len(), 0), 7);
        assert_eq!(&buffer[..7], b"secret\n");
        assert_eq!(call!(16, fd, 0x5404, original.as_ptr(), 0), 0);
        assert_eq!(call!(3, fd, 0, 0, 0), 0);
        say("KRUST_TTY_DEVICE_OK\n");
    }
    call!(500, 0, 0, 0, 0);
    panic!("shutdown returned");
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn guest_main(stack: *const usize, rtld_fini: usize) -> ! {
    assert_eq!(call!(39, 0, 0, 0, 0), 1);
    assert_eq!(call!(110, 0, 0, 0, 0), 0);
    assert_eq!(call!(111, 0, 0, 0, 0), 1);
    let mut status = 0i32;
    assert_eq!(
        call!(61, usize::MAX, core::ptr::addr_of_mut!(status), 1, 0),
        -10
    );
    assert_eq!(rtld_fini, 0);
    let mut byte = 0u8;
    assert_eq!(call!(0, 0, core::ptr::addr_of_mut!(byte), 0, 0), 0);
    let argc = unsafe { stack.read() };
    assert!((1..=2).contains(&argc));
    let name = unsafe { argument(stack, 0) };
    if name == b"/bin/ttytest" {
        terminal_probe();
    }
    if argc == 2 {
        assert_eq!(unsafe { argument(stack, 1) }, b"/sbin/init");
    }
    let mut cwd = [0u8; 16];
    assert_eq!(call!(79, cwd.as_mut_ptr(), cwd.len(), 0, 0), 1);
    assert_eq!(&cwd[..2], b"/\0");
    let mut cursor = unsafe { stack.add(argc + 2) };
    let mut path = false;
    let mut home = false;
    while unsafe { cursor.read() } != 0 {
        let value = unsafe { core::ffi::CStr::from_ptr(cursor.read() as *const i8) }.to_bytes();
        path |= value == b"PATH=/sbin:/bin:/usr/sbin:/usr/bin";
        home |= value == b"HOME=/";
        cursor = unsafe { cursor.add(1) };
    }
    assert!(path && home);
    say("KRUST_INIT_ARGV0=");
    say(core::str::from_utf8(name).unwrap());
    say("\nKRUST_INIT_PID1_OK\n");
    if name == b"/bin/exitinit" {
        exit(42);
    }
    if name == b"/bin/faultini" {
        unsafe {
            core::arch::asm!("mov byte ptr [{address}], 1", address = in(reg) 0usize);
        }
        panic!("init fault returned");
    }
    call!(500, 0, 0, 0, 0);
    panic!("shutdown returned");
}
