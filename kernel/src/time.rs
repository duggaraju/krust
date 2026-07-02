use core::sync::atomic::{AtomicU64, Ordering};

static BOOT_TSC: AtomicU64 = AtomicU64::new(0);

pub fn init() {
    BOOT_TSC.store(read_tsc(), Ordering::Relaxed);
}

pub fn uptime_ticks() -> u64 {
    read_tsc().saturating_sub(BOOT_TSC.load(Ordering::Relaxed))
}

#[cfg(feature = "arch-x86_64")]
fn read_tsc() -> u64 {
    unsafe { core::arch::x86_64::_rdtsc() }
}

#[cfg(not(feature = "arch-x86_64"))]
fn read_tsc() -> u64 {
    0
}
