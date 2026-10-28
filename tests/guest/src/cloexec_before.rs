#![no_std]
#![no_main]

krust_guest::entry!();

#[unsafe(no_mangle)]
pub extern "C" fn guest_main(_: *const usize) -> ! {
    krust_guest::cloexec::run(false)
}
