use bootloader_api::BootInfo;

use crate::arch::{ArchContext, ArchInterrupts, ArchPaging};

pub mod gdt;
pub mod interrupts;
pub mod paging;

pub fn init() {
    gdt::init();
    interrupts::init();
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
