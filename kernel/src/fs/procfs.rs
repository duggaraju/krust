extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::arch::x86_64::__cpuid;

use crate::mm::stats;
use crate::module::traits::{KernelModule, KernelRegistry, ModuleError};
use crate::process::scheduler::SCHEDULER;
use crate::process::task::{Pid, TaskMode, TaskState};

use super::vfs::{
    copy_name_into, write_u64_decimal_into, DirCursor, DirEntry, FileSystem, FileType, FsError,
    Inode,
};

pub struct ProcFs;

#[cfg(feature = "process")]
pub struct ProcFsModule;

impl ProcFs {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ProcFs {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "process")]
impl KernelModule for ProcFsModule {
    fn name(&self) -> &str {
        "procfs"
    }

    fn version(&self) -> &str {
        "0.1.0"
    }

    fn description(&self) -> &str {
        "Process information filesystem"
    }

    fn init(&self, _registry: &dyn KernelRegistry) -> Result<(), ModuleError> {
        crate::fs::register_filesystem(Arc::new(ProcFs::new()))
            .map_err(|_| ModuleError::InitFailed)
    }

    fn cleanup(&self, _registry: &dyn KernelRegistry) -> Result<(), ModuleError> {
        crate::fs::unregister_filesystem("proc").map_err(|_| ModuleError::CleanupFailed)
    }
}

#[cfg(feature = "process")]
pub fn register_module(registry: &dyn KernelRegistry) {
    let module: Arc<dyn KernelModule> = Arc::new(ProcFsModule);
    let _ = registry.register_module(module);
}

impl FileSystem for ProcFs {
    fn name(&self) -> &str {
        "proc"
    }

    fn root_inode(&self) -> Arc<dyn Inode> {
        Arc::new(ProcRootInode)
    }

    fn readdir(
        &self,
        inode: &dyn Inode,
        cursor: &mut DirCursor,
        name_buf: &mut [u8],
        visit: &mut dyn for<'a> FnMut(DirEntry<'a>) -> bool,
    ) -> Result<usize, FsError> {
        inode.readdir(cursor, name_buf, visit)
    }
}

struct ProcRootInode;

struct ProcCpuInfoInode;
struct ProcMemInfoInode;
struct ProcModulesDirInode;

#[cfg(feature = "modules")]
struct ProcModuleInode {
    name: String,
}

struct ProcTaskDirInode {
    pid: Pid,
}

struct ProcTaskStatusInode {
    pid: Pid,
}

struct ProcTaskCmdlineInode {
    pid: Pid,
}

struct ProcTaskCwdInode {
    pid: Pid,
}

struct ProcTaskStatInode {
    pid: Pid,
}

struct ProcTaskFdDirInode {
    pid: Pid,
}

struct ProcTaskFdLinkInode {
    pid: Pid,
    fd: usize,
}

impl Inode for ProcRootInode {
    fn read(&self, _offset: usize, _buf: &mut [u8]) -> Result<usize, FsError> {
        Err(FsError::IsADirectory)
    }

    fn write(&self, _offset: usize, _buf: &[u8]) -> Result<usize, FsError> {
        Err(FsError::IsADirectory)
    }

