use super::qemu::*;
use super::suites::{install, trace_syscalls};
use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::process::Command;
use std::time::Duration;

#[derive(Debug)]
pub enum Validation {
    Exact(&'static str),
    Pattern(&'static str),
    Listing,
    Processes,
    Network,
}

#[derive(Debug)]
pub struct Utility {
    pub name: String,
    pub command: &'static str,
    pub status: i32,
    pub validation: Validation,
}

pub fn case(name: &str) -> Result<Utility, String> {
    use Validation::*;
    const CONTENTS: &str = "alpha\nbeta\nalpha";
    let (command, validation) = match name {
        "sh" => ("/bin/sh -c pwd", Exact("/")),
        "ash" => ("/bin/ash -c pwd", Exact("/")),
        "bash" => ("/bin/bash -c pwd", Exact("/")),
        "ls" => ("/bin/ls /fixture", Listing),
        "cat" => ("/bin/cat /fixture/input", Exact(CONTENTS)),
        "echo" => ("/bin/echo utility echo", Exact("utility echo")),
        "cp" => (
            "/bin/cp /fixture/input /scratch/copied && /bin/cat /scratch/copied",
            Exact(CONTENTS),
        ),
        "mv" => (
            "/bin/mv /fixture/move /scratch/moved && /bin/cat /scratch/moved",
            Exact("move-data"),
        ),
        "rm" => (
            "/bin/rm /fixture/remove && test ! -e /fixture/remove",
            Exact(""),
        ),
        "mkdir" => (
            "/bin/mkdir /scratch/newdir && test -d /scratch/newdir",
            Exact(""),
        ),
        "rmdir" => (
            "/bin/rmdir /fixture/empty && test ! -d /fixture/empty",
            Exact(""),
        ),
        "ln" => (
            "/bin/ln -s /fixture/input /scratch/link && /bin/cat /scratch/link",
            Exact(CONTENTS),
        ),
        "touch" => (
            "/bin/touch /scratch/touched && test -f /scratch/touched",
            Exact(""),
        ),
        "grep" => ("/bin/grep beta /fixture/input", Exact("beta")),
        "sed" => (
            "/bin/sed 's/alpha/gamma/g' /fixture/input",
            Exact("gamma\nbeta\ngamma"),
        ),
        "awk" => ("/bin/awk '{print NR}' /fixture/input", Exact("1\n2\n3")),
        "cut" => ("/bin/cut -c 1 /fixture/input", Exact("a\nb\na")),
        "sort" => ("/bin/sort /fixture/input", Exact("alpha\nalpha\nbeta")),
        "uniq" => ("/bin/uniq /fixture/repeat", Exact("alpha\nbeta")),
        "head" => ("/bin/head -n 1 /fixture/input", Exact("alpha")),
        "tail" => ("/bin/tail -n 1 /fixture/input", Exact("alpha")),
        "wc" => (
            "/bin/wc -l /fixture/input",
            Pattern(r"^\s*3\s+/fixture/input$"),
        ),
        "find" => ("/bin/find /fixture/empty -type d", Exact("/fixture/empty")),
        "xargs" => ("/bin/xargs -a /fixture/one /bin/echo", Exact("one two")),
        "stat" => ("/bin/stat -c '%s' /fixture/input", Exact("17")),
        "chmod" => (
            "/bin/chmod 600 /fixture/mode && /bin/stat -c '%a' /fixture/mode",
            Exact("600"),
        ),
        "chown" => (
            "/bin/chown 123:456 /fixture/owner && /bin/stat -c '%u:%g' /fixture/owner",
            Exact("123:456"),
        ),
        "ps" => ("/bin/ps", Processes),
        "kill" => ("/bin/kill -0 $$", Exact("")),
        "env" => (
            "/bin/env UTILITY_ENV=kept /bin/printenv UTILITY_ENV",
            Exact("kept"),
        ),
        "printenv" => ("UTILITY_ENV=kept /bin/printenv UTILITY_ENV", Exact("kept")),
        "date" => ("/bin/date +%s", Pattern(r"^[0-9]+$")),
        "sleep" => ("/bin/sleep 0.01", Exact("")),
        "true" => ("/bin/true", Exact("")),
        "false" => ("/bin/false", Exact("")),
        "hexdump" => (
            "/bin/hexdump -v -e '1/1 \"%02x\"' /fixture/bytes",
            Exact("4142"),
        ),
        "od" => (
            "/bin/od -An -tx1 /fixture/bytes",
            Pattern(r"^\s*41\s+42\s*$"),
        ),
        "strings" => ("/bin/strings /fixture/strings", Exact("UTILITY_STRING")),
        "gzip" => (
            "/bin/gzip /fixture/zipin && /bin/gunzip -c /fixture/zipin.gz",
            Exact(CONTENTS),
        ),
        "gunzip" => ("/bin/gunzip -c /fixture/data.gz", Exact(CONTENTS)),
        "tar" => ("/bin/tar -tf /fixture/data.tar", Exact("input")),
        "wget" => (
            "/bin/wget -T 1 -t 1 -O /scratch/web http://127.0.0.1:9/",
            Network,
        ),
        _ => {
            return Err(format!(
                "No functional test defined for installed utility '{name}'"
            ));
        }
    };
    Ok(Utility {
        name: name.into(),
        command,
        status: if name == "false" { 1 } else { 0 },
        validation,
    })
}

impl Utility {
    pub fn validate(&self, output: &str, status: i32) -> Result<(), String> {
        if matches!(self.validation, Validation::Network) {
            return Err("wget needs a guest-reachable HTTP fixture".into());
        }
        let valid = match self.validation {
            Validation::Exact(expected) => output == expected,
            Validation::Pattern(pattern) => matches(pattern, output),
            Validation::Listing => [
                "input", "repeat", "one", "bytes", "strings", "data.gz", "data.tar", "empty",
            ]
            .iter()
            .all(|name| output.split_whitespace().any(|token| token == *name)),
            Validation::Processes => {
                output.split_whitespace().any(|token| token == "PID") && output.contains("busybox")
            }
            Validation::Network => unreachable!(),
        };
        if valid && status == self.status {
            Ok(())
        } else {
            Err(format!(
                "{} expected status {}, {:?}; got {status}: {output:?}",
                self.name, self.status, self.validation
            ))
        }
    }
    pub fn response(&self, output: &str) -> Result<(String, i32), String> {
        let markers = output
            .lines()
            .filter(|line| line.starts_with("KRUST_UTILITY_DONE:"))
            .collect::<Vec<_>>();
        if markers.len() != 1 {
            return Err(format!(
                "{}: missing/duplicate completion marker: {output}",
                self.name
            ));
        }
        let prefix = format!("KRUST_UTILITY_DONE:{}:", self.name);
        let status = markers[0]
            .strip_prefix(&prefix)
            .and_then(|v| v.parse::<u8>().ok())
            .ok_or_else(|| format!("{}: invalid completion marker: {}", self.name, markers[0]))?;
        let contents = output
            .lines()
            .filter(|line| *line != markers[0])
            .collect::<Vec<_>>()
            .join("\n");
        Ok((contents.trim_end_matches('\n').into(), i32::from(status)))
    }
}

pub fn installed(work: &Workspace, format: &str) -> Vec<String> {
    install(work, format);
    let mut names = fs::read_dir(work.root().join("bin"))
        .unwrap()
        .map(|entry| {
            entry
                .unwrap()
                .file_name()
                .into_string()
                .expect("ASCII applet name")
        })
        .filter(|name| name != "busybox")
        .collect::<Vec<_>>();
    names.sort();
    assert!(!names.is_empty(), "No installed utilities");
    for name in &names {
        case(name).unwrap();
    }
    names
}

fn prepare(work: &Workspace, format: &str, names: &[String]) {
    let root = work.root();
    for dir in ["fixture/empty", "cases", "etc"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    for (name, data) in [
        ("input", "alpha\nbeta\nalpha\n"),
        ("zipin", "alpha\nbeta\nalpha\n"),
        ("repeat", "alpha\nalpha\nbeta\n"),
        ("one", "one two\n"),
        ("bytes", "AB"),
        ("strings", "\0UTILITY_STRING\0"),
        ("move", "move-data\n"),
        ("remove", "remove-data\n"),
        ("mode", "mode-data\n"),
        ("owner", "owner-data\n"),
    ] {
        fs::write(root.join("fixture").join(name), data).unwrap();
    }
    let gzip = checked(
        Command::new("gzip")
            .args(["-n", "-c"])
            .arg(root.join("fixture/input")),
    );
    fs::write(root.join("fixture/data.gz"), gzip.stdout).unwrap();
    checked(
        Command::new("tar")
            .arg("-cf")
            .arg(root.join("fixture/data.tar"))
            .arg("-C")
            .arg(root.join("fixture"))
            .arg("input"),
    );
    fs::write(root.join("etc/fstab"), format!("/dev/sda / {format} defaults 0 0\nproc /proc proc defaults 0 0\ndev /dev dev defaults 0 0\nnone /scratch ramfs defaults 0 0\n")).unwrap();
    for name in names {
        let utility = case(name).unwrap();
        fs::write(
            root.join("cases").join(name),
            format!(
                "{}\nutility_status=$?\necho\necho KRUST_UTILITY_DONE:{name}:$utility_status\n",
                utility.command
            ),
        )
        .unwrap();
    }
}

pub fn run(format: &str, firmware: &str, selected: &[&str]) {
    assert!(["fat", "ext4"].contains(&format) && FIRMWARE.contains(&firmware));
    let work = Workspace::new(&format!("busybox-{format}-{firmware}"));
    let names = installed(&work, format);
    for name in selected {
        assert!(
            names.iter().any(|installed| installed == name),
            "Utility is not installed: {name}"
        );
    }
    let chosen = if selected.is_empty() {
        names.clone()
    } else {
        selected.iter().map(|s| s.to_string()).collect()
    };
    prepare(&work, format, &names);
    let disk = work.mkdisk(format);
    trace_syscalls(&disk);
    let _unchanged = UnchangedDisk::new(&disk);
    let mut report = fs::File::create(work.output.join("report.tsv")).unwrap();
    writeln!(
        report,
        "firmware\tutility\tresult\texpected_status\tactual_status\tlaunched"
    )
    .unwrap();
    let mut guest: Option<Guest> = None;
    let mut failures = Vec::new();
    let mut syscalls = BTreeSet::new();
    let mut unimplemented = BTreeSet::new();
    let syscall = regex::Regex::new(r"syscall entry:.*nr=([0-9]+) \(([^)]+)\)").unwrap();
    let mut boots = 0;
    for name in chosen {
        if guest.is_none() {
            let mut next =
                Guest::start(&work, &format!("boot-{boots}"), firmware, Some(&disk), &[]);
            next.total_timeout(Duration::from_secs(900));
            next.boot_for(Duration::from_secs(60));
            guest = Some(next);
            boots += 1;
        }
        let active = guest.as_mut().unwrap();
        let utility = case(&name).unwrap();
        let response = active.command_for(
            &format!("/bin/busybox sh /cases/{name}"),
            Duration::from_secs(60),
        );
        let trace = active.kernel_response();
        for captures in syscall.captures_iter(&trace) {
            syscalls.insert(format!("{} {}", &captures[1], &captures[2]));
        }
        for line in trace
            .lines()
            .filter(|line| line.contains("unimplemented syscall:"))
        {
            unimplemented.insert(line.to_string());
        }
        fs::write(work.output.join(format!("{name}-kernel.log")), &trace).unwrap();
        let launched = matches(
            &format!(r"execve: (loading|redirecting|symlink resolved) '/bin/{name}'"),
            &trace,
        );
        let mut actual = "missing".to_string();
        let result = match &response {
            Ok(output) => {
                fs::write(work.output.join(format!("{name}.log")), output).unwrap();
                utility.response(output).and_then(|(contents, status)| {
                    actual = status.to_string();
                    utility.validate(&contents, status)
                })
            }
            Err(error) => Err(format!("{name}: incomplete command: {error}")),
        };
        let result = result.and_then(|()| {
            if !launched {
                Err(format!("{name}: installed path was not executed"))
            } else if matches(r"Kernel panic|panicked|EXCEPTION:", &trace) {
                Err(format!("{name}: kernel fault: {trace}"))
            } else {
                Ok(())
            }
        });
        if let Err(error) = &result {
            eprintln!("{firmware}/{format}: {error}");
            failures.push(error.clone());
        }
        writeln!(
            report,
            "{firmware}\t{name}\t{}\t{}\t{actual}\t{}",
            if result.is_ok() { "PASS" } else { "FAIL" },
            utility.status,
            if launched { "yes" } else { "no" }
        )
        .unwrap();
        report.flush().unwrap();
        if response.is_err() {
            drop(guest.take());
        }
    }
    if let Some(mut guest) = guest {
        guest.total_timeout(Duration::from_secs(10));
        guest.shutdown("exit");
    }
    for (name, lines) in [
        ("syscalls.log", syscalls),
        ("unimplemented.log", unimplemented),
    ] {
        fs::write(
            work.output.join(name),
            lines
                .into_iter()
                .map(|line| format!("{line}\n"))
                .collect::<String>(),
        )
        .unwrap();
    }
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
