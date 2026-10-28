use crate::{exit, say, syscall};

fn call(nr: usize, a: usize, b: usize, c: usize, d: usize) -> isize {
    unsafe { syscall(nr, [a, b, c, d, 0, 0]) }
}
fn flags(fd: usize) -> isize {
    call(72, fd, 1, 0, 0)
}
fn setflags(fd: usize, value: usize) -> isize {
    call(72, fd, 2, value, 0)
}
fn duplicate(fd: usize, min: usize, cloexec: bool) -> isize {
    call(72, fd, if cloexec { 1030 } else { 0 }, min, 0)
}
fn open_data(flags: usize) -> isize {
    call(2, c"/bin/krust-cloexec-data".as_ptr() as usize, flags, 0, 0)
}
fn read_byte(fd: usize, expected: u8) {
    let mut byte = 0u8;
    assert_eq!(call(0, fd, (&mut byte as *mut u8) as usize, 1, 0), 1);
    assert_eq!(byte, expected);
}
fn bad_fd(fd: usize) {
    assert_eq!(flags(fd), -9);
    assert_eq!(setflags(fd, 0), -9);
    assert_eq!(setflags(fd, 1), -9);
}

pub fn run(after_exec: bool) -> ! {
    bad_fd(20);
    assert_eq!(flags(21), 0);
    if !after_exec {
        read_byte(21, b'a');
        bad_fd(usize::MAX);
        bad_fd(9999);
        assert_eq!(open_data(0x80000), 3);
        assert_eq!(
            call(
                257,
                (-100isize) as usize,
                c"/bin/krust-cloexec-data".as_ptr() as usize,
                0,
                0
            ),
            4
        );
        assert_eq!((flags(3), flags(4)), (1, 0));
        assert_eq!(call(72, 3, 3, 0, 0), 0);
        assert_eq!(duplicate(3, 5, false), 5);
        assert_eq!(duplicate(3, 6, true), 6);
        assert_eq!((flags(3), flags(5), flags(6)), (1, 0, 1));
        assert_eq!(duplicate(3, usize::MAX, false), -22);
        assert_eq!(duplicate(9999, usize::MAX, false), -9);
        assert_eq!(setflags(4, 0x101), 0);
        assert_eq!(flags(4), 1);
        read_byte(4, b'a');
        assert_eq!(call(8, 4, 0, 0, 0), 0);
        assert_eq!(flags(4), 1);
        assert_eq!(call(72, 4, 3, 0, 0), 0);
        assert_eq!(setflags(4, 0x80000), 0);
        assert_eq!(flags(4), 0);
        assert_eq!(
            call(
                257,
                (-100isize) as usize,
                c"/bin/krust-cloexec-data".as_ptr() as usize,
                0x80000,
                0
            ),
            7
        );
        assert_eq!(flags(7), 1);
        assert_eq!(open_data(0), 8);
        assert_eq!(setflags(8, 1), 0);
        assert_eq!(flags(8), 1);
        assert_eq!(setflags(8, 0), 0);
        assert_eq!(flags(8), 0);
        assert_eq!(call(3, 7, 0, 0, 0), 0);
        bad_fd(7);
        assert_eq!(open_data(0), 7);
        assert_eq!(flags(7), 0);
        assert_eq!(call(3, 7, 0, 0, 0), 0);
        assert_eq!(
            call(59, c"/bin/krust-cloexec-missing".as_ptr() as usize, 0, 0, 0),
            -2
        );
        assert_eq!((flags(3), flags(6), flags(4)), (1, 1, 0));
        assert_eq!(
            call(59, c"/bin/krust-cloexec-data".as_ptr() as usize, 0, 0, 0),
            -8
        );
        assert_eq!((flags(3), flags(6), flags(5)), (1, 1, 0));
        read_byte(3, b'a');
        assert_eq!(flags(3), 1);
        assert_eq!(call(3, 0, 0, 0, 0), 0);
        assert_eq!(duplicate(3, 0, false), 0);
        assert_eq!(flags(0), 0);
        assert_eq!(call(3, 0, 0, 0, 0), 0);
        assert_eq!(open_data(0x80000), 0);
        assert_eq!(flags(0), 1);
        say("KRUST_CLOEXEC_BEFORE_OK\n");
        call(59, c"/bin/krust-cloexec-after".as_ptr() as usize, 0, 0, 0);
        panic!("successful exec returned");
    }
    for fd in [0, 3, 6, 7] {
        bad_fd(fd);
    }
    assert_eq!((flags(4), flags(5), flags(8)), (0, 0, 0));
    read_byte(5, b'b');
    read_byte(21, b'b');
    assert_eq!(setflags(21, 1), 0);
    assert_eq!(flags(21), 1);
    assert_eq!(open_data(0), 0);
    assert_eq!(flags(0), 0);
    assert_eq!(open_data(0), 3);
    assert_eq!(flags(3), 0);
    assert_eq!(setflags(3, 1), 0);
    assert_eq!(call(3, 3, 0, 0, 0), 0);
    assert_eq!(open_data(0), 3);
    assert_eq!(flags(3), 0);
    say("KRUST_CLOEXEC_EXEC_OK\n");
    exit(0)
}
