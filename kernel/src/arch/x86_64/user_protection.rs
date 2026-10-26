//! mmap protections translated to leaf PTE bits, including an owned PROT_NONE marker.
pub const NO_ACCESS: u64 = 1 << 9;

pub fn mmap_page_flags(prot: usize) -> Result<u64, isize> {
    if prot & !7 != 0 {
        return Err(-22); // EINVAL
    }
    let mut flags = 1 << 2; // USER, including non-present PROT_NONE pages.
    if prot == 0 {
        flags |= NO_ACCESS;
    } else {
        flags |= 1; // PRESENT
    }
    // x86 has no write-only or execute-only PTEs: either also permits reads.
    if prot & 2 != 0 {
        flags |= 1 << 1;
    }
    if prot & 4 == 0 {
        flags |= 1 << 63; // NX
    }
    Ok(flags)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_protection_combinations() {
        for prot in 0..8 {
            let flags = mmap_page_flags(prot).unwrap();
            assert_ne!(flags & 4, 0);
            assert_eq!(flags & 1 != 0, prot != 0);
            assert_eq!(flags & NO_ACCESS != 0, prot == 0);
            assert_eq!(flags & 2 != 0, prot & 2 != 0);
            assert_eq!(flags & (1 << 63) != 0, prot & 4 == 0);
        }
    }

    #[test]
    fn rejects_unknown_bits() {
        for bit in 3..usize::BITS {
            for known in 0..8 {
                assert_eq!(mmap_page_flags(known | (1usize << bit)), Err(-22));
            }
        }
    }
}
