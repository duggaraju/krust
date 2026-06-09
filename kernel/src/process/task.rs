use super::context::CpuContext;

pub type Pid = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Ready,
    Running,
    Blocked,
    Zombie,
}

#[derive(Debug, Clone, Copy)]
pub struct Task {
    pub pid: Pid,
    pub state: TaskState,
    pub name: &'static str,
    pub kernel_stack_top: usize,
    pub context: CpuContext,
}

// SAFETY: Task is only accessed under a spin::Mutex in the scheduler.
// The kernel_stack_top is just an address value (usize), not a live pointer.
unsafe impl Send for Task {}

impl Task {
    pub fn new(pid: Pid, name: &'static str, entry_point: fn()) -> Self {
        let mut context = CpuContext::new();

        #[cfg(target_arch = "x86_64")]
        {
            context.rip = entry_point as usize as u64;
        }

        Self {
            pid,
            state: TaskState::Ready,
            name,
            kernel_stack_top: 0,
            context,
        }
    }
}