    fn ino(&self) -> u64 {
        PROC_ROOT_INO
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>, FsError> {
        match name {
            "cpuinfo" => return Ok(Arc::new(ProcCpuInfoInode)),
            "meminfo" => return Ok(Arc::new(ProcMemInfoInode)),
            "modules" => return Ok(Arc::new(ProcModulesDirInode)),
            _ => {}
        }

        let pid: Pid = name.parse().map_err(|_| FsError::NotFound)?;
        let scheduler = SCHEDULER.lock();
        let Some(scheduler) = scheduler.as_ref() else {
            return Err(FsError::NotFound);
        };
        if scheduler.task_by_pid(pid).is_none() {
            return Err(FsError::NotFound);
        }

        Ok(Arc::new(ProcTaskDirInode { pid }))
    }

    fn create(&self, _name: &str, _file_type: FileType) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotSupported)
    }

    fn file_type(&self) -> FileType {
        FileType::Directory
    }

    fn size(&self) -> usize {
        let scheduler = SCHEDULER.lock();
        scheduler
            .as_ref()
            .map_or(3, |sched| sched.tasks().count() + 3)
    }

    fn filesystem_name(&self) -> &'static str {
        "proc"
    }

    fn readdir(
        &self,
        cursor: &mut DirCursor,
        name_buf: &mut [u8],
        visit: &mut dyn for<'a> FnMut(DirEntry<'a>) -> bool,
    ) -> Result<usize, FsError> {
        let task_pids: Vec<Pid> = {
            let scheduler = SCHEDULER.lock();
            scheduler
                .as_ref()
                .map_or(Vec::new(), |sched| sched.tasks().map(|task| task.pid).collect())
        };
        let mut emitted = 0usize;
        let mut index = cursor.offset as usize;

        while index < 3 {
            let (name, file_type, ino) = match index {
                0 => ("cpuinfo", FileType::Regular, PROC_CPUINFO_INO),
                1 => ("meminfo", FileType::Regular, PROC_MEMINFO_INO),
                _ => ("modules", FileType::Directory, PROC_MODULES_INO),
            };
            let name = copy_name_into(name, name_buf)?;
            emitted += 1;
            index += 1;
            if !visit(DirEntry::new(name, file_type, ino)) {
                cursor.offset = index as u64;
                return Ok(emitted);
            }
        }

        for pid in task_pids.into_iter().skip(index.saturating_sub(3)) {
            let name = write_u64_decimal_into(pid as u64, name_buf)?;
            let entry = DirEntry::new(name, FileType::Directory, PROC_TASK_BASE_INO + pid);
            emitted += 1;
            index += 1;
            if !visit(entry) {
                cursor.offset = index as u64;
                return Ok(emitted);
            }
        }

        cursor.offset = index as u64;
        Ok(emitted)
    }
}

impl Inode for ProcCpuInfoInode {
    fn read(&self, offset: usize, buf: &mut [u8]) -> Result<usize, FsError> {
        let content = cpuinfo_content();
        read_text(&content, offset, buf)
    }

    fn write(&self, _offset: usize, _buf: &[u8]) -> Result<usize, FsError> {
        Err(FsError::NotSupported)
    }

    fn ino(&self) -> u64 {
        PROC_CPUINFO_INO
    }

    fn lookup(&self, _name: &str) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotADirectory)
    }

    fn create(&self, _name: &str, _file_type: FileType) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotADirectory)
    }

    fn file_type(&self) -> FileType {
        FileType::Regular
    }

    fn size(&self) -> usize {
        cpuinfo_content().len()
    }

    fn filesystem_name(&self) -> &'static str {
        "proc"
    }
}

impl Inode for ProcMemInfoInode {
    fn read(&self, offset: usize, buf: &mut [u8]) -> Result<usize, FsError> {
        let content = meminfo_content();
        read_text(&content, offset, buf)
    }

    fn write(&self, _offset: usize, _buf: &[u8]) -> Result<usize, FsError> {
        Err(FsError::NotSupported)
    }

    fn ino(&self) -> u64 {
        PROC_MEMINFO_INO
    }

    fn lookup(&self, _name: &str) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotADirectory)
    }

    fn create(&self, _name: &str, _file_type: FileType) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotADirectory)
    }

    fn file_type(&self) -> FileType {
        FileType::Regular
    }

    fn size(&self) -> usize {
        meminfo_content().len()
    }

    fn filesystem_name(&self) -> &'static str {
        "proc"
    }
}

impl Inode for ProcModulesDirInode {
    fn read(&self, _offset: usize, _buf: &mut [u8]) -> Result<usize, FsError> {
        Err(FsError::IsADirectory)
    }

    fn write(&self, _offset: usize, _buf: &[u8]) -> Result<usize, FsError> {
        Err(FsError::IsADirectory)
    }

