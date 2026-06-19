pub mod binfmt;
pub mod context;
pub mod scheduler;
pub mod task;

use alloc::string::String;
use alloc::string::ToString;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use self::scheduler::SCHEDULER;
use self::task::{ControllingTerminal, Credentials, Task, TaskMode, TaskState};
use crate::fs::vfs::FileDescriptor;
use crate::fs::vfs::{File, FileType, FsError, Inode, OpenFile};

pub const ROOT_PID: u64 = 1;
pub const SHELL_PID: u64 = 2;

static CURRENT_PID: AtomicU64 = AtomicU64::new(ROOT_PID);
static NEXT_PID: AtomicU64 = AtomicU64::new(3);

fn dummy_entry() {}

pub fn init() {
    scheduler::init();
    binfmt::init();
}

pub fn register_boot_processes(root_cwd: Arc<dyn Inode>) {
    let root_credentials = Credentials::root();
    let shell_credentials = Credentials::root();
    let shell_home = shell_credentials.home_dir.clone();
    let (shell_cwd, shell_cwd_path) = resolve_boot_home(&shell_home, Arc::clone(&root_cwd));

    let mut scheduler = SCHEDULER.lock();
    let Some(scheduler) = scheduler.as_mut() else {
        return;
    };

    if scheduler.task_by_pid(ROOT_PID).is_none() {
        scheduler.add_task(Task::new_with_credentials(
            ROOT_PID,
            ROOT_PID,
            Arc::clone(&root_cwd),
            String::from("/"),
            "root",
            dummy_entry,
            root_credentials,
        ));
    }
    if scheduler.task_by_pid(SHELL_PID).is_none() {
        let parent_pid = scheduler
            .task_by_pid(ROOT_PID)
            .map(|task| task.pid)
            .unwrap_or(ROOT_PID);
        let mut shell = Task::new_with_credentials(
            SHELL_PID,
            parent_pid,
            shell_cwd,
            shell_cwd_path.clone(),
            "shell",
            dummy_entry,
            shell_credentials,
        );
        shell.set_env(String::from("HOME"), shell_cwd_path);
        scheduler.add_task(shell);
    } else if let Some(shell) = scheduler.task_by_pid_mut(SHELL_PID) {
        shell.set_home_dir(shell_cwd_path.clone());
        shell.set_cwd(shell_cwd, shell_cwd_path.clone());
        shell.set_env(String::from("HOME"), shell_cwd_path);
    }

    let _ = scheduler.set_task_state(ROOT_PID, TaskState::Running);
    let _ = scheduler.set_task_state(SHELL_PID, TaskState::Running);
}

fn resolve_boot_home(home: &str, root_cwd: Arc<dyn Inode>) -> (Arc<dyn Inode>, String) {
    match crate::fs::vfs::lookup_path(home) {
        Ok(inode) if inode.file_type() == FileType::Directory => (inode, String::from(home)),
        _ => (root_cwd, String::from("/")),
    }
}

pub fn set_current_pid(pid: u64) {
    CURRENT_PID.store(pid, Ordering::Relaxed);
}

pub fn current_pid() -> u64 {
    CURRENT_PID.load(Ordering::Relaxed)
}

pub fn alloc_pid() -> u64 {
    NEXT_PID.fetch_add(1, Ordering::Relaxed)
}

pub fn current_task_mode() -> Option<TaskMode> {
    let pid = current_pid();
    let scheduler = SCHEDULER.lock();
    scheduler
        .as_ref()
        .and_then(|scheduler| scheduler.task_by_pid(pid))
        .map(|task| task.mode)
}

pub fn set_current_task_mode(mode: TaskMode) -> bool {
    let pid = current_pid();
    let mut scheduler = SCHEDULER.lock();
    let Some(scheduler) = scheduler.as_mut() else {
        return false;
    };
    let Some(task) = scheduler.task_by_pid_mut(pid) else {
        return false;
    };
    task.set_mode(mode);
    true
}

/// Execute a closure with mutable access to the current task
pub fn with_current_task_mut<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut Task) -> R,
{
    let pid = current_pid();
    let mut scheduler = SCHEDULER.lock();
    let scheduler = scheduler.as_mut()?;
    let task = scheduler.task_by_pid_mut(pid)?;
    Some(f(task))
}

pub fn with_task_mut<F, R>(pid: u64, f: F) -> Option<R>
where
    F: FnOnce(&mut Task) -> R,
{
    let mut scheduler = SCHEDULER.lock();
    let scheduler = scheduler.as_mut()?;
    let task = scheduler.task_by_pid_mut(pid)?;
    Some(f(task))
}

