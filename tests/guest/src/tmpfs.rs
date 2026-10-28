#![no_std]
#![no_main]

use core::ffi::CStr;
use core::ptr::addr_of_mut;
use krust_guest::{call, exit, say};

krust_guest::entry!();
const AT_FDCWD: isize = -100;

#[derive(Default)]
#[repr(C)]
struct Stat {
    dev: usize,
    ino: usize,
    nlink: usize,
    mode: u32,
    uid: u32,
    gid: u32,
    pad: u32,
    rdev: usize,
    size: usize,
    blksize: usize,
    blocks: usize,
    times: [usize; 6],
    unused: [usize; 3],
}
#[derive(Default)]
#[repr(C)]
struct StatFs {
    kind: usize,
    bsize: usize,
    blocks: usize,
    bfree: usize,
    bavail: usize,
    files: usize,
    ffree: usize,
    fsid: [i32; 2],
    namelen: usize,
    frsize: usize,
    flags: usize,
    spare: [usize; 4],
}
fn open(path: &CStr, flags: usize) -> isize {
    call!(2, path.as_ptr(), flags, 0o600, 0)
}
fn space(path: &CStr, blocks: usize, free_blocks: usize, free_files: usize) {
    let mut stats = StatFs::default();
    assert_eq!(call!(137, path.as_ptr(), addr_of_mut!(stats), 0, 0), 0);
    assert_eq!((stats.kind, stats.bsize), (0x0102_1994, 4096));
    assert_eq!(
        (stats.blocks, stats.bfree, stats.bavail),
        (blocks, free_blocks, free_blocks)
    );
    assert_eq!((stats.ffree, stats.namelen), (free_files, 255));
}