    fn ino(&self) -> u64 {
        PROC_MODULES_INO
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>, FsError> {
        let _ = name;
        #[cfg(feature = "modules")]
        {
            let registry = crate::module::registry::MODULE_REGISTRY.lock();
            if registry.get(name).is_some() {
                return Ok(Arc::new(ProcModuleInode {
                    name: String::from(name),
                }));
            }
            return Err(FsError::NotFound);
        }

        #[cfg(not(feature = "modules"))]
        {
            Err(FsError::NotFound)
        }
    }

    fn create(&self, _name: &str, _file_type: FileType) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotSupported)
    }

    fn file_type(&self) -> FileType {
        FileType::Directory
    }

    fn size(&self) -> usize {
        #[cfg(feature = "modules")]
        {
            return crate::module::registry::MODULE_REGISTRY
                .lock()
                .loaded_modules()
                .len();
        }

        #[cfg(not(feature = "modules"))]
        {
            0
        }
    }

    fn filesystem_name(&self) -> &'static str {
        "proc"
    }

    fn readdir(
        &self,
        cursor: &mut DirCursor,
        name_buf: &mut [u8],
        visit: &mut dyn for<'a> FnMut(DirEntry<'a>) -> bool,
    ) -> Result<usize, FsError> {
        #[cfg(feature = "modules")]
        {
            let registry = crate::module::registry::MODULE_REGISTRY.lock();
            let modules = registry.loaded_modules();
            let mut emitted = 0usize;
            let mut index = cursor.offset as usize;
            while index < modules.len() {
                let name = copy_name_into(modules[index], name_buf)?;
                let entry =
                    DirEntry::new(name, FileType::Regular, PROC_MODULE_BASE_INO + hash_name(modules[index]));
                emitted += 1;
                index += 1;
                if !visit(entry) {
                    cursor.offset = index as u64;
                    return Ok(emitted);
                }
            }
            cursor.offset = index as u64;
            return Ok(emitted);
        }

        #[cfg(not(feature = "modules"))]
        {
            let _ = (cursor, name_buf, visit);
            Ok(0)
        }
    }
}

#[cfg(feature = "modules")]
impl Inode for ProcModuleInode {
    fn read(&self, offset: usize, buf: &mut [u8]) -> Result<usize, FsError> {
        #[cfg(feature = "modules")]
        {
            let registry = crate::module::registry::MODULE_REGISTRY.lock();
            let Some(module) = registry.get(&self.name) else {
                return Err(FsError::NotFound);
            };

            let content = format!(
                "name: {}\nversion: {}\ndescription: {}\n",
                module.name(),
                module.version(),
                module.description()
            );
            return read_text(&content, offset, buf);
        }

        #[cfg(not(feature = "modules"))]
        {
            let content =
                "name: unknown\nversion: unknown\ndescription: modules feature disabled\n";
            read_text(content, offset, buf)
        }
    }

    fn write(&self, _offset: usize, _buf: &[u8]) -> Result<usize, FsError> {
        Err(FsError::NotSupported)
    }

    fn ino(&self) -> u64 {
        PROC_MODULE_BASE_INO + hash_name(&self.name)
    }

    fn lookup(&self, _name: &str) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotADirectory)
    }

    fn create(&self, _name: &str, _file_type: FileType) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotADirectory)
    }

    fn file_type(&self) -> FileType {
        FileType::Regular
    }

    fn size(&self) -> usize {
        #[cfg(feature = "modules")]
        {
            let registry = crate::module::registry::MODULE_REGISTRY.lock();
            let Some(module) = registry.get(&self.name) else {
                return 0;
            };
            return format!(
                "name: {}\nversion: {}\ndescription: {}\n",
                module.name(),
                module.version(),
                module.description()
            )
            .len();
        }

        #[cfg(not(feature = "modules"))]
        {
            "name: unknown\nversion: unknown\ndescription: modules feature disabled\n".len()
        }
    }
}

impl Inode for ProcTaskDirInode {
    fn read(&self, _offset: usize, _buf: &mut [u8]) -> Result<usize, FsError> {
        Err(FsError::IsADirectory)
    }

    fn write(&self, _offset: usize, _buf: &[u8]) -> Result<usize, FsError> {
        Err(FsError::IsADirectory)
    }

