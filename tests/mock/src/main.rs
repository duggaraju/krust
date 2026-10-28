use std::fs;
use std::io::{self, BufRead, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

fn main() {
    let mode = std::env::args().nth(1).expect("mock scenario");
    if mode == "child" {
        thread::sleep(Duration::from_secs(300));
        return;
    }
    let scratch = std::path::PathBuf::from(std::env::var_os("TMPDIR").expect("owned scratch"));
    let trace = scratch.join(format!("krust-trace-{}.log", std::process::id()));
    if mode != "no-trace" {
        fs::write(&trace, "virtual_address_offset: 0x10000000000\nHandling Segment: type_: Ok(Load), virtual_addr: 1234,\nfork: parent_pid=1\n").unwrap();
    }
    if mode != "fragmented" {
        eprintln!("headless kernel trace log: {}", trace.display());
    }
    if mode == "kernel-error" {
        fs::OpenOptions::new()
            .append(true)
            .open(&trace)
            .unwrap()
            .write_all(b"ERROR: unexpected errno -1\n")
            .unwrap();
    }
    if mode == "linger-child" {
        let child = Command::new(std::env::current_exe().unwrap())
            .arg("child")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        fs::write(scratch.join("child.pid"), child.id().to_string()).unwrap();
    }
    if mode == "no-prompt" {
        thread::sleep(Duration::from_secs(300));
        return;
    }
    prompt(&mode);
    for line in io::stdin().lock().lines() {
        let command = line.unwrap();
        if mode == "command-timeout" {
            thread::sleep(Duration::from_secs(300));
        }
        if command == "exit" || command == "shutdown" {
            if mode == "closed-pipes-live" {
                // Simulate EOF without process exit, which must still hit the deadline.
                unsafe {
                    drop(OwnedFd::from_raw_fd(1));
                    drop(OwnedFd::from_raw_fd(2));
                }
                thread::sleep(Duration::from_secs(300));
            }
            if mode == "shutdown-error" {
                println!("errno -5");
            }
            std::process::exit(if mode == "exit-failure" { 7 } else { 0 });
        }
        println!("{command}");
        if mode != "echo-only" {
            match command.as_str() {
                "/bin/busybox pwd" => println!("/"),
                "/bin/busybox echo KRUST_ECHO_OK" => println!("KRUST_ECHO_OK"),
                _ => panic!("unexpected mock command: {command}"),
            }
        }
        prompt(&mode);
    }
}

fn prompt(mode: &str) {
    if mode == "fragmented" {
        for byte in b"krust> " {
            io::stdout().write_all(&[*byte]).unwrap();
            io::stdout().flush().unwrap();
            eprintln!("mock stderr between prompt bytes");
            thread::sleep(Duration::from_millis(3));
        }
    } else {
        print!("krust> ");
        io::stdout().flush().unwrap();
    }
}
