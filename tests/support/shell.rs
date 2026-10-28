use super::qemu::{
    Guest, UnchangedDisk, Workspace, clean_response, fixture, has_kernel_errors, matches,
};

#[derive(Clone, Debug)]
pub enum Check {
    Exact(String),
    Quiet,
    Clear,
    Help,
    Environment(String),
    Patterns(Vec<&'static str>),
    Lines(Vec<&'static str>),
    Uptime,
    Listing,
}

#[derive(Clone, Debug)]
pub struct Case {
    pub command: String,
    pub check: Check,
}

impl Case {
    pub fn new(command: impl Into<String>, check: Check) -> Self {
        Self {
            command: command.into(),
            check,
        }
    }
    pub fn exact(command: impl Into<String>, expected: impl Into<String>) -> Self {
        Self::new(command, Check::Exact(expected.into()))
    }
    pub fn validate(&self, raw: &str, last_ticks: &mut u64) -> Result<(), String> {
        let output = clean_response(raw, &self.command);
        if !matches!(self.check, Check::Exact(_))
            && matches(
                r"EXCEPTION:|panicked|KRUST_GUEST_FAIL|exited with status|errno|command not found|usage:|Is a directory|not enabled|failed to",
                &output,
            )
        {
            return Err(format!("{}: unexpected error: {output}", self.command));
        }
        let valid = match &self.check {
            Check::Exact(expected) => output == *expected,
            Check::Quiet => output.trim().is_empty(),
            Check::Clear => raw.contains("\x1b[2J\x1b[H") && output.trim().is_empty(),
            Check::Help => {
                output.lines().any(|l| l == "Available commands:")
                    && [
                        "help", "echo", "set", "env", "clear", "mem", "ps", "ls", "stat", "pwd",
                        "cat", "cd", "mkdir", "write", "lsdev", "lspci", "lsmod", "uptime",
                        "shutdown", "exit",
                    ]
                    .iter()
                    .all(|name| matches(&format!(r"(?m)^  {name} +.* - "), &output))
            }
            Check::Environment(expected) => {
                let key = expected.split_once('=').unwrap().0;
                output.lines().any(|line| line == expected)
                    && output
                        .lines()
                        .filter(|line| line.starts_with(&format!("{key}=")))
                        .count()
                        == 1
            }
            Check::Patterns(patterns) => patterns.iter().all(|pattern| matches(pattern, &output)),
            Check::Lines(lines) => lines
                .iter()
                .all(|expected| output.lines().any(|line| line == *expected)),
            Check::Listing => {
                output.lines().any(|l| l == "  note.txt")
                    && output.lines().any(|l| l == "  .hidden")
                    && !matches(r"(?m)^  (\./|\.\./)$", &output)
            }
            Check::Uptime => {
                let ticks = output
                    .strip_prefix("ticks since boot: ")
                    .and_then(|v| v.parse::<u64>().ok());
                if let Some(ticks) = ticks.filter(|ticks| *ticks > 0 && *ticks >= *last_ticks) {
                    *last_ticks = ticks;
                    true
                } else {
                    false
                }
            }
        };
        if valid {
            Ok(())
        } else {
            Err(format!(
                "{}: {:?}\nactual: {output:?}",
                self.command, self.check
            ))
        }
    }
}

fn negative(cases: &mut Vec<Case>, command: &str, error: &str, cwd: &str) {
    cases.push(Case::exact(command, error));
    cases.push(Case::exact(
        "echo KRUST_SHELL_RECOVERY_OK",
        "KRUST_SHELL_RECOVERY_OK",
    ));
    cases.push(Case::exact("pwd", cwd));
}

pub fn cases(ramfs: bool) -> Vec<Case> {
    use Check::*;
    let mut cases = vec![Case::new("cd /", Quiet), Case::exact("pwd", "/")];
    if !ramfs {
        cases.extend([
            Case::new("", Quiet), Case::new("help", Help),
            Case::exact("echo KRUST_SHELL_ECHO_OK", "KRUST_SHELL_ECHO_OK"), Case::new("echo", Quiet),
            Case::new("set KRUST_SHELL_VALUE=first value", Quiet),
            Case::new("env", Environment("KRUST_SHELL_VALUE=first value".into())),
            Case::new("set KRUST_SHELL_VALUE second value", Quiet),
            Case::new("env", Environment("KRUST_SHELL_VALUE=second value".into())),
            Case::new("set KRUST_SHELL_EMPTY=", Quiet),
            Case::new("env", Environment("KRUST_SHELL_EMPTY=".into())),
            Case::new("clear", Clear), Case::exact("echo KRUST_SHELL_CLEAR_OK", "KRUST_SHELL_CLEAR_OK"),
            Case::new("mem", Patterns(vec![r"(?m)^MemTotal: +[1-9][0-9]* kB$", r"(?m)^MemFree: +[0-9]+ kB$"])),
            Case::new("ps", Patterns(vec![r"(?m)^PID  PPID   STATE   NAME$", r"(?m)^[0-9]+ +[0-9]+ +[^ ]+ +.*shell"])),
            Case::new("lsdev", Lines(vec!["Registered devices:", "  console", "  null", "  sda", "  ttyS0"])),
            Case::new("lspci", Patterns(vec![r"(?m)^bus: pci0$", r"(?m)^  sda +[0-9a-f]{2}:[0-9a-f]{2}\.[0-7] class=01 subclass=06 prog_if=01 bar5=0x[0-9a-f]+$"])),
            Case::new("lsmod", Lines(vec!["Loaded modules:", "  procfs", "  devfs", "  pci-bus", "  sata-dev"])),
            Case::new("uptime", Uptime),
        ]);
        for (command, error) in [
            ("cd missing", "cd: errno -2"),
            ("cat missing", "cat: missing: errno -2"),
            (
                "krust-no-such-command",
                "krust-no-such-command: command not found",
            ),
        ] {
            negative(&mut cases, command, error, "/");
        }
        cases.extend([
            Case::new("env", Environment("KRUST_SHELL_VALUE=second value".into())),
            Case::new("uptime", Uptime),
            Case::exact("/bin/busybox echo KRUST_FORK_EXEC_OK", "KRUST_FORK_EXEC_OK"),
            Case::new("exit", Quiet),
        ]);
        return cases;
    }
    cases.extend([
        Case::exact(
            "mkdir /krust-shell-test",
            "created directory: /krust-shell-test",
        ),
        Case::new("cd /krust-shell-test", Quiet),
        Case::exact("pwd", "/krust-shell-test"),
        Case::exact("mkdir nested", "created directory: nested"),
        Case::new("cd ./nested", Quiet),
        Case::exact("pwd", "/krust-shell-test/nested"),
        Case::exact(
            "write note.txt shell round trip one",
            "wrote 20 bytes to note.txt",
        ),
        Case::exact("cat note.txt", "shell round trip one"),
        Case::exact(
            "write note.txt shell round trip two",
            "wrote 20 bytes to note.txt",
        ),
        Case::exact("cat ./note.txt", "shell round trip two"),
        Case::exact(
            "cat /krust-shell-test/nested/note.txt",
            "shell round trip two",
        ),
        Case::exact("write .hidden hidden-data", "wrote 11 bytes to .hidden"),
        Case::new("ls", Listing),
        Case::new("ls -a", Lines(vec!["  note.txt", "  .hidden"])),
        Case::new(
            "ls -l",
            Patterns(vec![
                r"(?m)^note.txt +perms=[rwx-]{9} ino=[1-9][0-9]* +size=20 +type=file$",
                r"(?m)^\.hidden +perms=[rwx-]{9} ino=[1-9][0-9]* +size=11 +type=file$",
            ]),
        ),
        Case::exact("ls note.txt", "note.txt (file, 20 bytes)"),
        Case::new(
            "stat note.txt",
            Patterns(vec![
                r"(?m)^note.txt +perms=[rwx-]{9} ino=[1-9][0-9]* +size=20 +type=file$",
            ]),
        ),
        Case::new(
            "stat .",
            Patterns(vec![
                r"(?m)^\. +perms=[rwx-]{9} ino=[1-9][0-9]* +size=[0-9]+ +type=dir$",
            ]),
        ),
        Case::new("cd ..", Quiet),
        Case::exact("pwd", "/krust-shell-test"),
        Case::exact("ls", "  nested/"),
        Case::new("cd nested/.", Quiet),
        Case::exact("pwd", "/krust-shell-test/nested"),
    ]);
    for (command, error) in [
        ("set", "usage: set <key=value> | set <key> <value>"),
        ("set =bad", "usage: set <key=value> | set <key> <value>"),
        ("cd", "usage: cd <path>"),
        ("cat", "usage: cat <path>"),
        ("stat", "usage: stat <path>"),
        ("mkdir", "usage: mkdir <path>"),
        ("write", "usage: write <path> <data>"),
        ("write note.txt", "usage: write <path> <data>"),
        ("cd missing", "cd: errno -2"),
        ("cd note.txt", "cd: errno -20"),
        ("cat missing", "cat: missing: errno -2"),
        ("cat .", "cat: .: Is a directory"),
        ("stat missing", "stat: errno -2"),
        ("ls missing", "ls: errno -2"),
        ("write missing/note.txt text", "write: errno -2"),
        (
            "krust-no-such-command",
            "krust-no-such-command: command not found",
        ),
    ] {
        negative(&mut cases, command, error, "/krust-shell-test/nested");
    }
    cases.extend([
        Case::new("cd /", Quiet),
        Case::exact("pwd", "/"),
        Case::exact("echo KRUST_SHELL_RAMFS_OK", "KRUST_SHELL_RAMFS_OK"),
        Case::new("exit", Quiet),
    ]);
    cases
}

pub fn run() {
    let disk = fixture("KRUST_TEST_DISK", "target/krust-disk-ext4.vhd");
    let _unchanged = UnchangedDisk::new(&disk);
    let work = Workspace::new("shell");
    for firmware in ["bios", "uefi"] {
        for ramfs in [false, true] {
            let label = format!("{firmware}-{}", if ramfs { "ramfs" } else { "disk" });
            let mut guest = Guest::start(
                &work,
                &label,
                firmware,
                if ramfs { None } else { Some(&disk) },
                &[],
            );
            guest.boot();
            let mut last_ticks = 0;
            let cases = cases(ramfs);
            for case in &cases[..cases.len() - 1] {
                guest.send(&case.command);
                let raw = guest.until("krust> ").expect("shell command prompt");
                case.validate(&raw, &mut last_ticks)
                    .unwrap_or_else(|error| panic!("{label}: {error}"));
                assert!(
                    !has_kernel_errors(&guest.kernel_response()),
                    "{}",
                    guest.kernel_response()
                );
            }
            guest.shutdown(if firmware == "bios" {
                "shutdown"
            } else {
                "exit"
            });
            assert!(!has_kernel_errors(&guest.kernel()), "{}", guest.kernel());
            if !ramfs {
                assert!(guest.kernel().contains("fork: parent_pid="));
            }
            eprintln!("{label}: {} shell cases passed", cases.len());
        }
    }
}
