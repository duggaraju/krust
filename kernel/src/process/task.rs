extern crate alloc;

use super::context::CpuContext;
use crate::fs::vfs::{File, FileDescriptor, FileOpenMode, Inode, OpenFile, SeekFrom};
use crate::mm::address_space::AddressSpace;
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

/// Environment variable map: Copy-on-Write semantics via Arc<BTreeMap>
/// Wrapped in Arc for zero-cost task cloning. Only allocates on modification.
pub type EnvMap = BTreeMap<String, String>;

#[cfg(feature = "drivers")]
#[derive(Clone, Copy)]
pub(crate) enum ControllingTerminal {
    VirtualConsole(usize),
    Serial(usize),
}

pub type Pid = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Ready,
    Running,
    Blocked,
    Zombie,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskMode {
    Kernel,
    User,
}

#[derive(Clone)]
pub struct Credentials {
    pub uid: u32,
    pub gid: u32,
    pub home_dir: String,
}

impl Credentials {
    pub fn new(uid: u32, gid: u32, home_dir: String) -> Self {
        Self { uid, gid, home_dir }
    }

    pub fn root() -> Self {
        Self::new(0, 0, String::from("/root"))
    }
}

/// Process signals (Linux compatible)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Signal {
    /// SIGTERM: Software termination signal (graceful shutdown)
    Sigterm = 15,
    /// SIGKILL: Kill signal (cannot be caught or ignored)
    Sigkill = 9,
    /// SIGINT: Interrupt signal (from Ctrl-C)
    Sigint = 2,
    /// SIGSTOP: Stop signal (pause execution)
    Sigstop = 19,
    /// SIGCONT: Continue signal (resume from stop)
    Sigcont = 18,
}

impl Signal {
    /// Returns whether this signal should immediately terminate the task
    /// (no handlers, no pending queue).
    pub fn is_fatal(&self) -> bool {
        matches!(self, Signal::Sigkill | Signal::Sigterm)
    }

    /// Returns the standard exit code for this signal (128 + signal_number).
    pub fn exit_code(&self) -> i32 {
        128 + (*self as i32)
    }
}

pub struct Task {
    pub pid: Pid,
    pub parent_pid: Pid,
    pub cwd_inode: u64,
    pub cwd_path: String,
    cwd: Arc<dyn Inode>,
    pub state: TaskState,
    pub mode: TaskMode,
    pub name: &'static str,
    pub kernel_stack_top: usize,
    kernel_stack: Box<[u8]>,
    pub context: CpuContext,
    pub priority: u8,
    pub credentials: Credentials,
    pub user_stack_top: usize,
    pub fs_base: u64,
    pub gs_base: u64,
    pub brk_start: usize,
    pub brk_end: usize,
    pub brk_mapped_end: usize,
    pub mmap_start: usize,
    pub mmap_end: usize,
    pub userland: bool,
    pub exec_path: String,
    /// Time usage in milliseconds (incremented by timer interrupt every 10ms)
    pub time_usage_ms: u64,
    /// Time when task was last started running (in milliseconds since boot)
    start_time_ms: u64,
    fds: FdTable,
    /// Environment variables: Arc<EnvMap> enables CoW semantics.
    /// Cloning a task is O(1); modifying an env var triggers Arc::make_mut().
    pub env: Arc<EnvMap>,
    /// Command-line arguments passed to the binary
    pub argv: Vec<String>,
    pub exit_code: i32,
    pub address_space: AddressSpace,
    /// The controlling terminal for this process. Set by the kernel when
    /// spawning shell/interactive tasks; `None` for background tasks.
    #[cfg(feature = "drivers")]
    controlling_terminal: Option<ControllingTerminal>,
    /// Pending signals delivered to this task
    pub pending_signals: Vec<Signal>,
    /// Whether the task was terminated by a signal (used to track exit reason)
    pub terminated_by_signal: Option<Signal>,
}

// SAFETY: Task is only accessed under a spin::Mutex in the scheduler.
// The kernel_stack_top is just an address value (usize), not a live pointer.
unsafe impl Send for Task {}

impl Task {
    pub fn new(
        pid: Pid,
        parent_pid: Pid,
        cwd: Arc<dyn Inode>,
        cwd_path: String,
        name: &'static str,
        entry_point: fn(),
    ) -> Self {
        Self::new_with_credentials(
            pid,
            parent_pid,
            cwd,
            cwd_path,
            name,
            entry_point,
            Credentials::root(),
        )
    }

