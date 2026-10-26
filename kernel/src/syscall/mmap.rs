//! Pure preflight arithmetic; no address-space or placement state is changed here.

#[cfg(test)]
#[path = "../arch/x86_64/user_layout.rs"]
mod layout;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct MmapRange {
    pub base: usize,
    pub len: usize,
    pub end: usize,
}

pub(super) fn checked_mmap_range(
    base: usize,
    len: usize,
    page_size: usize,
    user_end: usize,
) -> Result<MmapRange, isize> {
    if len == 0 {
        return Err(-22); // EINVAL
    }
    let align = |value: usize| {
        value
            .checked_add(page_size - 1)
            .map(|v| v & !(page_size - 1))
    };
    let len = align(len).ok_or(-12isize)?;
    let base = align(base).ok_or(-12isize)?;
    let end = base.checked_add(len).ok_or(-12isize)?;
    if base >= user_end || end > user_end {
        return Err(-12); // ENOMEM
    }
    Ok(MmapRange { base, len, end })
}

#[cfg(test)]
mod tests {
    use super::layout::{PAGE_SIZE, USER_ADDRESS_END, USER_STACK_TOP};
    use super::*;

    fn range(base: usize, len: usize) -> Result<MmapRange, isize> {
        checked_mmap_range(base, len, PAGE_SIZE, USER_ADDRESS_END)
    }

    #[test]
    fn rejects_zero_and_length_rounding_overflow() {
        assert_eq!(range(0x6000_0000_0000, 0), Err(-22));
        for len in [usize::MAX, usize::MAX - 1, usize::MAX - PAGE_SIZE + 2] {
            assert_eq!(range(0x6000_0000_0000, len), Err(-12));
        }
        assert_eq!(
            range(0x6000_0000_0000, usize::MAX - PAGE_SIZE + 1),
            Err(-12)
        );
    }

    #[test]
    fn rejects_bad_bases_and_ends() {
        for base in [
            USER_ADDRESS_END,
            USER_ADDRESS_END + PAGE_SIZE,
            0xffff_8000_0000_0000,
            usize::MAX,
            usize::MAX - PAGE_SIZE + 1,
        ] {
            assert_eq!(range(base, PAGE_SIZE), Err(-12));
        }
        assert_eq!(range(USER_STACK_TOP, PAGE_SIZE + 1), Err(-12));
        assert_eq!(range(0x6000_0000_0000, USER_ADDRESS_END), Err(-12));
        assert_eq!(range(USER_ADDRESS_END - 1, 1), Err(-12));
        assert_eq!(range(PAGE_SIZE, usize::MAX - PAGE_SIZE + 1), Err(-12));
    }

    #[test]
    fn accepts_exact_boundary_and_preserves_upward_alignment() {
        assert_eq!(
            range(USER_STACK_TOP, PAGE_SIZE),
            Ok(MmapRange {
                base: USER_STACK_TOP,
                len: PAGE_SIZE,
                end: USER_ADDRESS_END
            })
        );
        assert_eq!(
            range(0x6000_0000_0001, PAGE_SIZE + 1),
            Ok(MmapRange {
                base: 0x6000_0000_1000,
                len: 2 * PAGE_SIZE,
                end: 0x6000_0000_3000,
            })
        );
        assert_eq!(range(PAGE_SIZE, 1).unwrap().len, PAGE_SIZE);
        assert_eq!(range(USER_STACK_TOP - 1, 1).unwrap().end, USER_ADDRESS_END);
    }
}
