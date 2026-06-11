use spin::LazyLock;
use x86_64::instructions::{hlt, interrupts};
use x86_64::registers::control::Cr2;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};

use super::gdt;

const IRQ_BASE_VECTOR: u8 = 32;

#[cfg(feature = "process")]
struct KernelModeGuard {
    previous: Option<crate::process::task::TaskMode>,
}

#[cfg(feature = "process")]
impl KernelModeGuard {
    fn enter() -> Self {
        let previous = crate::process::current_task_mode();
        let _ = crate::process::set_current_task_mode(crate::process::task::TaskMode::Kernel);
        Self { previous }
    }
}

#[cfg(feature = "process")]
impl Drop for KernelModeGuard {
    fn drop(&mut self) {
        if let Some(mode) = self.previous {
            let _ = crate::process::set_current_task_mode(mode);
        }
    }
}

static IDT: LazyLock<InterruptDescriptorTable> = LazyLock::new(|| {
    let mut idt = InterruptDescriptorTable::new();
    idt.breakpoint.set_handler_fn(breakpoint_handler);
    unsafe {
        idt.double_fault
            .set_handler_fn(double_fault_handler)
            .set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);
    }
    idt.page_fault.set_handler_fn(page_fault_handler);
    idt[IRQ_BASE_VECTOR].set_handler_fn(irq0_handler);
    idt[IRQ_BASE_VECTOR + 1].set_handler_fn(irq1_handler);
    idt[IRQ_BASE_VECTOR + 2].set_handler_fn(irq2_handler);
    idt[IRQ_BASE_VECTOR + 3].set_handler_fn(irq3_handler);
    idt[IRQ_BASE_VECTOR + 4].set_handler_fn(irq4_handler);
    idt[IRQ_BASE_VECTOR + 5].set_handler_fn(irq5_handler);
    idt[IRQ_BASE_VECTOR + 6].set_handler_fn(irq6_handler);
    idt[IRQ_BASE_VECTOR + 7].set_handler_fn(irq7_handler);
    idt[IRQ_BASE_VECTOR + 8].set_handler_fn(irq8_handler);
    idt[IRQ_BASE_VECTOR + 9].set_handler_fn(irq9_handler);
    idt[IRQ_BASE_VECTOR + 10].set_handler_fn(irq10_handler);
    idt[IRQ_BASE_VECTOR + 11].set_handler_fn(irq11_handler);
    idt[IRQ_BASE_VECTOR + 12].set_handler_fn(irq12_handler);
    idt[IRQ_BASE_VECTOR + 13].set_handler_fn(irq13_handler);
    idt[IRQ_BASE_VECTOR + 14].set_handler_fn(irq14_handler);
    idt[IRQ_BASE_VECTOR + 15].set_handler_fn(irq15_handler);
    idt
});

pub fn init() {
    IDT.load();
}

pub fn enable() {
    interrupts::enable();
}

pub fn disable() {
    interrupts::disable();
}

pub fn halt_loop() -> ! {
    loop {
        hlt();
    }
}

extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame) {
    #[cfg(feature = "process")]
    let _kernel_mode = KernelModeGuard::enter();
    log::info!("EXCEPTION: BREAKPOINT\n{stack_frame:#?}");
}

extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) -> ! {
    #[cfg(feature = "process")]
    let _kernel_mode = KernelModeGuard::enter();
    log::error!("EXCEPTION: DOUBLE FAULT ({error_code:#x})\n{stack_frame:#?}");
    halt_loop()
}

extern "x86-interrupt" fn page_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    #[cfg(feature = "process")]
    let _kernel_mode = KernelModeGuard::enter();
    log::error!("EXCEPTION: PAGE FAULT");
    log::error!("Accessed Address: {:?}", Cr2::read());
    log::error!("Error Code: {:?}", error_code);
    log::error!("{stack_frame:#?}");
    halt_loop()
}

macro_rules! define_irq_handler {
    ($name:ident, $vector:expr) => {
        extern "x86-interrupt" fn $name(_stack_frame: InterruptStackFrame) {
            #[cfg(feature = "process")]
            let _kernel_mode = KernelModeGuard::enter();
            log::warn!("INTERRUPT: vector {}", $vector);
        }
    };
}

define_irq_handler!(irq0_handler, IRQ_BASE_VECTOR + 0);
define_irq_handler!(irq1_handler, IRQ_BASE_VECTOR + 1);
define_irq_handler!(irq2_handler, IRQ_BASE_VECTOR + 2);
define_irq_handler!(irq3_handler, IRQ_BASE_VECTOR + 3);
define_irq_handler!(irq4_handler, IRQ_BASE_VECTOR + 4);
define_irq_handler!(irq5_handler, IRQ_BASE_VECTOR + 5);
define_irq_handler!(irq6_handler, IRQ_BASE_VECTOR + 6);
define_irq_handler!(irq7_handler, IRQ_BASE_VECTOR + 7);
define_irq_handler!(irq8_handler, IRQ_BASE_VECTOR + 8);
define_irq_handler!(irq9_handler, IRQ_BASE_VECTOR + 9);
define_irq_handler!(irq10_handler, IRQ_BASE_VECTOR + 10);
define_irq_handler!(irq11_handler, IRQ_BASE_VECTOR + 11);
define_irq_handler!(irq12_handler, IRQ_BASE_VECTOR + 12);
define_irq_handler!(irq13_handler, IRQ_BASE_VECTOR + 13);
define_irq_handler!(irq14_handler, IRQ_BASE_VECTOR + 14);
define_irq_handler!(irq15_handler, IRQ_BASE_VECTOR + 15);
