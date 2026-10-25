use x86_64::registers::control::{Cr0, Cr0Flags, Cr4, Cr4Flags};

pub fn init() {
    unsafe {
        // x86-64 userspace requires x87/SSE2. BIOS firmware need not enable
        // them, unlike UEFI; preserve paging and write-protection settings.
        Cr0::update(|flags| {
            flags.remove(Cr0Flags::EMULATE_COPROCESSOR | Cr0Flags::TASK_SWITCHED);
            flags.insert(Cr0Flags::MONITOR_COPROCESSOR | Cr0Flags::NUMERIC_ERROR);
        });
        Cr4::update(|flags| {
            flags.insert(Cr4Flags::OSFXSR | Cr4Flags::OSXMMEXCPT_ENABLE);
        });
        let mxcsr = 0x1f80u32;
        core::arch::asm!(
            "fninit",
            "ldmxcsr [{mxcsr}]",
            mxcsr = in(reg) &mxcsr,
            options(nostack)
        );
    }
}
