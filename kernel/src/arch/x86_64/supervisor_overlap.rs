//! Read-only four-level page-table range walk. Missing subtrees are skipped.

use super::user_layout::USER_ADDRESS_END;

const PRESENT: u64 = 1;
const USER: u64 = 1 << 2;
const HUGE: u64 = 1 << 7;
const TABLE_ADDRESS: u64 = 0x000f_ffff_ffff_f000;

pub(super) fn range_overlaps_supervisor(
    root: u64,
    start: usize,
    end: usize,
    mut read_entry: impl FnMut(u64, usize) -> Option<u64>,
) -> Option<bool> {
    if start >= end || end > USER_ADDRESS_END {
        return None;
    }
    walk(root, 4, 0, start, end, true, &mut read_entry)
}

fn walk(
    table: u64,
    level: usize,
    table_start: usize,
    start: usize,
    end: usize,
    parent_user: bool,
    read_entry: &mut impl FnMut(u64, usize) -> Option<u64>,
) -> Option<bool> {
    let span = 1usize << (12 + 9 * (level - 1));
    let first = (start - table_start) / span;
    let last = (end - 1 - table_start) / span;
    for index in first..=last {
        let entry = read_entry(table, index)?;
        if entry & PRESENT == 0 {
            continue;
        }
        let user = parent_user && entry & USER != 0;
        if level == 1 || ((level == 2 || level == 3) && entry & HUGE != 0) {
            if !user {
                return Some(true);
            }
            continue;
        }
        let entry_start = table_start + index * span;
        if walk(
            entry & TABLE_ADDRESS,
            level - 1,
            entry_start,
            start.max(entry_start),
            end.min(entry_start + span),
            user,
            read_entry,
        )? {
            return Some(true);
        }
    }
    Some(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tables() -> [[u64; 512]; 4] {
        let mut tables = [[0; 512]; 4];
        for (level, table) in tables.iter_mut().enumerate().take(3) {
            table[0] = ((level as u64 + 1) << 12) | PRESENT | USER;
        }
        tables[3][1] = 0x10000 | PRESENT | USER;
        tables
    }

    fn overlaps(tables: &[[u64; 512]; 4], start: usize, end: usize) -> Option<bool> {
        range_overlaps_supervisor(0, start, end, |table, index| {
            tables.get(table as usize / 4096)?.get(index).copied()
        })
    }

    #[test]
    fn effective_user_permission_includes_every_parent() {
        let original = tables();
        assert_eq!(overlaps(&original, 4096, 8192), Some(false));
        for level in 0..4 {
            let mut tables = original;
            tables[level][if level == 3 { 1 } else { 0 }] &= !USER;
            assert_eq!(overlaps(&tables, 4096, 8192), Some(true));
            assert_eq!(overlaps(&tables, 0, 4096), Some(false));
            assert_eq!(overlaps(&tables, 0, 8192), Some(true));
            assert_eq!(overlaps(&tables, 8192, 12288), Some(false));
        }
    }

    #[test]
    fn huge_pages_and_parent_permissions_are_checked() {
        for (level, size) in [(1, 1usize << 30), (2, 1usize << 21)] {
            let mut tables = tables();
            tables[level][0] = PRESENT | HUGE;
            assert_eq!(overlaps(&tables, 4096, 8192), Some(true));
            assert_eq!(overlaps(&tables, size, size + 4096), Some(false));
            tables[level][0] |= USER;
            assert_eq!(overlaps(&tables, 4096, 8192), Some(false));
            tables[0][0] &= !USER;
            assert_eq!(overlaps(&tables, 4096, 8192), Some(true));
        }
    }

    #[test]
    fn large_holes_are_skipped_and_read_failures_fail_closed() {
        let mut reads = 0;
        assert_eq!(
            range_overlaps_supervisor(0, 0, USER_ADDRESS_END, |_, _| {
                reads += 1;
                Some(0)
            }),
            Some(false)
        );
        assert_eq!(reads, 256);
        assert_eq!(range_overlaps_supervisor(0, 0, 4096, |_, _| None), None);
        assert_eq!(range_overlaps_supervisor(0, 0, 0, |_, _| Some(0)), None);
    }

    #[test]
    fn existing_user_mappings_are_not_a_collision_policy() {
        let tables = tables();
        let before = tables;
        assert_eq!(overlaps(&tables, 0, 12288), Some(false));
        assert_eq!(tables, before);
    }
}
