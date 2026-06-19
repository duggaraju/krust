use bootloader_api::BootInfo;
use x86_64::instructions::port::{PortGeneric, ReadWriteAccess};

use crate::arch::{ArchContext, ArchInterrupts, ArchPaging};

pub mod context;
pub mod gdt;
pub mod interrupts;
pub mod paging;
pub mod pit;
pub mod syscall;

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

    fn init(boot_info: &BootInfo) {
        paging::init(boot_info);
    }
}

#[derive(Debug, Default, Clone, Copy)]
#[repr(C)]
pub struct ContextFrame {
    pub rsp: u64,
    pub rip: u64,
    pub rflags: u64,
    pub cr3: u64,
}

pub struct Context;

impl ArchContext for Context {
    type ContextFrame = ContextFrame;

    unsafe fn switch(current: *mut Self::ContextFrame, next: *const Self::ContextFrame) {
        let _ = (current, next);
    }
}
