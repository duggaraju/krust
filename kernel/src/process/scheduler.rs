extern crate alloc;

use alloc::collections::VecDeque;
use spin::Mutex;

use super::task::{Task, TaskState};

pub static SCHEDULER: Mutex<Option<Scheduler>> = Mutex::new(None);

pub struct Scheduler {
    tasks: VecDeque<Task>,
    current_index: Option<usize>,
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            tasks: VecDeque::new(),
            current_index: None,
        }
    }

    pub fn add_task(&mut self, task: Task) {
        self.tasks.push_back(task);
    }

    pub fn schedule(&mut self) -> Option<&mut Task> {
        let len = self.tasks.len();
        if len == 0 {
            self.current_index = None;
            return None;
        }

        let start = self.current_index.map_or(0, |index| (index + 1) % len);

        // Find next runnable task index
        let mut next_index = None;
        for offset in 0..len {
            let index = (start + offset) % len;
            if let Some(task) = self.tasks.get(index) {
                if matches!(task.state, TaskState::Ready | TaskState::Running) {
                    next_index = Some(index);
                    break;
                }
            }
        }

        let next_index = match next_index {
            Some(i) => i,
            None => {
                self.current_index = None;
                return None;
            }
        };

        // Mark previous task as Ready
        if let Some(previous) = self.current_index.filter(|&p| p < len && p != next_index) {
            if let Some(task) = self.tasks.get_mut(previous) {
                if task.state == TaskState::Running {
                    task.state = TaskState::Ready;
                }
            }
        }

        self.current_index = Some(next_index);

        if let Some(task) = self.tasks.get_mut(next_index) {
            task.state = TaskState::Running;
            Some(task)
        } else {
            None
        }
    }

    pub fn current(&self) -> Option<&Task> {
        self.current_index.and_then(|index| self.tasks.get(index))
    }
}

pub fn init() {
    *SCHEDULER.lock() = Some(Scheduler::new());
}
