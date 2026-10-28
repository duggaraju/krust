#![feature(allocator_api)]
#![allow(dead_code)]
#![allow(unexpected_cfgs)]

extern crate alloc;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

std::thread_local! {
    static ALLOCATIONS_BEFORE_FAILURE: Cell<Option<usize>> = const { Cell::new(None) };
}
struct FaultAllocator;
fn fail_allocation() -> bool {
    ALLOCATIONS_BEFORE_FAILURE
        .try_with(|left| match left.get() {
            Some(0) => true,
            Some(count) => {
                left.set(Some(count - 1));
                false
            }
            None => false,
        })
        .unwrap_or(false)
}
unsafe impl GlobalAlloc for FaultAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if fail_allocation() {
            core::ptr::null_mut()
        } else {
            unsafe { System.alloc(layout) }
        }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if fail_allocation() {
            core::ptr::null_mut()
        } else {
            unsafe { System.alloc_zeroed(layout) }
        }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if fail_allocation() {
            core::ptr::null_mut()
        } else {
            unsafe { System.realloc(pointer, layout, size) }
        }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: FaultAllocator = FaultAllocator;

fn fail_after<T>(count: usize, operation: impl FnOnce() -> T) -> T {
    ALLOCATIONS_BEFORE_FAILURE.with(|left| left.set(Some(count)));
    let result = operation();
    ALLOCATIONS_BEFORE_FAILURE.with(|left| left.set(None));
    result
}

#[path = "../boot_settings.rs"]
mod boot_settings;
#[path = "../kernel/src/fs/memfs.rs"]
pub mod memfs;
#[path = "../kernel/src/fs/ramfs.rs"]
pub mod ramfs;
#[path = "../kernel/src/fs/vfs.rs"]
pub mod vfs;

mod fs {
    pub use super::vfs;

    pub fn filesystem(_: &str) -> Result<alloc::sync::Arc<dyn vfs::FileSystem>, vfs::FsError> {
        Err(vfs::FsError::NotFound)
    }
}

mod mm {
    pub mod page {
        use core::cell::Cell;

        pub const PAGE_SIZE: usize = 4096;
        std::thread_local! {
            pub static ALLOCATIONS_LEFT: Cell<Option<usize>> = const { Cell::new(None) };
        }
        pub struct OwnedPage(Box<[u8; PAGE_SIZE]>);
        impl OwnedPage {
            pub fn new() -> Option<Self> {
                if !ALLOCATIONS_LEFT.with(|left| match left.get() {
                    Some(0) => false,
                    Some(count) => {
                        left.set(Some(count - 1));
                        true
                    }
                    None => true,
                }) {
                    return None;
                }
                Some(Self(Box::try_new([0; PAGE_SIZE]).ok()?))
            }
            pub fn bytes(&self) -> &[u8; PAGE_SIZE] {
                &self.0
            }
            pub fn bytes_mut(&mut self) -> &mut [u8; PAGE_SIZE] {
                &mut self.0
            }
        }
    }
}

use boot_settings::TmpfsOptions;
use memfs::MemoryFs;
use vfs::{DirCursor, File, FileSystem, FileType, FsError, SetMetadata};

fn tmpfs(pages: usize, inodes: usize) -> MemoryFs {
    MemoryFs::tmpfs(TmpfsOptions {
        size: pages * 4096,
        nr_inodes: inodes,
        ..Default::default()
    })
    .unwrap()
}

