use std::ffi::OsString;

#[allow(dead_code)]
pub mod qemu;
#[allow(dead_code)]
pub mod shell;
#[allow(dead_code)]
pub mod suites;
#[allow(dead_code)]
pub mod utilities;

pub fn fixture(variable: &str, default: &str) -> OsString {
    std::env::var_os(variable).unwrap_or_else(|| default.into())
}