    fn ino(&self) -> u64 {
        PROC_TASK_BASE_INO + self.pid
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>, FsError> {
        ensure_task_exists(self.pid)?;
        match name {
            "status" => Ok(Arc::new(ProcTaskStatusInode { pid: self.pid })),
            "cmdline" => Ok(Arc::new(ProcTaskCmdlineInode { pid: self.pid })),
            "cwd" => Ok(Arc::new(ProcTaskCwdInode { pid: self.pid })),
            "stat" => Ok(Arc::new(ProcTaskStatInode { pid: self.pid })),
            "fd" => Ok(Arc::new(ProcTaskFdDirInode { pid: self.pid })),
            _ => Err(FsError::NotFound),
        }
    }

    fn create(&self, _name: &str, _file_type: FileType) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotSupported)
    }

    fn file_type(&self) -> FileType {
        FileType::Directory
    }

    fn size(&self) -> usize {
        5
    }

    fn filesystem_name(&self) -> &'static str {
        "proc"
    }

    fn readdir(
        &self,
        cursor: &mut DirCursor,
        name_buf: &mut [u8],
        visit: &mut dyn for<'a> FnMut(DirEntry<'a>) -> bool,
    ) -> Result<usize, FsError> {
        ensure_task_exists(self.pid)?;

        let entries = [
            ("status", FileType::Regular, PROC_TASK_STATUS_BASE_INO + self.pid),
            ("cmdline", FileType::Regular, PROC_TASK_CMDLINE_BASE_INO + self.pid),
            ("cwd", FileType::Symlink, PROC_TASK_CWD_BASE_INO + self.pid),
            ("stat", FileType::Regular, PROC_TASK_STAT_BASE_INO + self.pid),
            ("fd", FileType::Directory, PROC_TASK_FD_DIR_BASE_INO + self.pid),
        ];

        let mut emitted = 0usize;
        let mut index = cursor.offset as usize;
        while index < entries.len() {
            let (name, file_type, ino) = entries[index];
            let copied = copy_name_into(name, name_buf)?;
            emitted += 1;
            index += 1;
            if !visit(DirEntry::new(copied, file_type, ino)) {
                cursor.offset = index as u64;
                return Ok(emitted);
            }
        }

        cursor.offset = index as u64;
        Ok(emitted)
    }
}

impl Inode for ProcTaskStatusInode {
    fn read(&self, offset: usize, buf: &mut [u8]) -> Result<usize, FsError> {
        read_text(&task_status_content(self.pid)?, offset, buf)
    }

    fn write(&self, _offset: usize, _buf: &[u8]) -> Result<usize, FsError> {
        Err(FsError::NotSupported)
    }

    fn ino(&self) -> u64 {
        PROC_TASK_STATUS_BASE_INO + self.pid
    }

    fn lookup(&self, _name: &str) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotADirectory)
    }

    fn create(&self, _name: &str, _file_type: FileType) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotADirectory)
    }

    fn file_type(&self) -> FileType {
        FileType::Regular
    }

    fn size(&self) -> usize {
        task_status_content(self.pid).map_or(0, |content| content.len())
    }

    fn filesystem_name(&self) -> &'static str {
        "proc"
    }
}

impl Inode for ProcTaskCmdlineInode {
    fn read(&self, offset: usize, buf: &mut [u8]) -> Result<usize, FsError> {
        read_text(&task_cmdline_content(self.pid)?, offset, buf)
    }

    fn write(&self, _offset: usize, _buf: &[u8]) -> Result<usize, FsError> {
        Err(FsError::NotSupported)
    }

    fn ino(&self) -> u64 {
        PROC_TASK_CMDLINE_BASE_INO + self.pid
    }

    fn lookup(&self, _name: &str) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotADirectory)
    }

    fn create(&self, _name: &str, _file_type: FileType) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotADirectory)
    }

    fn file_type(&self) -> FileType {
        FileType::Regular
    }

    fn size(&self) -> usize {
        task_cmdline_content(self.pid).map_or(0, |content| content.len())
    }

    fn filesystem_name(&self) -> &'static str {
        "proc"
    }
}

impl Inode for ProcTaskCwdInode {
    fn read(&self, offset: usize, buf: &mut [u8]) -> Result<usize, FsError> {
        read_text(&task_cwd_target(self.pid)?, offset, buf)
    }

    fn write(&self, _offset: usize, _buf: &[u8]) -> Result<usize, FsError> {
        Err(FsError::NotSupported)
    }

    fn ino(&self) -> u64 {
        PROC_TASK_CWD_BASE_INO + self.pid
    }

    fn lookup(&self, _name: &str) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotADirectory)
    }

    fn create(&self, _name: &str, _file_type: FileType) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotADirectory)
    }

    fn file_type(&self) -> FileType {
        FileType::Symlink
    }

    fn size(&self) -> usize {
        task_cwd_target(self.pid).map_or(0, |target| target.len())
    }

    fn filesystem_name(&self) -> &'static str {
        "proc"
    }
}

impl Inode for ProcTaskStatInode {
    fn read(&self, offset: usize, buf: &mut [u8]) -> Result<usize, FsError> {
        read_text(&task_stat_content(self.pid)?, offset, buf)
    }

