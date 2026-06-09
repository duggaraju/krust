extern crate alloc;

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;
use core::arch::x86_64::__cpuid;
use spin::Mutex;

use super::task::{Pid, Task, TaskState};

pub static SCHEDULER: Mutex<Option<Scheduler>> = Mutex::new(None);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreWorkload {
    Idle,
    Task(Pid),
    Kernel(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessorCoreState {
    pub core_id: usize,
    pub workload: CoreWorkload,
}

pub struct Scheduler {
    tasks: VecDeque<Task>,
    current_index: Option<usize>,
    cores: Vec<ProcessorCoreState>,
}

impl Scheduler {
    pub fn new() -> Self {
        let core_count = detect_logical_cores().max(1);
        let mut cores = Vec::with_capacity(core_count);
        for core_id in 0..core_count {
            cores.push(ProcessorCoreState {
                core_id,
                workload: CoreWorkload::Idle,
            });
        }

        Self {
            tasks: VecDeque::new(),
            current_index: None,
            cores,
        }
    }

    pub fn add_task(&mut self, task: Task) {
        self.tasks.push_back(task);
    }

    pub fn tasks(&self) -> impl Iterator<Item = &Task> {
        self.tasks.iter()
    }

    pub fn task_by_pid(&self, pid: Pid) -> Option<&Task> {
        self.tasks.iter().find(|task| task.pid == pid)
    }

    pub fn task_by_pid_mut(&mut self, pid: Pid) -> Option<&mut Task> {
        self.tasks.iter_mut().find(|task| task.pid == pid)
    }

    pub fn set_task_state(&mut self, pid: Pid, state: TaskState) -> bool {
        if let Some(task) = self.tasks.iter_mut().find(|task| task.pid == pid) {
            task.state = state;
            if self
                .current_index
                .and_then(|idx| self.tasks.get(idx).map(|t| t.pid))
                == Some(pid)
                && state == TaskState::Zombie
            {
                self.current_index = None;
                self.clear_core_task(pid);
            }
            true
        } else {
            false
        }
    }

    pub fn schedule(&mut self) -> Option<&mut Task> {
        self.schedule_on(0)
    }

    pub fn schedule_on(&mut self, core_id: usize) -> Option<&mut Task> {
        let len = self.tasks.len();
        if len == 0 {
            self.current_index = None;
            self.set_core_idle(core_id);
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
                self.set_core_idle(core_id);
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

        let Some(selected_pid) = self.tasks.get(next_index).map(|task| task.pid) else {
            self.set_core_idle(core_id);
            return None;
        };
        self.set_core_task(core_id, selected_pid);

        let task = self
            .tasks
            .get_mut(next_index)
            .expect("task index just checked");
        task.state = TaskState::Running;
        Some(task)
    }

    pub fn current(&self) -> Option<&Task> {
        self.current_index.and_then(|index| self.tasks.get(index))
    }

    pub fn set_core_kernel_workload(&mut self, core_id: usize, reason: &str) -> bool {
        let Some(core) = self.cores.get_mut(core_id) else {
            return false;
        };
        core.workload = CoreWorkload::Kernel(String::from(reason));
        true
    }

    pub fn set_core_idle(&mut self, core_id: usize) {
        if let Some(core) = self.cores.get_mut(core_id) {
            core.workload = CoreWorkload::Idle;
        }
    }

    pub fn core_states(&self) -> &[ProcessorCoreState] {
        &self.cores
    }

    fn set_core_task(&mut self, core_id: usize, pid: Pid) {
        if let Some(core) = self.cores.get_mut(core_id) {
            core.workload = CoreWorkload::Task(pid);
        }
    }

    fn clear_core_task(&mut self, pid: Pid) {
        for core in &mut self.cores {
            if core.workload == CoreWorkload::Task(pid) {
                core.workload = CoreWorkload::Idle;
            }
        }
    }
}

pub fn init() {
    *SCHEDULER.lock() = Some(Scheduler::new());
}

fn detect_logical_cores() -> usize {
    let cpuid = __cpuid(1);
    ((cpuid.ebx >> 16) & 0xFF) as usize
}
