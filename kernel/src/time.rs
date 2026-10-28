use core::sync::atomic::{AtomicU64, Ordering};

static BOOT_TSC: AtomicU64 = AtomicU64::new(0);
static COUNTER_HZ: AtomicU64 = AtomicU64::new(0);

pub fn init() {
    let hz = crate::arch::current::clock::frequency();
    assert!(hz != 0, "CPU clock frequency must be nonzero");
    COUNTER_HZ.store(hz, Ordering::Relaxed);
    BOOT_TSC.store(crate::arch::current::clock::counter(), Ordering::Relaxed);
    log::info!("CPU accounting clock: {} Hz", hz);
}

pub fn uptime_ticks() -> u64 {
    crate::arch::current::clock::counter().saturating_sub(BOOT_TSC.load(Ordering::Relaxed))
}

pub fn monotonic_ns() -> u64 {
    let hz = COUNTER_HZ.load(Ordering::Relaxed);
    assert!(hz != 0, "CPU accounting clock is not initialized");
    (u128::from(uptime_ticks()) * 1_000_000_000 / u128::from(hz)).min(u128::from(u64::MAX)) as u64
}