#[test]
fn sparse_holes_and_exact_page_accounting() {
    let fs = tmpfs(2, 8);
    let file = fs.root_inode().create("file", FileType::Regular).unwrap();
    assert_eq!(file.write(1 << 30, b"x"), Ok(1));
    assert_eq!(file.size(), (1 << 30) + 1);
    assert_eq!(file.blocks(), 8);
    assert_eq!(fs.statfs().unwrap().blocks_free, 1);
    let mut buffer = [1; 16];
    assert_eq!(file.read((1 << 30) - 8, &mut buffer), Ok(9));
    assert_eq!(&buffer[..9], b"\0\0\0\0\0\0\0\0x");
    assert_eq!(file.read(0, &mut buffer), Ok(16));
    assert_eq!(buffer, [0; 16]);
    assert_eq!(file.read(file.size(), &mut buffer), Ok(0));
    assert_eq!(file.write(0, b"y"), Ok(1));
    assert_eq!(file.write(4096, b"z"), Err(FsError::NoSpace));
    assert_eq!(file.write(1 << 30, b"z"), Ok(1));
    assert_eq!(fs.statfs().unwrap().blocks_free, 0);
    assert_eq!(file.write(usize::MAX, b""), Ok(0));
    assert_eq!(file.write(usize::MAX, b"x"), Err(FsError::FileTooLarge));
}

#[test]
fn partial_writes_stop_at_capacity_and_preserve_sizes() {
    let fs = tmpfs(1, 4);
    let file = fs.root_inode().create("file", FileType::Regular).unwrap();
    assert_eq!(file.write(4090, &[7; 16]), Ok(6));
    assert_eq!(file.size(), 4096);
    assert_eq!(file.blocks(), 8);
    assert_eq!(file.write(4096, b"x"), Err(FsError::NoSpace));
    assert_eq!(file.size(), 4096);
}

#[test]
fn allocation_failure_rolls_back_charge_and_preserves_partial_progress() {
    let fs = tmpfs(3, 4);
    let file = fs.root_inode().create("file", FileType::Regular).unwrap();
    mm::page::ALLOCATIONS_LEFT.with(|left| left.set(Some(0)));
    assert_eq!(file.write(0, b"x"), Err(FsError::OutOfMemory));
    assert_eq!(file.size(), 0);
    assert_eq!(fs.statfs().unwrap().blocks_free, 3);
    mm::page::ALLOCATIONS_LEFT.with(|left| left.set(Some(1)));
    assert_eq!(file.write(0, &[1; 8192]), Ok(4096));
    assert_eq!(file.size(), 4096);
    assert_eq!(fs.statfs().unwrap().blocks_free, 2);
    mm::page::ALLOCATIONS_LEFT.with(|left| left.set(None));
    assert_eq!(file.write(4096, b"x"), Ok(1));
}

#[test]
fn metadata_allocation_failures_leave_no_inodes_or_pages_charged() {
    let mut successful_creates = 0;
    for allocation in 0..8 {
        let fs = tmpfs(2, 4);
        let root = fs.root_inode();
        let before = fs.statfs().unwrap();
        let result = fail_after(allocation, || root.create("file", FileType::Regular));
        match result {
            Ok(file) => {
                successful_creates += 1;
                let result = fail_after(0, || file.write(0, b"x"));
                assert_eq!(result, Err(FsError::OutOfMemory));
                assert_eq!(file.size(), 0);
                assert_eq!(fs.statfs().unwrap().blocks_free, before.blocks_free);
                root.unlink("file", false).unwrap();
                drop(file);
            }
            Err(error) => {
                assert_eq!(error, FsError::OutOfMemory);
                assert!(matches!(root.lookup("file"), Err(FsError::NotFound)));
            }
        }
        assert_eq!(fs.statfs().unwrap(), before);
    }
    assert!(successful_creates > 0);
    for allocation in 0..3 {
        assert!(matches!(
            fail_after(allocation, || MemoryFs::tmpfs(TmpfsOptions::default())),
            Err(FsError::OutOfMemory)
        ));
    }
}

