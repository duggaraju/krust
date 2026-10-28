use super::task::TaskMode;

pub const NS_PER_USER_TICK: u64 = 10_000_000;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CpuUsage {
    pub user_ns: u64,
    pub kernel_ns: u64,
}

impl CpuUsage {
    pub fn add(&mut self, other: Self) {
        self.user_ns = self.user_ns.saturating_add(other.user_ns);
        self.kernel_ns = self.kernel_ns.saturating_add(other.kernel_ns);
    }
}

#[derive(Debug)]
pub struct CpuAccounting {
    usage: CpuUsage,
    pub children: CpuUsage,
    mode: TaskMode,
    running_since: Option<u64>,
}

impl CpuAccounting {
    pub fn new() -> Self {
        Self {
            usage: CpuUsage::default(),
            children: CpuUsage::default(),
            mode: TaskMode::Kernel,
            running_since: None,
        }
    }

    pub fn snapshot(&self, now: u64) -> CpuUsage {
        let mut usage = self.usage;
        if let Some(start) = self.running_since {
            let elapsed = now.saturating_sub(start);
            match self.mode {
                TaskMode::User => usage.user_ns = usage.user_ns.saturating_add(elapsed),
                TaskMode::Kernel => usage.kernel_ns = usage.kernel_ns.saturating_add(elapsed),
            }
        }
        usage
    }

    pub fn pause(&mut self, now: u64) {
        self.usage = self.snapshot(now);
        self.running_since = None;
    }

    pub fn resume(&mut self, now: u64) {
        if self.running_since.is_none() {
            self.running_since = Some(now);
        }
    }

    pub fn set_mode(&mut self, mode: TaskMode, now: u64) {
        self.usage = self.snapshot(now);
        if self.running_since.is_some() {
            self.running_since = Some(now);
        }
        self.mode = mode;
    }

    pub fn reap(&mut self, child: &Self, now: u64) {
        self.children.add(child.snapshot(now));
        self.children.add(child.children);
    }
}
