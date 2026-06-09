extern crate alloc;

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use super::context::CpuContext;
use crate::fs::vfs::{File, FileDescriptor, FileOpenMode, Inode, OpenFile, SeekFrom};

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

pub struct Task {
    pub pid: Pid,
    pub parent_pid: Pid,
    pub cwd_inode: u64,
    pub cwd_path: String,
    cwd: Arc<dyn Inode>,
    pub state: TaskState,
    pub name: &'static str,
    pub kernel_stack_top: usize,
    pub context: CpuContext,
    fds: FdTable,
    /// The controlling terminal for this process. Set by the kernel when
    /// spawning shell/interactive tasks; `None` for background tasks.
    #[cfg(feature = "drivers")]
    controlling_terminal: Option<ControllingTerminal>,
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
        Self::new_with_terminal(
            pid,
            parent_pid,
            cwd,
            cwd_path,
            name,
            entry_point,
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
        #[cfg(feature = "drivers")] controlling_terminal: Option<ControllingTerminal>,
        #[cfg(not(feature = "drivers"))] _controlling_terminal: Option<()>,
    ) -> Self {
        let cwd_inode = cwd.ino();
        let mut context = CpuContext::new();

        #[cfg(target_arch = "x86_64")]
        {
            context.rip = entry_point as usize as u64;
        }

        Self {
            pid,
            parent_pid,
            cwd_inode,
            cwd_path,
            cwd,
            state: TaskState::Ready,
            name,
            kernel_stack_top: 0,
            context,
            fds: FdTable::new(),
            #[cfg(feature = "drivers")]
            controlling_terminal,
        }
        .with_default_stdio()
    }

    pub fn child_of(pid: Pid, parent: &Task, name: &'static str, entry_point: fn()) -> Self {
        Self::new_with_terminal(
            pid,
            parent.pid,
            Arc::clone(&parent.cwd),
            parent.cwd_path.clone(),
            name,
            entry_point,
            #[cfg(feature = "drivers")]
            parent.controlling_terminal,
            #[cfg(not(feature = "drivers"))]
            None,
        )
    }

    pub fn cwd(&self) -> Arc<dyn Inode> {
        Arc::clone(&self.cwd)
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

    fn with_default_stdio(mut self) -> Self {
        self.refresh_stdio();
        self
    }

    fn refresh_stdio(&mut self) {
        let target = self.default_stdio_path();
        let fallback = "/dev/null";

        if let Ok(file) = crate::fs::vfs::open_path(target)
            .or_else(|_| crate::fs::vfs::open_path(fallback))
        {
            self.fds.insert_at(0, file, FileOpenMode::ReadOnly);
        }
        if let Ok(file) = crate::fs::vfs::open_path(target)
            .or_else(|_| crate::fs::vfs::open_path(fallback))
        {
            self.fds.insert_at(1, file, FileOpenMode::WriteOnly);
        }
        if let Ok(file) = crate::fs::vfs::open_path(target)
            .or_else(|_| crate::fs::vfs::open_path(fallback))
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
        Self {
            pid: self.pid,
            parent_pid: self.parent_pid,
            cwd_inode: self.cwd_inode,
            cwd_path: self.cwd_path.clone(),
            cwd: Arc::clone(&self.cwd),
            state: self.state,
            name: self.name,
            kernel_stack_top: self.kernel_stack_top,
            context: self.context,
            fds: self.fds.clone(),
            #[cfg(feature = "drivers")]
            controlling_terminal: self.controlling_terminal,
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
        major,
        minor,
        inode: inode.ino(),
        mode,
    }
}
