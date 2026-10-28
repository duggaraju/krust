use nix::errno::Errno;
use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;
use regex::Regex;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const FIRMWARE: [&str; 2] = ["bios", "uefi"];
pub const RUNNER: &str = env!("CARGO_BIN_EXE_krust");

pub fn runner() -> PathBuf {
    fixture("KRUST_TEST_RUNNER", RUNNER)
}

pub fn matches(pattern: &str, text: &str) -> bool {
    Regex::new(pattern)
        .expect("valid test regex")
        .is_match(text)
}

pub fn checked(command: &mut Command) -> std::process::Output {
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("{command:?}: {error}"));
    assert!(
        output.status.success(),
        "{command:?}: {}\n{}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

pub fn digest(path: &Path) -> Vec<u8> {
    checked(Command::new("sha256sum").arg(path)).stdout
}

pub struct UnchangedDisk {
    path: PathBuf,
    before: Vec<u8>,
}

impl UnchangedDisk {
    pub fn new(path: &Path) -> Self {
        Self {
            path: path.into(),
            before: digest(path),
        }
    }
}

impl Drop for UnchangedDisk {
    fn drop(&mut self) {
        let output = Command::new("sha256sum").arg(&self.path).output();
        let unchanged = output
            .as_ref()
            .is_ok_and(|o| o.status.success() && o.stdout == self.before);
        if !unchanged {
            if thread::panicking() {
                eprintln!("Source VHD changed or unreadable: {}", self.path.display());
            } else {
                panic!("Source VHD changed or unreadable: {}", self.path.display());
            }
        }
    }
}

pub struct Workspace {
    pub output: PathBuf,
    pub scratch: PathBuf,
}

impl Workspace {
    pub fn new(name: &str) -> Self {
        assert!(name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-'));
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let parent = std::env::var_os("KRUST_TEST_OUTPUT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("target/cargo-tests/native")
            });
        fs::create_dir_all(&parent).expect("create test output parent");
        let output = loop {
            let path = parent.join(format!(
                "{name}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => break path,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("claim {}: {error}", path.display()),
            }
        };
        let scratch = output.join("scratch");
        fs::create_dir(&scratch).expect("create owned scratch directory");
        eprintln!("Test logs: {}", output.display());
        Self { output, scratch }
    }
    pub fn root(&self) -> PathBuf {
        self.scratch.join("root")
    }
    pub fn mkdisk(&self, format: &str) -> PathBuf {
        let disk = self.scratch.join(format!("{format}.vhd"));
        self.mkdisk_at(format, &disk, &[]);
        disk
    }
    pub fn mkdisk_at(&self, format: &str, disk: &Path, args: &[&std::ffi::OsStr]) {
        let result = checked(
            Command::new(runner())
                .arg("mkdisk")
                .arg(self.root())
                .arg(disk)
                .args(["--format", format])
                .args(args),
        );
        fs::write(
            self.output.join(format!("{format}-image.log")),
            result.stdout,
        )
        .unwrap();
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.scratch) {
            eprintln!("Cannot remove {}: {error}", self.scratch.display());
        }
    }
}

pub fn fixture(variable: &str, default: &str) -> PathBuf {
    let path = PathBuf::from(super::fixture(variable, default));
    fs::canonicalize(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

pub fn timeout() -> Duration {
    let text = std::env::var_os("KRUST_SMOKE_TIMEOUT_SECONDS")
        .map(|value| value.into_string().expect("ASCII timeout"))
        .unwrap_or_else(|| "120".into());
    parse_timeout(&text).expect("KRUST_SMOKE_TIMEOUT_SECONDS")
}

pub fn parse_timeout(text: &str) -> Result<Duration, String> {
    let error = || "must be an integer from 1 to 120 without leading zeros".to_string();
    let value = text.parse::<u64>().map_err(|_| error())?;
    if !(1..=120).contains(&value) || value.to_string() != text {
        return Err(error());
    }
    Ok(Duration::from_secs(value))
}

enum Event {
    Bytes { data: Vec<u8>, serial: bool },
    Closed,
    Error(std::io::Error),
}

pub struct Guest {
    child: Child,
    input: Option<ChildStdin>,
    events: Receiver<Event>,
    readers: Vec<JoinHandle<()>>,
    closed: usize,
    pending: Vec<u8>,
    pub console: String,
    trace: PathBuf,
    log: File,
    trace_log: PathBuf,
    deadline: Instant,
    trace_offset: usize,
}

impl Guest {
    pub fn id(&self) -> u32 {
        self.child.id()
    }
    pub fn start(
        work: &Workspace,
        label: &str,
        firmware: &str,
        disk: Option<&Path>,
        extra: &[&std::ffi::OsStr],
    ) -> Self {
        let mut command = Command::new(runner());
        command.args(["run", firmware, "--headless"]);
        if let Some(disk) = disk {
            command.arg("--vhd").arg(disk);
        } else {
            command.arg("--no-vhd");
        }
        command.args(extra).args(["--", "-snapshot", "-no-reboot"]);
        Self::spawn(work, label, command, timeout())
    }

    pub fn spawn(work: &Workspace, label: &str, mut command: Command, timeout: Duration) -> Self {
        let log = File::create(work.output.join(format!("{label}.log"))).unwrap();
        command
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .env("TMPDIR", &work.scratch)
            .env("LC_ALL", "C")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        let mut child = command
            .spawn()
            .unwrap_or_else(|error| panic!("{command:?}: {error}"));
        let input = child.stdin.take();
        let (sender, events) = mpsc::channel();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let readers = [Box::new(stdout) as Box<dyn Read + Send>, Box::new(stderr)]
            .into_iter()
            .enumerate()
            .map(|(index, mut stream)| {
                let sender = sender.clone();
                thread::spawn(move || {
                    let mut buffer = [0; 4096];
                    loop {
                        match stream.read(&mut buffer) {
                            Ok(0) => {
                                let _ = sender.send(Event::Closed);
                                break;
                            }
                            Ok(count) => {
                                if sender
                                    .send(Event::Bytes {
                                        data: buffer[..count].to_vec(),
                                        serial: index == 0,
                                    })
                                    .is_err()
                                {
                                    break;
                                }
                            }
                            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                                continue;
                            }
                            Err(error) => {
                                let _ = sender.send(Event::Error(error));
                                break;
                            }
                        }
                    }
                })
            })
            .collect();
        let trace = work.scratch.join(format!("krust-trace-{}.log", child.id()));
        Self {
            child,
            input,
            events,
            readers,
            closed: 0,
            pending: Vec::new(),
            console: String::new(),
            trace,
            log,
            trace_log: work.output.join(format!("{label}-kernel.log")),
            deadline: Instant::now() + timeout,
            trace_offset: 0,
        }
    }

    fn receive(&mut self) -> Result<(), String> {
        let remaining = self
            .deadline
            .checked_duration_since(Instant::now())
            .ok_or("guest timeout")?;
        match self
            .events
            .recv_timeout(remaining)
            .map_err(|error| format!("guest output timeout/disconnected: {error}"))?
        {
            Event::Bytes { data, serial } => {
                self.log.write_all(&data).map_err(|e| e.to_string())?;
                self.console.push_str(&String::from_utf8_lossy(&data));
                if serial {
                    self.pending.extend(data);
                }
            }
            Event::Closed => self.closed += 1,
            Event::Error(error) => return Err(format!("guest output read: {error}")),
        }
        Ok(())
    }

    pub fn until(&mut self, prompt: &str) -> Result<String, String> {
        loop {
            if let Some(position) = self
                .pending
                .windows(prompt.len())
                .position(|w| w == prompt.as_bytes())
            {
                let response = self
                    .pending
                    .drain(..position + prompt.len())
                    .collect::<Vec<_>>();
                return Ok(String::from_utf8_lossy(&response).into_owned());
            }
            if self.closed == 2 {
                return Err(format!("guest closed output before {prompt:?}"));
            }
            self.receive()?;
        }
    }
    pub fn boot(&mut self) {
        self.try_boot().expect("boot guest");
    }
    pub fn boot_for(&mut self, timeout: Duration) {
        let total_deadline = self.deadline;
        self.deadline = self.deadline.min(Instant::now() + timeout);
        let result = self.try_boot();
        self.deadline = total_deadline;
        result.expect("boot guest within command deadline");
    }
    pub fn try_boot(&mut self) -> Result<(), String> {
        let output = self.until("krust> ")?;
        if has_errors(&output) {
            return Err(format!("boot output: {output}"));
        }
        let trace = fs::read_to_string(&self.trace)
            .map_err(|error| format!("missing kernel trace: {error}"))?;
        if has_kernel_errors(&trace.replace('\r', "")) {
            return Err(format!("boot kernel trace: {trace}"));
        }
        Ok(())
    }
    pub fn send(&mut self, command: &str) {
        self.try_send(command).expect("write guest command");
    }
    fn try_send(&mut self, command: &str) -> Result<(), String> {
        assert!(!command.contains('\n'));
        // Interactive serial readers must receive the command and newline together.
        self.try_send_bytes(format!("{command}\n").as_bytes())
    }
    pub fn send_bytes(&mut self, bytes: &[u8]) {
        self.try_send_bytes(bytes).expect("write guest input");
    }
    fn try_send_bytes(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.trace_offset = fs::metadata(&self.trace)
            .map_err(|error| format!("kernel log: {error}"))?
            .len() as usize;
        self.input
            .as_mut()
            .expect("guest stdin")
            .write_all(bytes)
            .map_err(|error| format!("guest stdin: {error}"))
    }
    pub fn command(&mut self, command: &str) -> String {
        self.send(command);
        let raw = self
            .until("krust> ")
            .unwrap_or_else(|error| panic!("{command}: {error}\n{}", self.console));
        clean_response(&raw, command)
    }
    pub fn total_timeout(&mut self, timeout: Duration) {
        self.deadline = Instant::now() + timeout;
    }
    pub fn command_for(&mut self, command: &str, timeout: Duration) -> Result<String, String> {
        self.try_send(command)?;
        let total_deadline = self.deadline;
        self.deadline = self.deadline.min(Instant::now() + timeout);
        let response = self.until("krust> ");
        self.deadline = total_deadline;
        response.map(|raw| clean_response(&raw, command))
    }
    pub fn kernel(&self) -> String {
        fs::read_to_string(&self.trace)
            .unwrap_or_else(|error| panic!("{}: {error}", self.trace.display()))
            .replace('\r', "")
    }
    pub fn kernel_response(&self) -> String {
        let text = fs::read(&self.trace).expect("read kernel response");
        assert!(self.trace_offset <= text.len(), "kernel log truncated");
        String::from_utf8_lossy(&text[self.trace_offset..]).replace('\r', "")
    }
    pub fn finish(&mut self, expected: i32) -> ExitStatus {
        self.try_finish(expected)
            .unwrap_or_else(|error| panic!("{error}\n{}", self.console))
    }
    pub fn try_finish(&mut self, expected: i32) -> Result<ExitStatus, String> {
        self.input.take();
        while self.closed < 2 {
            self.receive()?;
        }
        let status = loop {
            if let Some(status) = self
                .child
                .try_wait()
                .map_err(|error| format!("wait guest: {error}"))?
            {
                break status;
            }
            if Instant::now() >= self.deadline {
                return Err("guest exit timeout".into());
            }
            thread::sleep(Duration::from_millis(10));
        };
        if status.code() != Some(expected) {
            return Err(format!("guest exit: {status}; expected {expected}"));
        }
        Ok(status)
    }
    pub fn shutdown(&mut self, command: &str) {
        self.try_shutdown(command).expect("clean guest shutdown");
    }
    pub fn try_shutdown(&mut self, command: &str) -> Result<(), String> {
        self.send(command);
        self.try_finish(0)?;
        let raw = String::from_utf8_lossy(&self.pending);
        if !clean_response(&raw, command).trim().is_empty() {
            return Err(format!("unexpected shutdown output: {raw}"));
        }
        Ok(())
    }
}

impl Drop for Guest {
    fn drop(&mut self) {
        let pid = Pid::from_raw(self.child.id() as i32);
        for signal in [Signal::SIGTERM, Signal::SIGKILL] {
            match killpg(pid, signal) {
                Ok(()) => thread::sleep(Duration::from_millis(50)),
                Err(Errno::ESRCH) => {}
                Err(error) => eprintln!("Cannot signal owned guest group {pid}: {error}"),
            }
        }
        if let Err(error) = self.child.wait() {
            eprintln!("Cannot reap guest: {error}");
        }
        for reader in self.readers.drain(..) {
            if let Err(error) = reader.join() {
                eprintln!("Guest output reader failed: {error:?}");
            }
        }
        while let Ok(event) = self.events.try_recv() {
            if let Event::Bytes { data, .. } = event {
                if let Err(error) = self.log.write_all(&data) {
                    eprintln!("Cannot retain guest output: {error}");
                }
            }
        }
        if self.trace.exists() {
            if let Err(error) = fs::copy(&self.trace, &self.trace_log) {
                eprintln!("Cannot retain kernel trace: {error}");
            }
        }
    }
}

pub fn clean_response(raw: &str, command: &str) -> String {
    let text = raw.replace('\r', "");
    let text = Regex::new(r"\x1b\[[0-9;]*[mHJ]")
        .unwrap()
        .replace_all(&text, "");
    let text = text.strip_suffix("krust> ").unwrap_or(&text);
    let text = text.strip_prefix(&format!("{command}\n")).unwrap_or(text);
    text.trim_end_matches('\n').to_string()
}
pub fn has_errors(text: &str) -> bool {
    matches(
        r"EXCEPTION:|panicked|Kernel panic|KRUST_GUEST_FAIL|exited with status|errno|command not found",
        text,
    )
}
pub fn has_kernel_errors(text: &str) -> bool {
    text.lines()
        .filter(|line| {
            !matches(
                r"^ERROR: syscall failed: nr=[0-9]+ \([a-z0-9_]+\) errno=-[0-9]+$",
                line,
            )
        })
        .any(has_errors)
}

pub fn prepare_root(work: &Workspace) {
    fs::create_dir_all(work.root().join("bin")).unwrap();
    fs::create_dir_all(work.root().join("etc")).unwrap();
    fs::copy(
        fixture("KRUST_TEST_BUSYBOX", "target/rootfs/bin/busybox"),
        work.root().join("bin/busybox"),
    )
    .unwrap();
}

pub fn inject(disk: &Path, source: &Path, destination: &str) {
    let source = source.to_str().expect("UTF-8 debugfs input path");
    assert!(!source.contains(['\n', '\0']));
    let source = source.replace('\\', "\\\\").replace('"', "\\\"");
    for operation in [
        format!("rm {destination}"),
        format!("write \"{source}\" {destination}"),
        format!("set_inode_field {destination} mode 0100755"),
    ] {
        checked(
            Command::new("debugfs")
                .args(["-w", "-R", &operation])
                .arg(disk),
        );
    }
    let output = checked(
        Command::new("debugfs")
            .args(["-R", &format!("stat {destination}")])
            .arg(disk),
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("Type: regular"),
        "debugfs failed to install {destination}"
    );
}