#[test]
fn append_is_atomic_across_separate_writers() {
    let fs = tmpfs(2, 4);
    let file = fs.root_inode().create("file", FileType::Regular).unwrap();
    std::thread::scope(|scope| {
        for value in 0..8u8 {
            let file = &file;
            scope.spawn(move || {
                for _ in 0..100 {
                    assert_eq!(file.append(&[value; 4]).unwrap().1, 4);
                }
            });
        }
    });
    assert_eq!(file.size(), 3200);
    let mut bytes = vec![0; 3200];
    file.read(0, &mut bytes).unwrap();
    let mut counts = [0; 8];
    for chunk in bytes.chunks_exact(4) {
        assert!(chunk.iter().all(|b| *b == chunk[0]));
        counts[chunk[0] as usize] += 1;
    }
    assert_eq!(counts, [100; 8]);
}
#[test]
fn truncate_reclaims_and_zeroes_tail_without_allocating_holes() {
    let fs = tmpfs(2, 4);
    let file = fs.root_inode().create("file", FileType::Regular).unwrap();
    assert_eq!(file.write(0, &[0xFF; 8192]), Ok(8192));
    file.truncate(3).unwrap();
    assert_eq!(fs.statfs().unwrap().blocks_free, 1);
    file.truncate(1 << 30).unwrap();
    assert_eq!(file.blocks(), 8);
    let mut buffer = [1; 8];
    file.read(0, &mut buffer).unwrap();
    assert_eq!(buffer, [255, 255, 255, 0, 0, 0, 0, 0]);
    file.truncate(0).unwrap();
    assert_eq!(fs.statfs().unwrap().blocks_free, 2);
    assert_eq!(file.size(), 0);
    assert_eq!(file.truncate(usize::MAX), Err(FsError::FileTooLarge));
}

#[test]
fn inode_limit_includes_root_and_open_unlinked_files() {
    let fs = tmpfs(2, 2);
    let root = fs.root_inode();
    let file = root.create("file", FileType::Regular).unwrap();
    file.write(0, b"alive").unwrap();
    assert!(matches!(
        root.create("other", FileType::Regular),
        Err(FsError::NoSpace)
    ));
    root.unlink("file", false).unwrap();
    assert!(matches!(root.lookup("file"), Err(FsError::NotFound)));
    assert_eq!(file.links(), 0);
    let mut buffer = [0; 5];
    assert_eq!(file.read(0, &mut buffer), Ok(5));
    assert_eq!(&buffer, b"alive");
    assert_eq!(fs.statfs().unwrap().files_free, 0);
    assert_eq!(fs.statfs().unwrap().blocks_free, 1);
    drop(file);
    assert_eq!(fs.statfs().unwrap().files_free, 1);
    assert_eq!(fs.statfs().unwrap().blocks_free, 2);
    root.create("other", FileType::Regular).unwrap();
}

#[test]
fn directory_removal_types_and_lifetimes() {
    let fs = tmpfs(1, 8);
    let root = fs.root_inode();
    let dir = root.create("dir", FileType::Directory).unwrap();
    assert_eq!(root.links(), 3);
    dir.create("child", FileType::Regular).unwrap();
    assert_eq!(root.unlink("dir", false), Err(FsError::IsADirectory));
    assert_eq!(root.unlink("dir", true), Err(FsError::NotEmpty));
    assert_eq!(dir.unlink("child", true), Err(FsError::NotADirectory));
    dir.unlink("child", false).unwrap();
    root.unlink("dir", true).unwrap();
    assert_eq!(root.links(), 2);
    assert_eq!(dir.links(), 0);
    assert!(matches!(
        dir.create("new", FileType::Regular),
        Err(FsError::NotFound)
    ));
}