    pub fn new_with_credentials(
        pid: Pid,
        parent_pid: Pid,
        cwd: Arc<dyn Inode>,
        cwd_path: String,
        name: &'static str,
        entry_point: fn(),
        credentials: Credentials,
    ) -> Self {
        Self::new_with_terminal(
            pid,
            parent_pid,
            cwd,
            cwd_path,
            name,
            entry_point,
            credentials,
            None,
        )
    }

    pub(crate) fn new_with_terminal(
        pid: Pid,
        parent_pid: Pid,
        cwd: Arc<dyn Inode>,
        cwd_path: String,
        name: &'static str,
        entry_point: fn(),
        credentials: Credentials,
        #[cfg(feature = "drivers")] controlling_terminal: Option<ControllingTerminal>,
        #[cfg(not(feature = "drivers"))] _controlling_terminal: Option<()>,
    ) -> Self {
        let cwd_inode = cwd.ino();
        let mut context = CpuContext::new();
        let kernel_stack = alloc::vec![0u8; KERNEL_STACK_SIZE].into_boxed_slice();
        let kernel_stack_top = (kernel_stack.as_ptr() as usize + kernel_stack.len()) & !0xf;
        let address_space = AddressSpace::kernel().expect("failed to allocate task address space");

        #[cfg(target_arch = "x86_64")]
        {
            context.rip = entry_point as usize as u64;
        }
        context.kernel_rsp = kernel_stack_top as u64;
        context.rflags = 0x202;
        context.cr3 = address_space.root_paddr();

        Self {
            pid,
            parent_pid,
            cwd_inode,
            cwd_path,
            cwd,
            state: TaskState::Ready,
            mode: TaskMode::Kernel,
            name,
            kernel_stack_top,
            kernel_stack,
            context,
            priority: 20,
            credentials,
            user_stack_top: 0,
            fs_base: 0,
            gs_base: 0,
            brk_start: 0,
            brk_end: 0,
            brk_mapped_end: 0,
            mmap_start: 0,
            mmap_end: 0,
            userland: false,
            exec_path: String::new(),
            time_usage_ms: 0,
            start_time_ms: 0,
            fds: FdTable::new(),
            env: Arc::new(BTreeMap::new()),
            argv: vec![],
            exit_code: 0,
            address_space,
            #[cfg(feature = "drivers")]
            controlling_terminal,
            pending_signals: Vec::new(),
            terminated_by_signal: None,
        }
        .with_default_stdio()
    }

    pub fn child_of(pid: Pid, parent: &Task, name: &'static str, entry_point: fn()) -> Self {
        let mut child = Self::new_with_terminal(
            pid,
            parent.pid,
            Arc::clone(&parent.cwd),
            parent.cwd_path.clone(),
            name,
            entry_point,
            parent.credentials.clone(),
            #[cfg(feature = "drivers")]
            parent.controlling_terminal,
            #[cfg(not(feature = "drivers"))]
            None,
        );
        // Share parent's environment map (CoW: cloned only on modification)
        child.env = Arc::clone(&parent.env);
        child.fds = parent.fds.clone();
        child.address_space = parent.address_space.clone();
        child.context.cr3 = child.address_space.root_paddr();
        child.brk_start = parent.brk_start;
        child.brk_end = parent.brk_end;
        child.brk_mapped_end = parent.brk_mapped_end;
        child.mmap_start = parent.mmap_start;
        child.mmap_end = parent.mmap_end;
        child
    }

    pub fn cwd(&self) -> Arc<dyn Inode> {
        Arc::clone(&self.cwd)
    }

    pub fn kernel_stack_range(&self) -> (usize, usize) {
        let top = self.kernel_stack_top;
        let base = top.saturating_sub(self.kernel_stack.len());
        (base, top)
    }

    /// Set the controlling terminal for this task.
    #[cfg(feature = "drivers")]
    pub(crate) fn set_controlling_terminal(&mut self, target: ControllingTerminal) {
        self.controlling_terminal = Some(target);
        self.refresh_stdio();
    }

    /// Return the controlling terminal, if one has been assigned.
    #[cfg(feature = "drivers")]
    pub(crate) fn controlling_terminal(&self) -> Option<ControllingTerminal> {
        self.controlling_terminal
    }

    pub fn set_cwd(&mut self, cwd: Arc<dyn Inode>, cwd_path: String) {
        self.cwd_inode = cwd.ino();
        self.cwd_path = cwd_path;
        self.cwd = cwd;
    }

    pub fn set_mode(&mut self, mode: TaskMode) {
        self.mode = mode;
    }

