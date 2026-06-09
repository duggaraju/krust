use bootloader_api::BootInfo;

#[cfg(target_arch = "x86_64")]
pub mod x86_64;

#[cfg(target_arch = "x86_64")]
pub use self::x86_64 as current;

pub trait ArchInterrupts {
    fn enable();
    fn disable();
    fn halt_loop() -> !;
}

pub trait ArchPaging {
    type PageTable;
    type Mapper;

    fn init(boot_info: &BootInfo);
}

pub trait ArchContext {
    type ContextFrame;

    unsafe fn switch(current: *mut Self::ContextFrame, next: *const Self::ContextFrame);
}
