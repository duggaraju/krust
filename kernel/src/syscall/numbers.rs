pub const SYS_READ: usize = 0;
pub const SYS_WRITE: usize = 1;
pub const SYS_OPEN: usize = 2;
pub const SYS_CLOSE: usize = 3;
pub const SYS_STAT: usize = 4;
pub const SYS_CHDIR: usize = 80;
pub const SYS_TIMES: usize = 100;
pub const SYS_MMAP: usize = 9;
pub const SYS_BRK: usize = 12;
pub const SYS_GETPID: usize = 39;
pub const SYS_FORK: usize = 57;
pub const SYS_EXECVE: usize = 59;
pub const SYS_EXIT: usize = 60;
pub const SYS_WAIT4: usize = 61;
pub const SYS_SHUTDOWN: usize = 500;

pub const MAX_SYSCALL: usize = 512;