    fn write(&self, _offset: usize, _buf: &[u8]) -> Result<usize, FsError> {
        Err(FsError::NotSupported)
    }

    fn ino(&self) -> u64 {
        PROC_TASK_STAT_BASE_INO + self.pid
    }

    fn lookup(&self, _name: &str) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotADirectory)
    }

    fn create(&self, _name: &str, _file_type: FileType) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotADirectory)
    }

    fn file_type(&self) -> FileType {
        FileType::Regular
    }

    fn size(&self) -> usize {
        task_stat_content(self.pid).map_or(0, |content| content.len())
    }

    fn filesystem_name(&self) -> &'static str {
        "proc"
    }
}

impl Inode for ProcTaskFdDirInode {
    fn read(&self, _offset: usize, _buf: &mut [u8]) -> Result<usize, FsError> {
        Err(FsError::IsADirectory)
    }

    fn write(&self, _offset: usize, _buf: &[u8]) -> Result<usize, FsError> {
        Err(FsError::IsADirectory)
    }

    fn ino(&self) -> u64 {
        PROC_TASK_FD_DIR_BASE_INO + self.pid
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>, FsError> {
        ensure_task_exists(self.pid)?;
        let fd: usize = name.parse().map_err(|_| FsError::NotFound)?;
        if task_fd_target(self.pid, fd).is_err() {
            return Err(FsError::NotFound);
        }
        Ok(Arc::new(ProcTaskFdLinkInode { pid: self.pid, fd }))
    }

    fn create(&self, _name: &str, _file_type: FileType) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotSupported)
    }

    fn file_type(&self) -> FileType {
        FileType::Directory
    }

    fn size(&self) -> usize {
        task_fd_entries(self.pid).map_or(0, |entries| entries.len())
    }

    fn filesystem_name(&self) -> &'static str {
        "proc"
    }

    fn readdir(
        &self,
        cursor: &mut DirCursor,
        name_buf: &mut [u8],
        visit: &mut dyn for<'a> FnMut(DirEntry<'a>) -> bool,
    ) -> Result<usize, FsError> {
        let fds = task_fd_entries(self.pid)?;

        let mut emitted = 0usize;
        let mut index = cursor.offset as usize;
        while index < fds.len() {
            let fd_name = write_u64_decimal_into(fds[index].0 as u64, name_buf)?;
            let ino = PROC_TASK_FD_LINK_BASE_INO + (self.pid * 1024) + (fds[index].0 as u64);
            let entry = DirEntry::new(fd_name, FileType::Symlink, ino);
            emitted += 1;
            index += 1;
            if !visit(entry) {
                cursor.offset = index as u64;
                return Ok(emitted);
            }
        }

        cursor.offset = index as u64;
        Ok(emitted)
    }
}

impl Inode for ProcTaskFdLinkInode {
    fn read(&self, offset: usize, buf: &mut [u8]) -> Result<usize, FsError> {
        read_text(&task_fd_target(self.pid, self.fd)?, offset, buf)
    }

    fn write(&self, _offset: usize, _buf: &[u8]) -> Result<usize, FsError> {
        Err(FsError::NotSupported)
    }

    fn ino(&self) -> u64 {
        PROC_TASK_FD_LINK_BASE_INO + (self.pid * 1024) + (self.fd as u64)
    }

    fn lookup(&self, _name: &str) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotADirectory)
    }

    fn create(&self, _name: &str, _file_type: FileType) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotADirectory)
    }

    fn file_type(&self) -> FileType {
        FileType::Symlink
    }

    fn size(&self) -> usize {
        task_fd_target(self.pid, self.fd).map_or(0, |target| target.len())
    }

    fn filesystem_name(&self) -> &'static str {
        "proc"
    }
}

const PROC_ROOT_INO: u64 = 1_000;
const PROC_CPUINFO_INO: u64 = 1_001;
const PROC_MEMINFO_INO: u64 = 1_002;
const PROC_MODULES_INO: u64 = 1_003;
const PROC_TASK_BASE_INO: u64 = 10_000;
const PROC_TASK_STATUS_BASE_INO: u64 = 11_000;
const PROC_TASK_CMDLINE_BASE_INO: u64 = 12_000;
const PROC_TASK_CWD_BASE_INO: u64 = 13_000;
const PROC_TASK_STAT_BASE_INO: u64 = 14_000;
const PROC_TASK_FD_DIR_BASE_INO: u64 = 15_000;
const PROC_TASK_FD_LINK_BASE_INO: u64 = 100_000;
#[cfg(feature = "modules")]
const PROC_MODULE_BASE_INO: u64 = 20_000;

