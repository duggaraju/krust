use std::ffi::OsString;
use std::process::Command;
use std::sync::Mutex;

static SCRIPT_LOCK: Mutex<()> = Mutex::new(());

pub fn fixture(variable: &str, default: &str) -> OsString {
    std::env::var_os(variable).unwrap_or_else(|| default.into())
}

pub fn run(script: &str, configure: impl FnOnce(&mut Command)) {
    let mut command = Command::new("bash");
    command.arg(script).current_dir(env!("CARGO_MANIFEST_DIR"));
    configure(&mut command);
    // Legacy suites share log names. Release the lock before asserting so a
    // failed compatibility case does not poison subsequent tests.
    let result = {
        let _guard = SCRIPT_LOCK.lock().expect("test script lock poisoned");
        command.output()
    };
    let output = result.unwrap_or_else(|error| panic!("cannot launch {script}: {error}"));
    print!("{}", String::from_utf8_lossy(&output.stdout));
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    assert!(
        output.status.success(),
        "{script} failed with {}; see output above and retained logs",
        output.status
    );
}
