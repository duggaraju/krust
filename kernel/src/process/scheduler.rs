extern crate alloc;

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;
use core::arch::x86_64::__cpuid;
use spin::Mutex;

use super::task::{Pid, Task, TaskMode, TaskState};

pub static SCHEDULER: Mutex<Option<Scheduler>> = Mutex::new(None);

/// System uptime in milliseconds since boot (incremented by timer interrupt every 10ms)
pub static UPTIME_MS: Mutex<u64> = Mutex::new(0);

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

pub struct SwitchPlan {
    pub old_ctx: *mut crate::process::context::CpuContext,
    pub new_ctx: *const crate::process::context::CpuContext,
    pub new_pid: Pid,
}

impl Scheduler {
    fn is_canonical_address(addr: u64) -> bool {
        let sign = (addr >> 47) & 1;
        let upper = addr >> 48;
        if sign == 0 {
            upper == 0
        } else {
            upper == 0xFFFF
        }
    }

    fn validate_switch_target(task: &Task) -> bool {
        let (stack_base, stack_top) = task.kernel_stack_range();
        let rsp = task.context.kernel_stack_pointer() as usize;
        let rip = task.context.kernel_entry_point();
        let rsp_in_range = rsp >= stack_base && rsp <= stack_top;
        let rsp_canonical = Self::is_canonical_address(task.context.kernel_stack_pointer());
        let rip_canonical = Self::is_canonical_address(rip);
        let rip_nonzero = rip != 0;
        let rsp_ok = match task.mode {
            TaskMode::Kernel => rsp_canonical,
            TaskMode::User => rsp_in_range && rsp_canonical,
        };

        if !rsp_ok || !rip_canonical || !rip_nonzero {
            log::error!(
                "sched: refusing switch to invalid target pid={} mode={:?} context={:?} stack=[0x{:x}..0x{:x}] checks: rsp_in_range={} rsp_canonical={} rip_canonical={} rip_nonzero={}",
                task.pid,
                task.mode,
                task.context,
                stack_base,
                stack_top,
                rsp_in_range,
                rsp_canonical,
                rip_canonical,
                rip_nonzero
            );
            return false;
        }
        true
    }

    fn normalize_switch_target(task: &mut Task) {
        let (stack_base, stack_top) = task.kernel_stack_range();
        let rsp = task.context.kernel_stack_pointer() as usize;
        let kernel_rsp = task.context.kernel_stack_top() as usize;
        let rsp_in_range = rsp >= stack_base && rsp <= stack_top;
        let kernel_rsp_in_range = kernel_rsp >= stack_base && kernel_rsp <= stack_top;

        if !kernel_rsp_in_range {
            return;
        }

        match task.mode {
            TaskMode::Kernel => {}
            TaskMode::User => {
                // User tasks must still run on a kernel stack while in kernel context;
                // user_stack_pointer/user_entry_point are consumed by enter_usermode/sysret paths.
                if !rsp_in_range {
                    task.context
                        .set_kernel_stack_pointer(task.context.kernel_stack_top());
                    if task.context.frame_pointer() == 0 {
                        task.context
                            .set_frame_pointer(task.context.kernel_stack_top());
                    }
                    log::warn!(
                        "sched: repaired user task pid={} mode={:?} context={:?}",
                        task.pid,
                        task.mode,
                        task.context
                    );
                }
                if task.context.user_stack_pointer() == 0 && task.user_stack_top != 0 {
                    task.context
                        .set_user_stack_pointer(task.user_stack_top as u64);
                }
                if task.context.user_entry_point() == 0 && task.context.kernel_entry_point() != 0 {
                    task.context
                        .set_user_entry_point(task.context.kernel_entry_point());
                }
            }
        }
    }

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
        crate::process::set_current_pid(selected_pid);
        self.set_core_task(core_id, selected_pid);