#[test]
fn rename_cross_directory_replacement_cycles_and_open_target() {
    let fs = tmpfs(4, 16);
    let root = fs.root_inode();
    let dir = root.create("dir", FileType::Directory).unwrap();
    let child = dir.create("child", FileType::Directory).unwrap();
    assert_eq!(
        root.rename("dir", child.as_ref(), "loop"),
        Err(FsError::InvalidArgument)
    );
    assert_eq!(
        dir.rename("child", child.as_ref(), "loop"),
        Err(FsError::InvalidArgument)
    );
    let source = root.create("source", FileType::Regular).unwrap();
    source.write(0, b"source").unwrap();
    let target = dir.create("target", FileType::Regular).unwrap();
    target.write(0, b"target").unwrap();
    root.rename("source", dir.as_ref(), "target").unwrap();
    assert_eq!(dir.lookup("target").unwrap().ino(), source.ino());
    assert_eq!(target.links(), 0);
    let mut buffer = [0; 6];
    target.read(0, &mut buffer).unwrap();
    assert_eq!(&buffer, b"target");
    dir.rename("target", dir.as_ref(), "renamed").unwrap();
    dir.rename("renamed", dir.as_ref(), "renamed").unwrap();
    assert!(matches!(dir.lookup("target"), Err(FsError::NotFound)));
    let other = tmpfs(1, 4);
    assert_eq!(
        dir.rename("renamed", other.root_inode().as_ref(), "x"),
        Err(FsError::CrossDevice)
    );
    assert_eq!(
        dir.rename("renamed", dir.as_ref(), "child"),
        Err(FsError::IsADirectory)
    );
}

#[test]
fn directory_rename_replacement_tracks_parent_links() {
    let fs = tmpfs(1, 16);
    let root = fs.root_inode();
    let a = root.create("a", FileType::Directory).unwrap();
    let b = root.create("b", FileType::Directory).unwrap();
    let moved = a.create("moved", FileType::Directory).unwrap();
    let replaced = b.create("replaced", FileType::Directory).unwrap();
    a.rename("moved", b.as_ref(), "replaced").unwrap();
    assert_eq!((a.links(), b.links()), (2, 3));
    assert_eq!(replaced.links(), 0);
    assert_eq!(b.lookup("replaced").unwrap().ino(), moved.ino());
    assert_eq!(
        root.rename("b", moved.as_ref(), "cycle"),
        Err(FsError::InvalidArgument)
    );
    b.create("nonempty", FileType::Directory)
        .unwrap()
        .create("file", FileType::Regular)
        .unwrap();
    assert_eq!(
        b.rename("replaced", b.as_ref(), "nonempty"),
        Err(FsError::NotEmpty)
    );
}

#[test]
fn metadata_and_streamed_directory_entries_agree() {
    let fs = MemoryFs::tmpfs(TmpfsOptions {
        mode: 0o1777,
        uid: 12,
        gid: 34,
        ..Default::default()
    })
    .unwrap();
    let root = fs.root_inode();
    assert_eq!((root.mode(), root.uid(), root.gid()), (0o1777, 12, 34));
    let file = root.create("file", FileType::Regular).unwrap();
    file.set_metadata(SetMetadata {
        uid: Some(56),
        gid: Some(78),
        mode: Some(0o640),
    })
    .unwrap();
    let mut cursor = DirCursor::default();
    assert_eq!(
        root.readdir(&mut cursor, &mut [], &mut |entry| {
            assert_eq!(
                (entry.name, entry.uid, entry.gid, entry.mode),
                ("file", 56, 78, 0o640)
            );
            false
        }),
        Ok(1)
    );
    assert_eq!(
        root.readdir(&mut cursor, &mut [], &mut |_| panic!("past end")),
        Ok(0)
    );
    for name in ["", ".", "..", "a/b", "a\0b"] {
        assert!(matches!(
            root.create(name, FileType::Regular),
            Err(FsError::InvalidArgument)
        ));
    }
    assert!(matches!(
        root.create(&"x".repeat(256), FileType::Regular),
        Err(FsError::NameTooLong)
    ));
    assert_eq!(root.write(0, b"x"), Err(FsError::IsADirectory));
    assert_eq!(root.truncate(0), Err(FsError::IsADirectory));
}

