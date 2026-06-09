pub mod context;
pub mod scheduler;
pub mod task;

use alloc::string::String;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicU64, Ordering};

use self::scheduler::SCHEDULER;
use self::task::{ControllingTerminal, Task, TaskState};
use crate::fs::vfs::{File, FsError, Inode, OpenFile};
use crate::fs::vfs::FileDescriptor;

pub const ROOT_PID: u64 = 1;
pub const SHELL_PID: u64 = 2;

static CURRENT_PID: AtomicU64 = AtomicU64::new(ROOT_PID);

fn dummy_entry() {}

pub fn init() {
    scheduler::init();
}

pub fn register_boot_processes(root_cwd: Arc<dyn Inode>) {
    let mut scheduler = SCHEDULER.lock();
    let Some(scheduler) = scheduler.as_mut() else {
        return;
    };

    if scheduler.task_by_pid(ROOT_PID).is_none() {
        scheduler.add_task(Task::new(
            ROOT_PID,
            ROOT_PID,
            Arc::clone(&root_cwd),
            String::from("/"),
            "root",
            dummy_entry,
        ));
    }
    if scheduler.task_by_pid(SHELL_PID).is_none() {
        let parent_pid = scheduler
            .task_by_pid(ROOT_PID)
            .map(|task| task.pid)
            .unwrap_or(ROOT_PID);
        scheduler.add_task(Task::new(
            SHELL_PID,
            parent_pid,
            Arc::clone(&root_cwd),
            String::from("/"),
            "shell",
            dummy_entry,
        ));
    }

    let _ = scheduler.set_task_state(ROOT_PID, TaskState::Running);
    let _ = scheduler.set_task_state(SHELL_PID, TaskState::Running);
}

pub fn set_current_pid(pid: u64) {
    CURRENT_PID.store(pid, Ordering::Relaxed);
}

pub fn current_pid() -> u64 {
    CURRENT_PID.load(Ordering::Relaxed)
}

pub fn current_cwd() -> Option<Arc<dyn Inode>> {
    let pid = current_pid();
    let scheduler = SCHEDULER.lock();
    scheduler
        .as_ref()
        .and_then(|scheduler| scheduler.task_by_pid(pid))
        .map(|task| task.cwd())
}

pub fn current_cwd_path() -> Option<String> {
    let pid = current_pid();
    let scheduler = SCHEDULER.lock();
    scheduler
        .as_ref()
        .and_then(|scheduler| scheduler.task_by_pid(pid))
        .map(|task| task.cwd_path.clone())
}

pub fn set_current_cwd(cwd: Arc<dyn Inode>, cwd_path: String) -> Result<(), isize> {
    let pid = current_pid();
    let mut scheduler = SCHEDULER.lock();
    let Some(scheduler) = scheduler.as_mut() else {
        return Err(-1);
    };
    let Some(task) = scheduler.task_by_pid_mut(pid) else {
        return Err(-1);
    };
    task.set_cwd(cwd, cwd_path);
    Ok(())
}

pub fn open_file(file: OpenFile) -> Result<usize, isize> {
    let pid = current_pid();
    let mut scheduler = SCHEDULER.lock();
    let Some(scheduler) = scheduler.as_mut() else {
        return Err(-1);
    };
    let Some(task) = scheduler.task_by_pid_mut(pid) else {
        return Err(-1);
    };

    Ok(task.open_fd(file))
}

pub fn close_fd(fd: usize) -> Result<(), isize> {
    let pid = current_pid();
    let file = {
        let mut scheduler = SCHEDULER.lock();
        let Some(scheduler) = scheduler.as_mut() else {
            return Err(-1);
        };
        let Some(task) = scheduler.task_by_pid_mut(pid) else {
            return Err(-1);
        };
        task.close_fd(fd).ok_or(-9isize)?
    };
    let mut file = file;
    file.close().map_err(errno_from_fs)
}

fn errno_from_fs(err: FsError) -> isize {
    match err {
        FsError::NotFound => -2,
        FsError::PermissionDenied => -13,
        FsError::NotADirectory => -20,
        FsError::IsADirectory => -21,
        FsError::AlreadyExists => -17,
        FsError::IoError => -5,
        FsError::Busy => -16,
        FsError::NotSupported => -95,
    }
}

