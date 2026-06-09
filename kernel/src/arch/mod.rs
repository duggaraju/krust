use bootloader_api::BootInfo;

#[cfg(target_arch = "x86_64")]
pub mod x86_64;

#[cfg(target_arch = "x86_64")]
pub use self::x86_64 as current;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownStatus {
    Success,
    Failure,
}

pub fn shutdown(status: ShutdownStatus) -> ! {
    #[cfg(target_arch = "x86_64")]
    {
        current::shutdown(status)
    }

    #[cfg(not(target_arch = "x86_64"))]
    loop {
        let _ = status;
        core::hint::spin_loop();
    }
}

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