#[unsafe(no_mangle)]
pub extern "C" fn guest_main(_: *const usize) -> ! {
    let mut st = Stat::default();
    let mut stats = StatFs::default();
    let mut data = [0u8; 8193];
    assert_eq!(call!(4, c"/tmp".as_ptr(), addr_of_mut!(st), 0, 0), 0);
    assert_eq!((st.mode & 0o7777, st.nlink), (0o1777, 2));
    space(c"/tmp", 2, 2, 7);
    space(c"/run", 1, 1, 1);
    space(c"/parent", 1, 1, 2);
    space(c"/parent/child", 1, 1, 1);
    assert_eq!(call!(137, c"/tmp".as_ptr(), 0, 0, 0), -14);
    assert_eq!(call!(76, 1, 0, 0, 0), -14);
    assert_eq!(call!(76, c"/tmp/missing".as_ptr(), usize::MAX, 0, 0), -22);

    let fd = open(c"/tmp/file", 0x42);
    assert!(fd >= 3);
    assert_eq!(open(c"/tmp/file", 0xC2), -17);
    assert_eq!(call!(138, fd, addr_of_mut!(stats), 0, 0), 0);
    assert_eq!(stats.ffree, 6);
    assert_eq!(call!(91, fd, 0o640, 0, 0), 0);
    assert_eq!(call!(93, fd, 12, 34, 0), 0);
    assert_eq!(call!(5, fd, addr_of_mut!(st), 0, 0), 0);
    assert_eq!((st.mode & 0o7777, st.uid, st.gid), (0o640, 12, 34));
    assert_eq!(call!(8, fd, 1usize << 30, 0, 0), 1isize << 30);
    assert_eq!(call!(1, fd, b"x".as_ptr(), 1, 0), 1);
    assert_eq!(call!(5, fd, addr_of_mut!(st), 0, 0), 0);
    assert_eq!((st.size, st.blocks), ((1 << 30) + 1, 8));
    space(c"/tmp", 2, 1, 6);
    assert_eq!(call!(8, fd, (1usize << 30) - 1, 0, 0), (1isize << 30) - 1);
    assert_eq!(call!(0, fd, data.as_mut_ptr(), 2, 0), 2);
    assert_eq!(&data[..2], b"\0x");
    assert_eq!(call!(77, fd, 0, 0, 0), 0);
    data.fill(0x7F);
    assert_eq!(call!(8, fd, 0, 0, 0), 0);
    assert_eq!(call!(1, fd, data.as_ptr(), data.len(), 0), 8192);
    assert_eq!(call!(1, fd, data.as_ptr(), 1, 0), -28);
    space(c"/tmp", 2, 0, 6);
    assert_eq!(call!(77, fd, 3, 0, 0), 0);
    assert_eq!(call!(77, fd, 8192, 0, 0), 0);
    assert_eq!(call!(8, fd, 0, 0, 0), 0);
    assert_eq!(call!(0, fd, data.as_mut_ptr(), 8, 0), 8);
    assert_eq!((data[0], data[2], data[3], data[7]), (0x7F, 0x7F, 0, 0));
    space(c"/tmp", 2, 1, 6);
    space(c"/run", 1, 1, 1);
    assert_eq!(call!(87, c"/tmp/file".as_ptr(), 0, 0, 0), 0);
    assert_eq!(open(c"/tmp/file", 0), -2);
    assert_eq!(call!(5, fd, addr_of_mut!(st), 0, 0), 0);
    assert_eq!(st.nlink, 0);
    assert_eq!(call!(8, fd, 0, 0, 0), 0);
    assert_eq!(call!(0, fd, data.as_mut_ptr(), 1, 0), 1);
    assert_eq!(data[0], 0x7F);
    assert_eq!(call!(3, fd, 0, 0, 0), 0);
    space(c"/tmp", 2, 2, 7);

    let fd = call!(257, AT_FDCWD, c"/tiny/file".as_ptr(), 0x42, 0o600);
    assert!(fd >= 3);
    assert_eq!(open(c"/tiny/other", 0x42), -28);
    assert_eq!(call!(263, AT_FDCWD, c"/tiny/file".as_ptr(), 1, 0), -22);
    assert_eq!(call!(263, AT_FDCWD, c"/tiny/file".as_ptr(), 0, 0), 0);
    assert_eq!(open(c"/tiny/other", 0x42), -28);
    assert_eq!(call!(3, fd, 0, 0, 0), 0);
    let fd = open(c"/tiny/other", 0x42);
    assert!(fd >= 3);
    assert_eq!(call!(263, 9999, c"/tiny/other".as_ptr(), 0, 0), 0);
    assert_eq!(call!(3, fd, 0, 0, 0), 0);
    assert_eq!(call!(263, 9999, c"relative".as_ptr(), 0, 0), -95);
    space(c"/tiny", 1, 1, 1);

    assert_eq!(call!(83, c"/tmp/a".as_ptr(), 0o700, 0, 0), 0);
    assert_eq!(call!(258, AT_FDCWD, c"/tmp/b".as_ptr(), 0o750, 0), 0);
    assert_eq!(call!(83, c"/tmp/a/child".as_ptr(), 0o700, 0, 0), 0);
    assert_eq!(
        call!(82, c"/tmp/a".as_ptr(), c"/tmp/a/child/loop".as_ptr(), 0, 0),
        -22
    );
    assert_eq!(call!(87, c"/tmp/a".as_ptr(), 0, 0, 0), -21);
    assert_eq!(call!(84, c"/tmp/a".as_ptr(), 0, 0, 0), -39);
    assert_eq!(
        call!(
            264,
            AT_FDCWD,
            c"/tmp/a/child".as_ptr(),
            AT_FDCWD,
            c"/tmp/b/child".as_ptr()
        ),
        0
    );
    assert_eq!(call!(84, c"/tmp/a".as_ptr(), 0, 0, 0), 0);
    let fd = open(c"/tmp/b/file", 0x42);
    assert!(fd >= 3);
    assert_eq!(call!(1, fd, b"keep".as_ptr(), 4, 0), 4);
    let appended = open(c"/tmp/b/file", 0x402);
    assert!(appended >= 3);
    assert_eq!(call!(72, appended, 3, 0, 0), 0x402);
    assert_eq!(call!(8, appended, 0, 0, 0), 0);
    assert_eq!(call!(1, appended, b"more".as_ptr(), 4, 0), 4);
    assert_eq!(call!(5, appended, addr_of_mut!(st), 0, 0), 0);
    assert_eq!(st.size, 8);
    assert_eq!(call!(72, appended, 4, 0, 0), 0);
    assert_eq!(call!(72, appended, 3, 0, 0), 2);
    assert_eq!(call!(3, appended, 0, 0, 0), 0);
    assert_eq!(
        call!(82, c"/tmp/b/file".as_ptr(), c"/run/file".as_ptr(), 0, 0),
        -18
    );
    assert_eq!(
        call!(
            82,
            c"/tmp/b/file".as_ptr(),
            c"/tmp/b/renamed".as_ptr(),
            0,
            0
        ),
        0
    );
    assert_eq!(call!(90, c"/tmp/b/renamed".as_ptr(), 0o600, 0, 0), 0);
    let other = open(c"/tmp/b/renamed", 0x202);
    assert!(other >= 3);
    assert_eq!(call!(5, fd, addr_of_mut!(st), 0, 0), 0);
    assert_eq!(st.size, 0);
    assert_eq!(call!(3, other, 0, 0, 0), 0);
    assert_eq!(call!(87, c"/tmp/b/renamed".as_ptr(), 0, 0, 0), 0);
    assert_eq!(call!(3, fd, 0, 0, 0), 0);
    assert_eq!(call!(263, AT_FDCWD, c"/tmp/b/child".as_ptr(), 0x200, 0), 0);
    assert_eq!(call!(84, c"/tmp/b".as_ptr(), 0, 0, 0), 0);
    assert_eq!(call!(84, c"/tmp".as_ptr(), 0, 0, 0), -16);
    assert_eq!(call!(84, c"/parent".as_ptr(), 0, 0, 0), -16);
    space(c"/tmp", 2, 2, 7);

    let fd = open(c"/tmp/reuse", 0x42);
    assert!(fd >= 3);
    for _ in 0..2048 {
        assert_eq!(call!(8, fd, 0, 0, 0), 0);
        assert_eq!(call!(1, fd, data.as_ptr(), 4096, 0), 4096);
        assert_eq!(call!(77, fd, 0, 0, 0), 0);
    }
    assert_eq!(call!(87, c"/tmp/reuse".as_ptr(), 0, 0, 0), 0);
    assert_eq!(call!(3, fd, 0, 0, 0), 0);
    space(c"/tmp", 2, 2, 7);

    let fd = open(c"/large/file", 0x42);
    assert!(fd >= 3);
    for i in 0..768 {
        data[0] = i as u8;
        assert_eq!(call!(1, fd, data.as_ptr(), 4096, 0), 4096);
    }
    assert_eq!(call!(5, fd, addr_of_mut!(st), 0, 0), 0);
    assert_eq!(st.size, 3 * 1024 * 1024);
    space(c"/large", 1024, 256, 0);
    assert_eq!(call!(8, fd, 767 * 4096, 0, 0), 767 * 4096);
    assert_eq!(call!(0, fd, data.as_mut_ptr(), 1, 0), 1);
    assert_eq!(data[0], 255);
    assert_eq!(call!(76, c"/large/file".as_ptr(), 0, 0, 0), 0);
    assert_eq!(call!(87, c"/large/file".as_ptr(), 0, 0, 0), 0);
    assert_eq!(call!(3, fd, 0, 0, 0), 0);
    space(c"/large", 1024, 1024, 1);
    say("KRUST_TMPFS_OK\n");
    exit(0)
}
