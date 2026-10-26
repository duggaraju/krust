//! The ARM64 runtime mapper is not implemented; do not claim protection support.
pub fn mmap_page_flags(prot: usize) -> Result<u64, isize> {
    if prot & !7 != 0 {
        Err(-22) // EINVAL
    } else {
        Err(-38) // ENOSYS
    }
}