fn state_name(state: TaskState) -> &'static str {
    match state {
        TaskState::Ready => "Ready",
        TaskState::Running => "Running",
        TaskState::Blocked => "Blocked",
        TaskState::Zombie => "Zombie",
    }
}

fn mode_name(mode: TaskMode) -> &'static str {
    match mode {
        TaskMode::Kernel => "Kernel",
        TaskMode::User => "User",
    }
}

fn read_text(content: &str, offset: usize, buf: &mut [u8]) -> Result<usize, FsError> {
    let bytes = content.as_bytes();
    if offset >= bytes.len() {
        return Ok(0);
    }
    let n = core::cmp::min(buf.len(), bytes.len() - offset);
    buf[..n].copy_from_slice(&bytes[offset..offset + n]);
    Ok(n)
}

fn ensure_task_exists(pid: Pid) -> Result<(), FsError> {
    let scheduler = SCHEDULER.lock();
    let Some(scheduler) = scheduler.as_ref() else {
        return Err(FsError::NotFound);
    };
    if scheduler.task_by_pid(pid).is_none() {
        return Err(FsError::NotFound);
    }

    Ok(())
}

fn task_status_content(pid: Pid) -> Result<String, FsError> {
    let scheduler = SCHEDULER.lock();
    let Some(scheduler) = scheduler.as_ref() else {
        return Err(FsError::NotFound);
    };
    let Some(task) = scheduler.task_by_pid(pid) else {
        return Err(FsError::NotFound);
    };

    Ok(format!(
        "Name:\t{}\nState:\t{}\nMode:\t{}\nPid:\t{}\nPPid:\t{}\n",
        task.name,
        state_name(task.state),
        mode_name(task.mode),
        task.pid,
        task.parent_pid,
    ))
}

fn task_cmdline_content(pid: Pid) -> Result<String, FsError> {
    let scheduler = SCHEDULER.lock();
    let Some(scheduler) = scheduler.as_ref() else {
        return Err(FsError::NotFound);
    };
    let Some(task) = scheduler.task_by_pid(pid) else {
        return Err(FsError::NotFound);
    };

    Ok(format!("{}\n", task.name))
}

fn task_cwd_target(pid: Pid) -> Result<String, FsError> {
    let scheduler = SCHEDULER.lock();
    let Some(scheduler) = scheduler.as_ref() else {
        return Err(FsError::NotFound);
    };
    let Some(task) = scheduler.task_by_pid(pid) else {
        return Err(FsError::NotFound);
    };

    Ok(task.cwd_path.clone())
}

fn task_stat_content(pid: Pid) -> Result<String, FsError> {
    let scheduler = SCHEDULER.lock();
    let Some(scheduler) = scheduler.as_ref() else {
        return Err(FsError::NotFound);
    };
    let Some(task) = scheduler.task_by_pid(pid) else {
        return Err(FsError::NotFound);
    };

    let mut content = format!(
        "pid: {}\nppid: {}\ncwd_inode: {}\nname: {}\nstate: {}\nmode: {}\n",
        task.pid,
        task.parent_pid,
        task.cwd_inode,
        task.name,
        state_name(task.state),
        mode_name(task.mode),
    );

    for (fd, desc) in task.fd_entries() {
        content.push_str(&format!(
            "fd: {} dev: {}:{} ino: {} mode: {}\n",
            fd,
            desc.major,
            desc.minor,
            desc.inode,
            desc.mode.as_str(),
        ));
    }

    Ok(content)
}

fn task_fd_entries(
    pid: Pid,
) -> Result<Vec<(usize, crate::fs::vfs::FileDescriptor)>, FsError> {
    let scheduler = SCHEDULER.lock();
    let Some(scheduler) = scheduler.as_ref() else {
        return Err(FsError::NotFound);
    };
    let Some(task) = scheduler.task_by_pid(pid) else {
        return Err(FsError::NotFound);
    };

    Ok(task.fd_entries().collect())
}