    pub fn uid(&self) -> u32 {
        self.credentials.uid
    }

    pub fn gid(&self) -> u32 {
        self.credentials.gid
    }

    pub fn home_dir(&self) -> &str {
        self.credentials.home_dir.as_str()
    }

    pub fn set_uid(&mut self, uid: u32) {
        self.credentials.uid = uid;
    }

    pub fn set_gid(&mut self, gid: u32) {
        self.credentials.gid = gid;
    }

    pub fn set_home_dir(&mut self, home_dir: String) {
        self.credentials.home_dir = home_dir;
    }

    fn with_default_stdio(mut self) -> Self {
        self.refresh_stdio();
        self
    }

    fn refresh_stdio(&mut self) {
        let target = self.default_stdio_path();
        let fallback = "/dev/null";

        if let Ok(file) =
            crate::fs::vfs::open_path(target).or_else(|_| crate::fs::vfs::open_path(fallback))
        {
            self.fds.insert_at(0, file, FileOpenMode::ReadOnly);
        }
        if let Ok(file) =
            crate::fs::vfs::open_path(target).or_else(|_| crate::fs::vfs::open_path(fallback))
        {
            self.fds.insert_at(1, file, FileOpenMode::WriteOnly);
        }
        if let Ok(file) =
            crate::fs::vfs::open_path(target).or_else(|_| crate::fs::vfs::open_path(fallback))
        {
            self.fds.insert_at(2, file, FileOpenMode::WriteOnly);
        }
    }

    fn default_stdio_path(&self) -> &'static str {
        #[cfg(feature = "drivers")]
        if let Some(ctty) = self.controlling_terminal {
            return controlling_terminal_device_path(ctty);
        }

        "/dev/null"
    }

    pub fn open_fd(&mut self, file: OpenFile) -> usize {
        self.fds.insert(file, FileOpenMode::ReadWrite)
    }

    pub fn close_fd(&mut self, fd: usize) -> Option<OpenFile> {
        self.fds.remove(fd)
    }

    pub fn take_fd(&mut self, fd: usize) -> Option<OpenFile> {
        self.fds.remove(fd)
    }

    pub fn put_fd(&mut self, fd: usize, file: OpenFile) {
        self.fds.insert_at(fd, file, FileOpenMode::ReadWrite);
    }

    pub fn fd_entries(&self) -> impl Iterator<Item = (usize, FileDescriptor)> + '_ {
        self.fds.iter().map(|(fd, entry)| (fd, entry.descriptor))
    }

    pub fn fd_descriptor(&self, fd: usize) -> Option<FileDescriptor> {
        self.fds.descriptor(fd)
    }

    /// Get an environment variable (zero-copy)
    pub fn get_env(&self, key: &str) -> Option<&str> {
        self.env.get(key).map(|s| s.as_str())
    }

    /// Set an environment variable (CoW: clones map only if shared)
    pub fn set_env(&mut self, key: String, value: String) {
        Arc::make_mut(&mut self.env).insert(key, value);
    }

    /// Set command-line arguments
    pub fn set_argv(&mut self, argv: Vec<String>) {
        self.argv = argv;
    }

    /// Update task's entry point for a loaded binary
    #[cfg(target_arch = "x86_64")]
    pub fn set_entry_point(&mut self, entry: usize) {
        self.context.rip = entry as u64;
    }

    /// Apply a binary LoadPlan: update argv and entry point
    pub fn apply_load_plan(&mut self, plan: &crate::process::binfmt::LoadPlan, argv: Vec<String>) {
        self.set_argv(argv);
        self.set_entry_point(plan.entry_point);
        self.context.user_rip = plan.entry_point as u64;
    }

    pub fn prepare_user_exec(&mut self, entry: usize, stack_top: usize) {
        self.context.user_rip = entry as u64;
        self.context.user_rsp = stack_top as u64;
        self.context.rip =
            crate::arch::x86_64::context::enter_usermode as *const () as usize as u64;
        self.context.rflags = 0x202;
        self.context.kernel_rsp = self.kernel_stack_top as u64;
        self.context.rsp = self.context.kernel_rsp;
        self.context.rbp = self.context.kernel_rsp;
        self.context.cr3 = self.address_space.root_paddr();
        self.user_stack_top = stack_top;
        self.mode = TaskMode::User;
        self.userland = true;
    }

    pub fn initialize_brk(&mut self, start: usize) {
        self.brk_start = start;
        self.brk_end = start;
        self.brk_mapped_end = start;
    }

    pub fn set_mmap_start(&mut self, start: usize) {
        self.mmap_start = start;
        self.mmap_end = start;
    }

    /// Record when this task started running (called when transitioning to Running state)
    pub fn set_start_time(&mut self, uptime_ms: u64) {
        self.start_time_ms = uptime_ms;
    }

    /// Increment time usage by one time quantum (called by timer interrupt)
    pub fn increment_time_usage(&mut self) {
        self.time_usage_ms += 10; // PIT timer fires every 10ms
    }

    pub fn set_exit_code(&mut self, code: i32) {
        self.exit_code = code;
    }

    pub fn exit_code(&self) -> i32 {
        self.exit_code
    }

    pub fn set_user_entry(&mut self, entry: usize, stack_top: usize) {
        self.context.user_rip = entry as u64;
        self.context.user_rsp = stack_top as u64;
        self.user_stack_top = stack_top;
    }

    pub fn activate_address_space(&self) {
        self.address_space.activate();
    }

    /// Deliver a signal to this task.
    /// Fatal signals (SIGKILL, SIGTERM) immediately terminate.
    /// Other signals are queued for later processing.
    pub fn deliver_signal(&mut self, signal: Signal) {
        if signal.is_fatal() {
            self.terminated_by_signal = Some(signal);
            self.exit_code = signal.exit_code();
            self.state = TaskState::Zombie;
        } else {
            self.pending_signals.push(signal);
        }
    }

    /// Check for pending signals and return the first one if any.
    /// Fatal signals are processed first.
    pub fn next_pending_signal(&mut self) -> Option<Signal> {
        if let Some(signal) = self.terminated_by_signal {
            return Some(signal);
        }
        self.pending_signals.pop()
    }

    /// Process pending signals: handle fatal ones immediately, others can be handled by user code.
    /// Returns `true` if a fatal signal terminated the task.
    pub fn process_pending_signals(&mut self) -> bool {
        while let Some(signal) = self.next_pending_signal() {
            if signal.is_fatal() {
                self.terminated_by_signal = Some(signal);
                self.exit_code = signal.exit_code();
                self.state = TaskState::Zombie;
                return true;
            }
        }
        false
    }
}

