extern crate alloc;

#[path = "../../kernel/src/process/fd_table.rs"]
mod fd_table;

use fd_table::{FD_CLOEXEC, FdTable, O_CLOEXEC};
use std::cell::Cell;
use std::rc::Rc;

#[test]
fn invalid_descriptors_are_ebadf() {
    let mut table = FdTable::<usize>::new();
    for fd in [0, 3, 9999, usize::MAX] {
        assert_eq!(table.flags(fd), Err(-9));
        for flags in [0, 1, usize::MAX] {
            assert_eq!(table.set_flags(fd, flags), Err(-9));
        }
        assert_eq!(table.duplicate(fd, 0, true), Err(-9));
    }
    assert_eq!(table.duplicate(3, usize::MAX, true), Err(-9));
}

#[test]
fn flags_are_local_masked_and_not_open_flags() {
    let mut table = FdTable::new();
    let fd = table.insert(42, FD_CLOEXEC);
    assert_eq!(fd, 0);
    assert_eq!(table.flags(fd), Ok(1));
    table.set_flags(fd, usize::MAX).unwrap();
    assert_eq!(table.flags(fd), Ok(1));
    table.set_flags(fd, O_CLOEXEC).unwrap();
    assert_eq!(table.flags(fd), Ok(0));
    assert_eq!(table.get(fd), Some(&42));
}

#[test]
fn duplicate_clears_or_sets_flags_but_shares_description() {
    let mut table = FdTable::new();
    let description = Rc::new(Cell::new(0));
    let fd = table.insert(description.clone(), FD_CLOEXEC);
    let normal = table.duplicate(fd, 4, false).unwrap();
    let flagged = table.duplicate(fd, 4, true).unwrap();
    assert_eq!((normal, flagged), (4, 5));
    assert_eq!(table.flags(normal), Ok(0));
    assert_eq!(table.flags(flagged), Ok(1));
    table.set_flags(fd, 0).unwrap();
    assert_eq!(table.flags(flagged), Ok(1));
    table.get(normal).unwrap().set(123);
    assert_eq!(table.get(flagged).unwrap().get(), 123);
    assert_eq!(description.get(), 123);
    assert_eq!(table.duplicate(fd, usize::MAX, false), Err(-22));
}

#[test]
fn close_and_reuse_clear_flags_and_choose_lowest_available() {
    let mut table = FdTable::new();
    table.insert_at(2, 7, FD_CLOEXEC);
    assert_eq!(table.insert(8, 0), 0);
    assert_eq!(table.duplicate(2, 0, false), Ok(1));
    assert_eq!(table.remove(2), Some(7));
    assert_eq!(table.flags(2), Err(-9));
    assert_eq!(table.set_flags(2, 1), Err(-9));
    assert_eq!(table.insert(9, 0), 2);
    assert_eq!(table.flags(2), Ok(0));
    table.insert_at(2, 10, 0);
    assert_eq!(table.flags(2), Ok(0));
}

#[test]
fn fork_copies_flags_without_sharing_table_and_exec_only_closes_child() {
    let mut parent = FdTable::new();
    let file = Rc::new(Cell::new(0));
    parent.insert_at(3, file.clone(), FD_CLOEXEC);
    parent.insert_at(4, file.clone(), 0);
    let mut child = parent.clone();
    assert_eq!(child.flags(3), Ok(1));
    assert_eq!(child.flags(4), Ok(0));
    child.set_flags(4, 1).unwrap();
    assert_eq!(parent.flags(4), Ok(0));
    child.set_flags(4, 0).unwrap();
    child.get(4).unwrap().set(5);
    assert_eq!(parent.get(4).unwrap().get(), 5);
    let removed = child.close_on_exec();
    assert_eq!(removed.len(), 1);
    assert_eq!(child.flags(3), Err(-9));
    assert_eq!(child.flags(4), Ok(0));
    assert_eq!(parent.flags(3), Ok(1));
    assert_eq!(child.iter().map(|(fd, _)| fd).collect::<Vec<_>>(), vec![4]);
}

#[test]
fn exec_releases_only_flagged_aliases_and_survivor_remains_live() {
    let mut table = FdTable::new();
    let file = Rc::new(());
    table.insert(file.clone(), FD_CLOEXEC);
    table.duplicate(0, 1, false).unwrap();
    assert_eq!(Rc::strong_count(&file), 3);
    drop(table.close_on_exec());
    assert_eq!(Rc::strong_count(&file), 2);
    assert_eq!(table.flags(1), Ok(0));
    drop(table.remove(1));
    assert_eq!(Rc::strong_count(&file), 1);
}
