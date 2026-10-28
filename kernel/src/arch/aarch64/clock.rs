pub fn counter() -> u64 {
    let value;
    unsafe {
        core::arch::asm!("isb", "mrs {}, cntvct_el0", out(reg) value, options(nomem, nostack));
    }
    value
}

pub fn frequency() -> u64 {
    let value;
    unsafe {
        core::arch::asm!("mrs {}, cntfrq_el0", out(reg) value, options(nomem, nostack));
    }
    value
}
