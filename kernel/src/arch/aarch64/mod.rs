#[cfg(feature = "boot-uefi")]
pub mod boot;
pub mod context;
pub mod interrupts;
pub mod paging;
pub mod syscall;
pub mod userspace;

use crate::arch::{ArchContext, ArchInterrupts, ArchPaging};

pub fn shutdown(status: crate::arch::ShutdownStatus) -> ! {
    let _code = match status {
        crate::arch::ShutdownStatus::Success => 0,
        crate::arch::ShutdownStatus::Failure => 1,
    };

    // ARM64 shutdown via PSCI (Power State Coordination Interface)
    // System off command: SYSTEM_OFF = 0x84000008
    unsafe {
        core::arch::asm!(
            "hvc #0",
            in("x0") 0x84000008u64,
            options(noreturn, preserves_flags)
        );
    }
}

pub struct Interrupts;

impl ArchInterrupts for Interrupts {
    fn enable() {
        interrupts::enable();
    }

    fn disable() {
        interrupts::disable();
    }

    fn halt_loop() -> ! {
        interrupts::halt_loop()
    }
}

pub struct Paging;

impl ArchPaging for Paging {
    type PageTable = paging::PageTable;
    type Mapper = paging::OffsetPageTable;

    fn init() {
        paging::init();
    }
}

pub struct Context;

impl ArchContext for Context {
    type ContextFrame = context::CpuContext;

    unsafe fn switch(current: *mut Self::ContextFrame, next: *const Self::ContextFrame) {
        let _ = (current, next);
        // Note: actual switching is done via context::switch() inline assembly.
        // This trait impl exists for architectural compatibility.
    }
}

pub fn init() {
    interrupts::init();
    syscall::init();
}