struct FdTable {
    slots: Vec<Option<FdEntry>>,
}

struct FdEntry {
    descriptor: FileDescriptor,
    file: OpenFile,
}

impl Clone for Task {
    fn clone(&self) -> Self {
        fn rebase_stack_pointer(
            value: u64,
            old_base: usize,
            old_top: usize,
            new_base: usize,
        ) -> u64 {
            let ptr = value as usize;
            if ptr >= old_base && ptr <= old_top {
                let offset = ptr - old_base;
                return (new_base + offset) as u64;
            }
            value
        }

        let (old_base, old_top) = self.kernel_stack_range();
        let kernel_stack = self.kernel_stack.clone();
        let new_base = kernel_stack.as_ptr() as usize;
        let kernel_stack_top = (kernel_stack.as_ptr() as usize + kernel_stack.len()) & !0xf;
        let mut context = self.context;
        context.rsp = rebase_stack_pointer(context.rsp, old_base, old_top, new_base);
        context.rbp = rebase_stack_pointer(context.rbp, old_base, old_top, new_base);
        context.kernel_rsp = rebase_stack_pointer(context.kernel_rsp, old_base, old_top, new_base);
        Self {
            pid: self.pid,
            parent_pid: self.parent_pid,
            cwd_inode: self.cwd_inode,
            cwd_path: self.cwd_path.clone(),
            cwd: Arc::clone(&self.cwd),
            state: self.state,
            mode: self.mode,
            name: self.name,
            kernel_stack_top,
            kernel_stack,
            context,
            priority: self.priority,
            credentials: self.credentials.clone(),
            user_stack_top: self.user_stack_top,
            fs_base: self.fs_base,
            gs_base: self.gs_base,
            brk_start: self.brk_start,
            brk_end: self.brk_end,
            brk_mapped_end: self.brk_mapped_end,
            mmap_start: self.mmap_start,
            mmap_end: self.mmap_end,
            userland: self.userland,
            exec_path: self.exec_path.clone(),
            time_usage_ms: self.time_usage_ms,
            start_time_ms: self.start_time_ms,
            fds: self.fds.clone(),
            env: Arc::clone(&self.env), // CoW: cheap clone
            argv: self.argv.clone(),
            exit_code: self.exit_code,
            address_space: self.address_space.clone(),
            #[cfg(feature = "drivers")]
            controlling_terminal: self.controlling_terminal,
            pending_signals: Vec::new(), // New task starts with no pending signals
            terminated_by_signal: None,
        }
    }
}

