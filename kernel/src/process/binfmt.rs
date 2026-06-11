extern crate alloc;

use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use bitflags::bitflags;
use spin::Mutex;
use xmas_elf::program::{SegmentData, Type as ProgramType};
use xmas_elf::ElfFile;

const PROBE_READ_LIMIT: usize = 512;

#[derive(Debug, Clone)]
pub struct ExecRequest {
    pub path: String,
    pub argv: Vec<String>,
    pub envp: Vec<String>,
}

impl ExecRequest {
    pub fn from_path(path: impl Into<String>) -> Self {
        let path = path.into();
        Self {
            argv: vec![path.clone()],
            envp: Vec::new(),
            path,
        }
    }
}

#[derive(Debug, Clone)]
pub enum BinaryFormatAction {
    Load(LoadPlan),
    Redirect(ExecRequest),
}

#[derive(Debug, Clone)]
pub struct LoadPlan {
    pub format: &'static str,
    pub entry_point: usize,
    pub segments: Vec<LoadSegment>,
    pub interpreter: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LoadSegment {
    pub virtual_address: usize,
    pub memory_size: usize,
    pub file_offset: usize,
    pub file_size: usize,
    pub flags: SegmentPermissions,
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct SegmentPermissions: u32 {
        const READ = 0b001;
        const WRITE = 0b010;
        const EXECUTE = 0b100;
    }
}

pub trait BinaryFormatHandler: Send + Sync {
    fn name(&self) -> &'static str;
    fn inspect(
        &self,
        request: &ExecRequest,
        data: &[u8],
    ) -> Result<Option<BinaryFormatAction>, BinfmtError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinfmtError {
    AlreadyRegistered,
    NotFound,
    NotExecutableFormat,
    InvalidFormat,
    Io(isize),
}

struct HandlerRegistry {
    handlers: Vec<Arc<dyn BinaryFormatHandler>>,
}

impl HandlerRegistry {
    const fn new() -> Self {
        Self { handlers: Vec::new() }
    }

    fn register(&mut self, handler: Arc<dyn BinaryFormatHandler>) -> Result<(), BinfmtError> {
        if self
            .handlers
            .iter()
            .any(|existing| existing.name() == handler.name())
        {
            return Err(BinfmtError::AlreadyRegistered);
        }
        self.handlers.push(handler);
        Ok(())
    }

    fn unregister(&mut self, name: &str) -> Result<(), BinfmtError> {
        let Some(index) = self.handlers.iter().position(|handler| handler.name() == name) else {
            return Err(BinfmtError::NotFound);
        };
        self.handlers.remove(index);
        Ok(())
    }

