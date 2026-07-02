use crate::process::task::Pid;
use alloc::vec::Vec;

/// A wait queue allows tasks to sleep until woken by another task.
/// Tasks can sleep on a queue and will be suspended until explicitly woken.
#[derive(Debug)]
pub struct WaitQueue {
    waiting_tasks: Vec<Pid>,
}

impl WaitQueue {
    /// Create a new empty wait queue.
    pub fn new() -> Self {
        WaitQueue {
            waiting_tasks: Vec::new(),
        }
    }

    /// Add a task to the wait queue. The task should be in Waiting state.
    pub fn add_task(&mut self, pid: Pid) {
        self.waiting_tasks.push(pid);
    }

    /// Wake the first task in the queue (FIFO). Returns the PID if found.
    pub fn wake_one(&mut self) -> Option<Pid> {
        if self.waiting_tasks.is_empty() {
            None
        } else {
            Some(self.waiting_tasks.remove(0))
        }
    }

    /// Wake all tasks in the queue. Returns list of PIDs.
    pub fn wake_all(&mut self) -> Vec<Pid> {
        core::mem::take(&mut self.waiting_tasks)
    }

    /// Check if queue has waiting tasks.
    pub fn is_empty(&self) -> bool {
        self.waiting_tasks.is_empty()
    }

    /// Number of tasks waiting.
    pub fn len(&self) -> usize {
        self.waiting_tasks.len()
    }
}

impl Default for WaitQueue {
    fn default() -> Self {
        Self::new()
    }
}
