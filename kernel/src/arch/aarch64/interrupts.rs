/// ARM64 exception handling and interrupt controller setup.
///
/// ARM64 uses the exception vector table and GIC (Generic Interrupt Controller)
/// for interrupt handling.

pub fn init() {
    // Initialize GIC and exception vectors
    // TODO: Implement GIC initialization and exception vector setup
}

pub fn enable() {
    // Enable IRQs by clearing the I bit in DAIF (Disable Asynchronous Interrupt Flags)
    unsafe {
        core::arch::asm!(
            "msr daifclr, #0x1",
            options(nomem, nostack, preserves_flags)
        );
    }
}

pub fn disable() {
    // Disable IRQs by setting the I bit in DAIF
    unsafe {
        core::arch::asm!(
            "msr daifset, #0x1",
            options(nomem, nostack, preserves_flags)
        );
    }
}

pub fn halt_loop() -> ! {
    loop {
        // WFI (Wait For Interrupt) instruction
        unsafe {
            core::arch::asm!("wfi", options(nomem, nostack));
        }
    }
}
