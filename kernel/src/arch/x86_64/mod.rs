use x86_64::instructions::port::{PortGeneric, ReadWriteAccess};

use crate::arch::{ArchContext, ArchInterrupts, ArchPaging};

pub mod clock;
pub mod context;
pub mod cpu;
pub mod gdt;
pub mod interrupts;
pub mod paging;
pub mod pic;
pub mod pit;
pub mod syscall;
mod supervisor_overlap;
pub mod user_layout;
pub mod user_protection;
pub mod userspace;

pub fn shutdown(status: crate::arch::ShutdownStatus) -> ! {
    use x86_64::instructions::port::Port;

    // Best-effort emulator shutdown via ISA debug-exit.
    // Environments that do not expose this port will simply fall through.
    let code = match status {
        crate::arch::ShutdownStatus::Success => 0x10u32,
        crate::arch::ShutdownStatus::Failure => 0x11u32,
    };

    unsafe {
        let mut port: PortGeneric<u32, ReadWriteAccess> = Port::new(0xf4);
        port.write(code);
    }

    interrupts::halt_loop()
}

pub fn init() {
    cpu::init();
    gdt::init();
    interrupts::init();
    pit::init();
    syscall::init();
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
    type Mapper = paging::OffsetPageTable<'static>;

    fn init() {
        unreachable!("arch paging init is wired by the active boot path");
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
