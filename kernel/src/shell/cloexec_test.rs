//! Debug-only integration entry point while resumable userspace fork is unavailable.
use super::Console;
use crate::{process, syscall::impls};
use alloc::{format, string::String, vec, vec::Vec};

pub fn run(console: &mut Console) {
    let mut owned = Vec::new();
    let result = (|| -> Result<(), isize> {
        let source = impls::open("/bin/krust-cloexec-data", 0)?;
        owned.push(source);
        let flagged = process::dup_fd(source, 20, true)?;
        owned.push(flagged);
        let normal = process::dup_fd(source, 21, false)?;
        owned.push(normal);
        if flagged != 20 || normal != 21 {
            return Err(-22);
        }
        impls::close(source)?;
        owned.remove(0);
        let path = String::from("/bin/krust-cloexec-before");
        let child = impls::fork_execve(path.clone(), vec![path])?;
        let (_, status) = process::wait_for_child(Some(child))?;
        if status != 0 || process::fd_flags(flagged)? != 1 || process::fd_flags(normal)? != 0 {
            return Err(-5);
        }
        // Both images read the same inherited open description. Child flag changes
        // must remain local, while their file offset changes must reach the parent.
        let mut byte = [0];
        if impls::read(normal, &mut byte)? != 1 || byte[0] != b'c' {
            return Err(-5);
        }
        Ok(())
    })();
    for fd in owned {
        if let Err(errno) = impls::close(fd) {
            console.write_str(&format!("KRUST_CLOEXEC_CLEANUP_FAIL errno={errno}\n"));
        }
    }
    match result {
        Ok(()) => console.write_str("KRUST_CLOEXEC_FORK_OK\n"),
        Err(errno) => console.write_str(&format!("KRUST_CLOEXEC_FORK_FAIL errno={errno}\n")),
    }
}