pub fn read_fd(fd: usize, buf: &mut [u8]) -> Result<usize, isize> {
    let pid = current_pid();
    let file = {
        let mut scheduler = SCHEDULER.lock();
        let Some(scheduler) = scheduler.as_mut() else {
            return Err(-1);
        };
        let Some(task) = scheduler.task_by_pid_mut(pid) else {
            return Err(-1);
        };
        match task.take_fd(fd) {
            Some(file) => file,
            None => return Err(-9),
        }
    };

    let mut file = file;
    let result = file.read(buf).map_err(errno_from_fs);

    let mut scheduler = SCHEDULER.lock();
    let Some(scheduler) = scheduler.as_mut() else {
        return Err(-1);
    };
    let Some(task) = scheduler.task_by_pid_mut(pid) else {
        return Err(-1);
    };
    task.put_fd(fd, file);
    result
}

pub fn write_fd(fd: usize, buf: &[u8]) -> Result<usize, isize> {
    let pid = current_pid();
    let file = {
        let mut scheduler = SCHEDULER.lock();
        let Some(scheduler) = scheduler.as_mut() else {
            return Err(-1);
        };
        let Some(task) = scheduler.task_by_pid_mut(pid) else {
            return Err(-1);
        };
        match task.take_fd(fd) {
            Some(file) => file,
            None => return Err(-9),
        }
    };

    let mut file = file;
    let result = file.write(buf).map_err(errno_from_fs);

    let mut scheduler = SCHEDULER.lock();
    let Some(scheduler) = scheduler.as_mut() else {
        return Err(-1);
    };
    let Some(task) = scheduler.task_by_pid_mut(pid) else {
        return Err(-1);
    };
    task.put_fd(fd, file);
    result
}

pub fn with_fd<R>(
    fd: usize,
    f: impl FnOnce(&mut OpenFile) -> Result<R, isize>,
) -> Result<R, isize> {
    let pid = current_pid();
    let mut file = {
        let mut scheduler = SCHEDULER.lock();
        let Some(scheduler) = scheduler.as_mut() else {
            return Err(-1);
        };
        let Some(task) = scheduler.task_by_pid_mut(pid) else {
            return Err(-1);
        };
        match task.take_fd(fd) {
            Some(file) => file,
            None => return Err(-9),
        }
    };

    let result = f(&mut file);

    let mut scheduler = SCHEDULER.lock();
    let Some(scheduler) = scheduler.as_mut() else {
        return Err(-1);
    };
    let Some(task) = scheduler.task_by_pid_mut(pid) else {
        return Err(-1);
    };
    task.put_fd(fd, file);
    result
}

pub fn fd_descriptor(fd: usize) -> Result<FileDescriptor, isize> {
    let pid = current_pid();
    let scheduler = SCHEDULER.lock();
    let Some(scheduler) = scheduler.as_ref() else {
        return Err(-1);
    };
    let Some(task) = scheduler.task_by_pid(pid) else {
        return Err(-1);
    };
    task.fd_descriptor(fd).ok_or(-9)
}

pub fn mark_shell_exited() {
    if let Some(scheduler) = SCHEDULER.lock().as_mut() {
        let _ = scheduler.set_task_state(SHELL_PID, TaskState::Zombie);
    }
}

pub fn terminate_root_process() -> bool {
    if let Some(scheduler) = SCHEDULER.lock().as_mut() {
        return scheduler.set_task_state(ROOT_PID, TaskState::Zombie);
    }
    false
}

/// Assign a controlling terminal to the given process.
#[cfg(feature = "drivers")]
pub(crate) fn set_controlling_terminal(pid: u64, target: ControllingTerminal) {
    if let Some(scheduler) = SCHEDULER.lock().as_mut() {
        if let Some(task) = scheduler.task_by_pid_mut(pid) {
            task.set_controlling_terminal(target);
        }
    }
}

/// Return the controlling terminal of the current process, if set.
#[cfg(feature = "drivers")]
pub(crate) fn current_controlling_terminal() -> Option<ControllingTerminal> {
    let pid = current_pid();
    let scheduler = SCHEDULER.lock();
    scheduler
        .as_ref()
        .and_then(|s| s.task_by_pid(pid))
        .and_then(|task| task.controlling_terminal())
}