impl FdTable {
    fn new() -> Self {
        Self { slots: Vec::new() }
    }

    fn insert(&mut self, file: OpenFile, mode: FileOpenMode) -> usize {
        let descriptor = descriptor_for(file.inode().as_ref(), mode);
        let entry = FdEntry { descriptor, file };

        for fd in 3..self.slots.len() {
            if self.slots[fd].is_none() {
                self.slots[fd] = Some(entry);
                return fd;
            }
        }

        let fd = self.slots.len().max(3);
        if self.slots.len() < fd {
            self.slots.resize_with(fd, || None);
        }
        self.slots.push(Some(entry));
        fd
    }

    fn insert_at(&mut self, fd: usize, file: OpenFile, mode: FileOpenMode) {
        if fd >= self.slots.len() {
            self.slots.resize_with(fd + 1, || None);
        }
        let descriptor = descriptor_for(file.inode().as_ref(), mode);
        self.slots[fd] = Some(FdEntry { descriptor, file });
    }

    fn remove(&mut self, fd: usize) -> Option<OpenFile> {
        if fd >= self.slots.len() {
            return None;
        }
        self.slots[fd].take().map(|entry| entry.file)
    }

    fn iter(&self) -> impl Iterator<Item = (usize, &FdEntry)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(fd, slot)| slot.as_ref().map(|entry| (fd, entry)))
    }

    fn descriptor(&self, fd: usize) -> Option<FileDescriptor> {
        self.slots
            .get(fd)
            .and_then(|slot| slot.as_ref().map(|entry| entry.descriptor))
    }
}

impl Clone for FdTable {
    fn clone(&self) -> Self {
        let mut cloned = Self {
            slots: Vec::with_capacity(self.slots.len()),
        };

        for slot in &self.slots {
            let cloned_slot = slot.as_ref().and_then(|entry| clone_fd_entry(entry));
            cloned.slots.push(cloned_slot);
        }

        cloned
    }
}

fn clone_fd_entry(entry: &FdEntry) -> Option<FdEntry> {
    let inode = crate::fs::vfs::lookup_inode_by_tuple(
        entry.descriptor.fs_id,
        entry.descriptor.major,
        entry.descriptor.minor,
        entry.descriptor.inode,
    )?;
    let mut cloned_file = OpenFile::new(inode, None).ok()?;
    let _ = cloned_file.seek(SeekFrom::Start(entry.file.position()));
    Some(FdEntry {
        descriptor: entry.descriptor,
        file: cloned_file,
    })
}

#[cfg(feature = "drivers")]
fn controlling_terminal_device_path(target: ControllingTerminal) -> &'static str {
    match target {
        ControllingTerminal::VirtualConsole(0) => "/dev/tty0",
        ControllingTerminal::VirtualConsole(1) => "/dev/tty1",
        ControllingTerminal::VirtualConsole(2) => "/dev/tty2",
        ControllingTerminal::VirtualConsole(3) => "/dev/tty3",
        ControllingTerminal::VirtualConsole(4) => "/dev/tty4",
        ControllingTerminal::VirtualConsole(5) => "/dev/tty5",
        ControllingTerminal::VirtualConsole(6) => "/dev/tty6",
        ControllingTerminal::VirtualConsole(7) => "/dev/tty7",
        ControllingTerminal::VirtualConsole(8) => "/dev/tty8",
        ControllingTerminal::VirtualConsole(9) => "/dev/tty9",
        ControllingTerminal::VirtualConsole(10) => "/dev/tty10",
        ControllingTerminal::VirtualConsole(11) => "/dev/tty11",
        ControllingTerminal::VirtualConsole(_) => "/dev/null",
        ControllingTerminal::Serial(0) => "/dev/ttyS0",
        ControllingTerminal::Serial(1) => "/dev/ttyS1",
        ControllingTerminal::Serial(_) => "/dev/null",
    }
}

fn descriptor_for(inode: &dyn Inode, mode: FileOpenMode) -> FileDescriptor {
    let (major, minor) = inode.device_numbers().unwrap_or((0, 0));
    FileDescriptor {
        fs_id: inode.fs_id(),
        major,
        minor,
        inode: inode.ino(),
        mode,
    }
}

const KERNEL_STACK_SIZE: usize = 32 * 1024;