        let task = self
            .tasks
            .get_mut(next_index)
            .expect("task index just checked");
        task.state = TaskState::Running;
        task.activate_address_space();
        crate::arch::activate_task_runtime_state(task);
        Some(task)
    }

    pub fn current(&self) -> Option<&Task> {
        self.current_index.and_then(|index| self.tasks.get(index))
    }

    /// Perform a preemptive context switch if another task is scheduled.
    /// Marks the current task as Ready, schedules the next task, and performs the switch if needed.
    pub fn preempt_and_switch(&mut self) -> Option<SwitchPlan> {
        let old_idx = self.current_index;
        let old_pid = old_idx.and_then(|idx| self.tasks.get(idx).map(|t| t.pid));

        // Mark current task as Ready for rescheduling
        if let Some(pid) = old_pid {
            if !matches!(
                self.task_by_pid(pid).map(|task| task.state),
                Some(TaskState::Zombie)
            ) {
                let _ = self.set_task_state(pid, TaskState::Ready);
            }
        }

        // Schedule next task
        if let Some(next_task) = self.schedule_on(0) {
            let new_pid = next_task.pid;

            // Only switch if we're moving to a different task
            if Some(new_pid) != old_pid && old_idx.is_some() {
                log::debug!(
                    "Context switch: PID {} → PID {}",
                    old_pid.unwrap_or(0),
                    new_pid
                );

                let old_idx = old_idx.unwrap();
                let new_idx = self.current_index.unwrap();

                // Get pointers to both contexts
                let old_ctx_ptr = &mut self.tasks[old_idx].context as *mut _;
                let new_ctx_ptr = &self.tasks[new_idx].context as *const _;
                return Some(SwitchPlan {
                    old_ctx: old_ctx_ptr,
                    new_ctx: new_ctx_ptr,
                    new_pid,
                });
            }
        }

        None
    }

    /// Switch execution to a specific runnable PID.
    pub fn switch_to_pid(&mut self, pid: Pid) -> Option<SwitchPlan> {
        if self.current_index.is_none() {
            let current_pid = crate::process::current_pid();
            self.current_index = self.tasks.iter().position(|task| task.pid == current_pid);
        }

        let old_idx = self.current_index?;
        let target_idx = self.tasks.iter().position(|task| task.pid == pid)?;
        if old_idx == target_idx {
            return None;
        }

        if !matches!(
            self.tasks.get(target_idx).map(|task| task.state),
            Some(TaskState::Ready | TaskState::Running)
        ) {
            return None;
        }
        if let Some(task) = self.tasks.get_mut(target_idx) {
            Self::normalize_switch_target(task);
        }
        if !Self::validate_switch_target(self.tasks.get(target_idx)?) {
            return None;
        }

        let old_pid = self.tasks.get(old_idx).map(|task| task.pid)?;
        let new_pid = self.tasks.get(target_idx).map(|task| task.pid)?;

        if let Some(task) = self.tasks.get_mut(old_idx) {
            if task.state == TaskState::Running {
                task.state = TaskState::Ready;
            }
        }

        self.current_index = Some(target_idx);
        crate::process::set_current_pid(new_pid);
        self.set_core_task(0, new_pid);

        let kernel_stack_top = {
            let task = self.tasks.get_mut(target_idx)?;
            task.state = TaskState::Running;
            task.activate_address_space();
            task.kernel_stack_top
        };
        if let Some(task) = self.tasks.get(target_idx) {
            let _ = kernel_stack_top;
            crate::arch::activate_task_runtime_state(task);
        }

        let old_ctx_ptr = &mut self.tasks[old_idx].context as *mut _;
        let new_ctx_ptr = &self.tasks[target_idx].context as *const _;
        log::debug!("sched: explicit switch PID {} -> PID {}", old_pid, new_pid);
        Some(SwitchPlan {
            old_ctx: old_ctx_ptr,
            new_ctx: new_ctx_ptr,
            new_pid,
        })
    }

    pub fn lower_current_priority(&mut self) -> Option<u8> {
        let idx = self.current_index?;
        let task = self.tasks.get_mut(idx)?;
        task.priority = task.priority.saturating_sub(1);
        Some(task.priority)
    }

    pub fn reap_zombie_child(
        &mut self,
        parent_pid: Pid,
        expected_pid: Option<Pid>,
    ) -> Option<(Pid, i32)> {
        let idx = self.tasks.iter().position(|task| {
            task.parent_pid == parent_pid
                && task.state == TaskState::Zombie
                && !matches!(expected_pid, Some(expected) if expected != task.pid)
        })?;

        let task = self.tasks.remove(idx)?;
        let pid = task.pid;
        let status = task.exit_code();
        self.clear_core_task(pid);

        if let Some(current) = self.current_index {
            self.current_index = if idx < current {
                Some(current - 1)
            } else if idx == current {
                None
            } else {
                Some(current)
            };
        }

        Some((pid, status))
    }

    pub fn preempt_if_higher_priority(&mut self) -> Option<SwitchPlan> {
        let current_idx = self.current_index?;
        let current_priority = self.tasks.get(current_idx)?.priority;
        let mut next_index = None;
        let mut next_priority = current_priority;

        for (idx, task) in self.tasks.iter().enumerate() {
            if idx == current_idx {
                continue;
            }
            if !matches!(task.state, TaskState::Ready | TaskState::Running) {
                continue;
            }
            if task.priority > next_priority {
                next_priority = task.priority;
                next_index = Some(idx);
            }
        }

        let next_index = next_index?;
        if next_priority <= current_priority {
            return None;
        }

        let old_pid = self.tasks.get(current_idx).map(|task| task.pid)?;
        let new_pid = self.tasks.get(next_index).map(|task| task.pid)?;

        if let Some(task) = self.tasks.get_mut(current_idx) {
            if task.state == TaskState::Running {
                task.state = TaskState::Ready;
            }
        }

        self.current_index = Some(next_index);
        crate::process::set_current_pid(new_pid);
        self.set_core_task(0, new_pid);

        let kernel_stack_top = {
            let task = self.tasks.get_mut(next_index)?;
            task.state = TaskState::Running;
            task.activate_address_space();
            task.kernel_stack_top
        };
        if let Some(task) = self.tasks.get(next_index) {
            let _ = kernel_stack_top;
            crate::arch::activate_task_runtime_state(task);
        }

        log::debug!(
            "Priority preemption: PID {} → PID {} ({} -> {})",
            old_pid,
            new_pid,
            current_priority,
            next_priority
        );

        let old_ctx_ptr = &mut self.tasks[current_idx].context as *mut _;
        let new_ctx_ptr = &self.tasks[next_index].context as *const _;
        Some(SwitchPlan {
            old_ctx: old_ctx_ptr,
            new_ctx: new_ctx_ptr,
            new_pid,
        })
    }

    /// Increment the current task's time usage by one time quantum (called by timer interrupt)
    pub fn increment_current_time_usage(&mut self) {
        if let Some(idx) = self.current_index {
            if let Some(task) = self.tasks.get_mut(idx) {
                task.increment_time_usage();
            }
        }
    }

    /// Set the start time for the current task (called when transitioning to Running)
    pub fn set_current_task_start_time(&mut self, uptime_ms: u64) {
        if let Some(idx) = self.current_index {
            if let Some(task) = self.tasks.get_mut(idx) {
                task.set_start_time(uptime_ms);
            }
        }
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