    fn clear_dynamic(&mut self) {
        self.handlers.retain(|handler| {
            let name = handler.name();
            name == ElfHandler::NAME || name == ShebangHandler::NAME
        });
    }
}

static BINFMT_HANDLERS: Mutex<HandlerRegistry> = Mutex::new(HandlerRegistry::new());

pub fn init() {
    let mut registry = BINFMT_HANDLERS.lock();
    registry.handlers.clear();
    let _ = registry.register(Arc::new(ShebangHandler));
    let _ = registry.register(Arc::new(ElfHandler));
    log::info!("registered built-in binfmt handlers: shebang, elf");
}

pub fn reset_dynamic_handlers() {
    BINFMT_HANDLERS.lock().clear_dynamic();
}

pub fn register_handler(handler: Arc<dyn BinaryFormatHandler>) -> Result<(), BinfmtError> {
    let name = handler.name();
    let result = BINFMT_HANDLERS.lock().register(handler);
    if result.is_ok() {
        log::info!("registered binfmt handler '{}'", name);
    }
    result
}

pub fn unregister_handler(name: &str) -> Result<(), BinfmtError> {
    let result = BINFMT_HANDLERS.lock().unregister(name);
    if result.is_ok() {
        log::info!("unregistered binfmt handler '{}'", name);
    }
    result
}

pub fn probe_path(request: &ExecRequest) -> Result<BinaryFormatAction, BinfmtError> {
    let bytes = crate::syscall::impls::read_file(request.path.as_str(), PROBE_READ_LIMIT)
        .map_err(BinfmtError::Io)?;
    let registry = BINFMT_HANDLERS.lock();
    for handler in &registry.handlers {
        if let Some(result) = handler.inspect(request, &bytes)? {
            return Ok(result);
        }
    }
    Err(BinfmtError::NotExecutableFormat)
}

struct ShebangHandler;

impl ShebangHandler {
    const NAME: &'static str = "shebang";
}

impl BinaryFormatHandler for ShebangHandler {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn inspect(
        &self,
        request: &ExecRequest,
        data: &[u8],
    ) -> Result<Option<BinaryFormatAction>, BinfmtError> {
        if data.len() < 2 || data[0] != b'#' || data[1] != b'!' {
            return Ok(None);
        }

        let line_end = data.iter().position(|byte| *byte == b'\n').unwrap_or(data.len());
        let line = core::str::from_utf8(&data[2..line_end])
            .map_err(|_| BinfmtError::InvalidFormat)?
            .trim();
        if line.is_empty() {
            return Err(BinfmtError::InvalidFormat);
        }

        let mut parts = line.split_whitespace();
        let interpreter = parts.next().ok_or(BinfmtError::InvalidFormat)?;
        let interpreter_arg = parts.next().map(ToString::to_string);

        let mut argv = Vec::new();
        argv.push(interpreter.to_string());
        if let Some(arg) = interpreter_arg {
            argv.push(arg);
        }
        argv.push(request.path.clone());
        if request.argv.len() > 1 {
            argv.extend(request.argv.iter().skip(1).cloned());
        }

        Ok(Some(BinaryFormatAction::Redirect(ExecRequest {
            path: interpreter.to_string(),
            argv,
            envp: request.envp.clone(),
        })))
    }
}

struct ElfHandler;

impl ElfHandler {
    const NAME: &'static str = "elf";
}

impl BinaryFormatHandler for ElfHandler {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn inspect(
        &self,
        _request: &ExecRequest,
        data: &[u8],
    ) -> Result<Option<BinaryFormatAction>, BinfmtError> {
        let elf = match ElfFile::new(data) {
            Ok(elf) => elf,
            Err(_) => return Ok(None),
        };

        let entry_point = elf.header.pt2.entry_point() as usize;
        let mut segments = Vec::new();
        let mut interpreter = None;

        for program_header in elf.program_iter() {
            match program_header.get_type() {
                Ok(ProgramType::Load) => {
                    segments.push(LoadSegment {
                        virtual_address: program_header.virtual_addr() as usize,
                        memory_size: program_header.mem_size() as usize,
                        file_offset: program_header.offset() as usize,
                        file_size: program_header.file_size() as usize,
                        flags: permissions_from_program_header(program_header.flags()),
                    });
                }
                Ok(ProgramType::Interp) => {
                    let interp_bytes = program_header
                        .get_data(&elf)
                        .map_err(|_| BinfmtError::InvalidFormat)?;
                    interpreter = match interp_bytes {
                        SegmentData::Undefined(bytes) => core::str::from_utf8(bytes)
                            .ok()
                            .map(|value| value.trim_end_matches('\0').to_string()),
                        _ => None,
                    };
                }
                _ => {}
            }
        }

        Ok(Some(BinaryFormatAction::Load(LoadPlan {
            format: Self::NAME,
            entry_point,
            segments,
            interpreter,
        })))
    }
}

fn permissions_from_program_header(flags: xmas_elf::program::Flags) -> SegmentPermissions {
    let mut permissions = SegmentPermissions::empty();
    if flags.is_read() {
        permissions |= SegmentPermissions::READ;
    }
    if flags.is_write() {
        permissions |= SegmentPermissions::WRITE;
    }
    if flags.is_execute() {
        permissions |= SegmentPermissions::EXECUTE;
    }
    permissions
}