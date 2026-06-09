extern crate alloc;

use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use spin::Mutex;

use super::ldisc::LineDiscipline;
use super::traits::{Device, DeviceError, DeviceType};

/// Maximum number of bytes that can be queued in either direction of a PTY pipe.
const PTY_BUF_SIZE: usize = 4096;

// ── Linux-compatible major/minor numbers ─────────────────────────────────────
// Manager (ptmN):    major 5,   minor 128 + index
// Subsidiary (ptsN): major 136, minor index
// ptmx allocator:    major 5,   minor 2  (registered in drivers/mod.rs)
pub const PTY_MANAGER_MAJOR: u16 = 5;
pub const PTY_MANAGER_MINOR_BASE: u16 = 128;
pub const PTY_SUB_MAJOR: u16 = 136;

// ── Shared bidirectional buffer ───────────────────────────────────────────────

struct PtyShared {
    /// Line discipline on the subsidiary end; processes canonical mode, echo, output translation.
    ldisc: LineDiscipline,
    /// Echo bytes from the ldisc that the manager should read back (keyboard echo).
    echo_to_mgr: VecDeque<u8>,
    /// Processed output bytes from the subsidiary, readable by the manager.
    sub_to_mgr: VecDeque<u8>,
}

impl PtyShared {
    fn new() -> Self {
        Self {
            ldisc: LineDiscipline::new(),
            echo_to_mgr: VecDeque::new(),
            sub_to_mgr: VecDeque::new(),
        }
    }
}

// ── Manager side ──────────────────────────────────────────────────────────────

/// The manager (master) side of a pseudo-terminal pair.
///
/// - `write` pushes data into the manager→subsidiary channel.
/// - `read`  drains data that the subsidiary pushed into the subsidiary→manager channel.
pub struct PtyManagerDevice {
    name: String,
    pub index: usize,
    shared: Arc<Mutex<PtyShared>>,
}

impl Device for PtyManagerDevice {
    fn name(&self) -> &str {
        &self.name
    }

    fn device_type(&self) -> DeviceType {
        DeviceType::Char
    }

    /// Manager reads: drain echo bytes first (keyboard echo), then any output
    /// that the subsidiary application sent.
    fn read(&self, _offset: usize, buf: &mut [u8]) -> Result<usize, DeviceError> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut shared = self.shared.lock();
        let mut n = 0usize;
        // echo bytes take priority
        while n < buf.len() {
            match shared.echo_to_mgr.pop_front() {
                Some(b) => { buf[n] = b; n += 1; }
                None => break,
            }
        }
        // then application output
        while n < buf.len() {
            match shared.sub_to_mgr.pop_front() {
                Some(b) => { buf[n] = b; n += 1; }
                None => break,
            }
        }
        Ok(n)
    }

    /// Manager writes: bytes are treated as raw terminal input and fed through
    /// the line discipline.  Any echo produced is queued back for the manager
    /// to read.  Cooked bytes become available for the subsidiary to read.
    fn write(&self, _offset: usize, buf: &[u8]) -> Result<usize, DeviceError> {
        let mut shared = self.shared.lock();
        for &byte in buf {
            let echo = shared.ldisc.process_input(byte);
            echo.emit(|echo_bytes| {
                let available = PTY_BUF_SIZE.saturating_sub(shared.echo_to_mgr.len());
                for &b in &echo_bytes[..echo_bytes.len().min(available)] {
                    shared.echo_to_mgr.push_back(b);
                }
            });
        }
        Ok(buf.len())
    }
}

// ── Subsidiary side ───────────────────────────────────────────────────────────

/// The subsidiary (slave) side of a pseudo-terminal pair.
///
/// - `write` pushes data into the subsidiary→manager channel.
/// - `read`  drains data that the manager pushed into the manager→subsidiary channel.
pub struct PtySubsidiaryDevice {
    name: String,
    pub index: usize,
    shared: Arc<Mutex<PtyShared>>,
}

impl Device for PtySubsidiaryDevice {
    fn name(&self) -> &str {
        &self.name
    }

    fn device_type(&self) -> DeviceType {
        DeviceType::Char
    }

    /// Subsidiary reads: return cooked bytes from the line discipline's read queue.
    fn read(&self, _offset: usize, buf: &mut [u8]) -> Result<usize, DeviceError> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut shared = self.shared.lock();
        Ok(shared.ldisc.read(buf))
    }

    /// Subsidiary writes: process through the output discipline (ONLCR etc.)
    /// before pushing bytes to the manager-readable channel.
    fn write(&self, _offset: usize, buf: &[u8]) -> Result<usize, DeviceError> {
        let mut shared = self.shared.lock();
        let mut processed = alloc::vec::Vec::new();
        shared.ldisc.process_output(buf, &mut processed);
        let available = PTY_BUF_SIZE.saturating_sub(shared.sub_to_mgr.len());
        let n = processed.len().min(available);
        for &byte in &processed[..n] {
            shared.sub_to_mgr.push_back(byte);
        }
        Ok(buf.len())
    }
}

// ── PTY table ─────────────────────────────────────────────────────────────────

struct PtyEntry {
    index: usize,
}

struct PtyTable {
    entries: Vec<PtyEntry>,
    next_index: usize,
}

impl PtyTable {
    const fn new() -> Self {
        Self {
            entries: Vec::new(),
            next_index: 0,
        }
    }

    fn alloc(&mut self) -> (usize, Arc<PtyManagerDevice>, Arc<PtySubsidiaryDevice>) {
        let index = self.next_index;
        self.next_index = self.next_index.saturating_add(1);

        let shared = Arc::new(Mutex::new(PtyShared::new()));
        let manager = Arc::new(PtyManagerDevice {
            name: format!("ptm{}", index),
            index,
            shared: Arc::clone(&shared),
        });
        let subsidiary = Arc::new(PtySubsidiaryDevice {
            name: format!("pts{}", index),
            index,
            shared,
        });

        self.entries.push(PtyEntry { index });
        (index, manager, subsidiary)
    }

    fn subsidiary_indices(&self) -> Vec<usize> {
        self.entries.iter().map(|e| e.index).collect()
    }

    fn count(&self) -> usize {
        self.entries.len()
    }
}

static PTY_TABLE: Mutex<PtyTable> = Mutex::new(PtyTable::new());

// ── Public API ────────────────────────────────────────────────────────────────

/// Allocate a new PTY pair, register both sides in the device registry, and
/// return the index of the pair.
///
/// - Manager device registered as `ptmN`  (major 5, minor 128+N).
/// - Subsidiary registered as `ptsN` (major 136, minor N).
pub fn alloc_pty() -> Result<usize, super::registry::RegistryError> {
    let (index, manager, subsidiary) = PTY_TABLE.lock().alloc();

    let mgr_name = format!("ptm{}", index);
    let sub_name = format!("pts{}", index);

    super::registry::register(
        &mgr_name,
        manager,
        PTY_MANAGER_MAJOR,
        PTY_MANAGER_MINOR_BASE + index as u16,
    )?;
    super::registry::register(&sub_name, subsidiary, PTY_SUB_MAJOR, index as u16)?;

    Ok(index)
}

/// Returns the indices of all currently allocated PTY subsidiary devices.
pub fn subsidiary_indices() -> Vec<usize> {
    PTY_TABLE.lock().subsidiary_indices()
}

/// Returns the total number of allocated PTY pairs.
pub fn pty_count() -> usize {
    PTY_TABLE.lock().count()
}