pub fn current_env_var(key: &str) -> Option<String> {
    with_current_task_mut(|task| task.get_env(key).map(|value| value.to_string())).flatten()
}

pub fn current_uid() -> u32 {
    with_current_task_mut(|task| task.uid()).unwrap_or(0)
}

pub fn current_gid() -> u32 {
    with_current_task_mut(|task| task.gid()).unwrap_or(0)
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
    close_fd_for_pid(pid, fd)
}

fn close_fd_for_pid(pid: u64, fd: usize) -> Result<(), isize> {
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

pub fn close_all_fds(pid: u64) -> Result<(), isize> {
    let fds = with_task_mut(pid, |task| task.fd_entries().map(|(fd, _)| fd).collect::<Vec<_>>())
        .ok_or(-1isize)?;
    for fd in fds {
        let _ = close_fd_for_pid(pid, fd);
    }
    Ok(())
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

pub fn fork_current_task() -> Result<u64, isize> {
    let parent_pid = current_pid();
    let mut scheduler = SCHEDULER.lock();
    let Some(scheduler) = scheduler.as_mut() else {
        return Err(-1);
    };
    let Some(parent) = scheduler.task_by_pid(parent_pid).cloned() else {
        return Err(-1);
    };
    let parent_priority_before = parent.priority;

    let child_pid = alloc_pid();
    let mut child = parent.clone();
    child.pid = child_pid;
    child.parent_pid = parent_pid;
    child.state = TaskState::Ready;
    child.context.rax = 0;
    if child.context.rsp == 0 {
        child.context.rsp = child.kernel_stack_top as u64;
    }
    if child.context.rbp == 0 {
        child.context.rbp = child.kernel_stack_top as u64;
    }
    child.context.kernel_rsp = child.kernel_stack_top as u64;
    child.context.cr3 = child.address_space.root_paddr();
    let child_priority_before = child.priority;

    if let Some(parent_task) = scheduler.task_by_pid_mut(parent_pid) {
        parent_task.priority = parent_task.priority.saturating_sub(1);
    }

    scheduler.add_task(child);
    let parent_priority_after = scheduler
        .task_by_pid(parent_pid)
        .map(|task| task.priority)
        .unwrap_or(parent_priority_before);
    let child_priority_after = scheduler
        .task_by_pid(child_pid)
        .map(|task| task.priority)
        .unwrap_or(child_priority_before);
    let scheduler_current_pid = scheduler.current().map(|task| task.pid);
    let task_count = scheduler.tasks().count();
    let runnable_count = scheduler
        .tasks()
        .filter(|task| matches!(task.state, TaskState::Ready | TaskState::Running))
        .count();

    log::info!(
        "fork: parent_pid={} child_pid={} parent_prio(before={},after={}) child_prio(before={},after={}) sched_current={:?} tasks={} runnable={}",
        parent_pid,
        child_pid,
        parent_priority_before,
        parent_priority_after,
        child_priority_before,
        child_priority_after,
        scheduler_current_pid,
        task_count,
        runnable_count
    );
    Ok(child_pid)
}

pub fn wait_for_child(pid: Option<u64>) -> Result<(u64, i32), isize> {
    use x86_64::instructions::hlt;
    let parent_pid = current_pid();

    loop {
        let maybe_child = {
            let mut scheduler = SCHEDULER.lock();
            let Some(scheduler) = scheduler.as_mut() else {
                return Err(-1);
            };
            scheduler.reap_zombie_child(parent_pid, pid)
        };

        if let Some(result) = maybe_child {
            return Ok(result);
        }

        hlt();
    }
}

pub fn sched_yield_current_task() -> Option<u8> {
    let pid = current_pid();
    log::debug!("sched_yield: pid={} lowering priority", pid);
    let mut scheduler = SCHEDULER.lock();
    let scheduler = scheduler.as_mut()?;
    scheduler.lower_current_priority()
}

pub fn yield_to_task(pid: u64) -> Option<scheduler::SwitchPlan> {
    let mut scheduler = SCHEDULER.lock();
    let scheduler = scheduler.as_mut()?;
    scheduler.switch_to_pid(pid)
}

pub fn preempt_current_task_if_higher_priority() -> Option<scheduler::SwitchPlan> {
    let mut scheduler = SCHEDULER.lock();
    let scheduler = scheduler.as_mut()?;
    scheduler.preempt_if_higher_priority()
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