#[test]
fn mounts_have_independent_namespaces_and_quotas() {
    let a = tmpfs(1, 2);
    let b = tmpfs(1, 2);
    assert_ne!(a.fs_id(), b.fs_id());
    a.root_inode()
        .create("file", FileType::Regular)
        .unwrap()
        .write(0, b"a")
        .unwrap();
    assert!(matches!(
        b.root_inode().lookup("file"),
        Err(FsError::NotFound)
    ));
    assert_eq!(b.statfs().unwrap().blocks_free, 1);
    b.root_inode()
        .create("file", FileType::Regular)
        .unwrap()
        .write(0, b"b")
        .unwrap();
}

#[test]
fn concurrent_writes_and_creates_cannot_exceed_mount_limits() {
    let fs = tmpfs(3, 5);
    let root = fs.root_inode();
    let files: Vec<_> = (0..4)
        .map(|i| root.create(&i.to_string(), FileType::Regular).unwrap())
        .collect();
    std::thread::scope(|scope| {
        for file in &files {
            scope.spawn(move || {
                for page in 0..4 {
                    match file.write(page * 4096, b"x") {
                        Ok(1) | Err(FsError::NoSpace) => {}
                        result => panic!("unexpected write result {result:?}"),
                    }
                }
            });
        }
    });
    assert_eq!(fs.statfs().unwrap().blocks_free, 0);
    assert_eq!(files.iter().map(|file| file.blocks()).sum::<u64>(), 24);
    for file in files {
        file.truncate(0).unwrap();
    }
    for i in 0..4 {
        root.unlink(&i.to_string(), false).unwrap();
    }
    std::thread::scope(|scope| {
        for i in 0..16 {
            let root = &root;
            scope.spawn(
                move || match root.create(&i.to_string(), FileType::Regular) {
                    Ok(_) | Err(FsError::NoSpace) => {}
                    Err(error) => panic!("{error:?}"),
                },
            );
        }
    });
    assert_eq!(fs.statfs().unwrap().files_free, 0);
}

#[test]
fn ramfs_keeps_unbounded_contiguous_file_semantics_with_shared_namespace() {
    let fs = ramfs::RamFs::new();
    assert_eq!(fs.name(), "ramfs");
    assert_eq!(fs.statfs(), Err(FsError::NotSupported));
    let root = fs.root_inode();
    let file = root.create("file", FileType::Regular).unwrap();
    assert_eq!(file.filesystem_name(), "ramfs");
    file.write(8192, b"x").unwrap();
    assert_eq!(file.size(), 8193);
    assert_eq!(file.blocks(), 17);
    let mut buffer = [1; 4];
    file.read(0, &mut buffer).unwrap();
    assert_eq!(buffer, [0; 4]);
    file.truncate(1).unwrap();
    root.rename("file", root.as_ref(), "renamed").unwrap();
    root.unlink("renamed", false).unwrap();
}

#[test]
fn vfs_open_unlinked_inode_releases_tuple_and_storage_after_last_close() {
    let fs = alloc::sync::Arc::new(tmpfs(2, 4));
    vfs::mount_root(fs.clone()).unwrap();
    let root = fs.root_inode();
    let inode = root.create("file", FileType::Regular).unwrap();
    inode.write(0, b"alive").unwrap();
    let ino = inode.ino();
    let mut open = vfs::open_path("/file").unwrap();
    let descriptor = open.descriptor(vfs::FileOpenMode::ReadWrite);
    let mut alias = vfs::open_by_descriptor(descriptor).unwrap();
    root.unlink("file", false).unwrap();
    drop(inode);
    let mut buffer = [0; 5];
    assert_eq!(open.read(&mut buffer), Ok(5));
    assert_eq!(&buffer, b"alive");
    open.close().unwrap();
    drop(open);
    assert!(vfs::lookup_inode_by_tuple(fs.fs_id(), 0, 0, ino).is_some());
    alias.close().unwrap();
    drop(alias);
    assert!(vfs::lookup_inode_by_tuple(fs.fs_id(), 0, 0, ino).is_none());
    assert_eq!(fs.statfs().unwrap().blocks_free, 2);
    assert_eq!(fs.statfs().unwrap().files_free, 3);
}