fn task_fd_target(pid: Pid, fd: usize) -> Result<String, FsError> {
    let entries = task_fd_entries(pid)?;
    let Some((_, desc)) = entries.into_iter().find(|(entry_fd, _)| *entry_fd == fd) else {
        return Err(FsError::NotFound);
    };

    if super::vfs::lookup_inode_by_tuple(desc.major, desc.minor, desc.inode).is_some() {
        Ok(format!(
            "inode:{}:{}:{}",
            desc.major, desc.minor, desc.inode
        ))
    } else {
        Err(FsError::NotFound)
    }
}

fn cpuinfo_content() -> String {
    let vendor = vendor_string();
    let brand = brand_string();
    let family = cpu_family();
    let model = cpu_model();
    let stepping = cpu_stepping();
    let logical_cpus = logical_cpu_count().max(1);

    let mut out = String::new();
    for processor in 0..logical_cpus {
        out.push_str(&format!(
            "processor\t: {}\nvendor_id\t: {}\ncpu family\t: {}\nmodel\t\t: {}\nmodel name\t: {}\nstepping\t: {}\ncore id\t\t: {}\nphysical id\t: 0\nsiblings\t: {}\ncpu cores\t: {}\n\n",
            processor,
            vendor,
            family,
            model,
            brand,
            stepping,
            processor,
            logical_cpus,
            logical_cpus,
        ));
    }
    out
}

fn meminfo_content() -> String {
    let stats = stats::get().copied().unwrap_or(stats::MemoryStats {
        total_bytes: 0,
        usable_bytes: 0,
    });
    format!(
        "MemTotal:       {} kB\nMemFree:        {} kB\nMemAvailable:   {} kB\nBuffers:        0 kB\nCached:         0 kB\n",
        stats.total_bytes / 1024,
        stats.usable_bytes / 1024,
        stats.usable_bytes / 1024,
    )
}

fn vendor_string() -> String {
    let cpuid = __cpuid(0);
    let mut bytes = [0u8; 12];
    bytes[0..4].copy_from_slice(&cpuid.ebx.to_le_bytes());
    bytes[4..8].copy_from_slice(&cpuid.edx.to_le_bytes());
    bytes[8..12].copy_from_slice(&cpuid.ecx.to_le_bytes());
    String::from_utf8_lossy(&bytes)
        .trim_end_matches('\0')
        .to_string()
}

fn brand_string() -> String {
    let max_extended = __cpuid(0x8000_0000).eax;
    if max_extended < 0x8000_0004 {
        return "unknown".to_string();
    }

    let mut bytes = [0u8; 48];
    for (i, leaf) in [0x8000_0002, 0x8000_0003, 0x8000_0004].iter().enumerate() {
        let cpuid = __cpuid(*leaf);
        let base = i * 16;
        bytes[base..base + 4].copy_from_slice(&cpuid.eax.to_le_bytes());
        bytes[base + 4..base + 8].copy_from_slice(&cpuid.ebx.to_le_bytes());
        bytes[base + 8..base + 12].copy_from_slice(&cpuid.ecx.to_le_bytes());
        bytes[base + 12..base + 16].copy_from_slice(&cpuid.edx.to_le_bytes());
    }
    String::from_utf8_lossy(&bytes)
        .trim_end_matches('\0')
        .trim()
        .to_string()
}

fn cpu_family() -> u32 {
    let cpuid = __cpuid(1);
    let base_family = (cpuid.eax >> 8) & 0xF;
    let ext_family = (cpuid.eax >> 20) & 0xFF;
    if base_family == 0xF {
        base_family + ext_family
    } else {
        base_family
    }
}

fn cpu_model() -> u32 {
    let cpuid = __cpuid(1);
    let base_model = (cpuid.eax >> 4) & 0xF;
    let ext_model = (cpuid.eax >> 16) & 0xF;
    let family = (cpuid.eax >> 8) & 0xF;
    if family == 0x6 || family == 0xF {
        (ext_model << 4) | base_model
    } else {
        base_model
    }
}

fn cpu_stepping() -> u32 {
    __cpuid(1).eax & 0xF
}

fn logical_cpu_count() -> usize {
    let cpuid = __cpuid(1);
    ((cpuid.ebx >> 16) & 0xFF) as usize
}

#[cfg(feature = "modules")]
fn hash_name(name: &str) -> u64 {
    let mut hash = 0u64;
    for byte in name.bytes() {
        hash = hash.wrapping_mul(131).wrapping_add(u64::from(byte));
    }
    hash
}
