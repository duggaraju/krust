use spin::LazyLock;
use x86_64::VirtAddr;
use x86_64::instructions::segmentation::{CS, DS, ES, SS, Segment};
use x86_64::instructions::tables::load_tss;
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;

pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;

const DOUBLE_FAULT_STACK_SIZE: usize = 4096 * 5;

static TSS: LazyLock<TaskStateSegment> = LazyLock::new(|| {
    let mut tss = TaskStateSegment::new();
    static DOUBLE_FAULT_STACK: [u8; DOUBLE_FAULT_STACK_SIZE] = [0; DOUBLE_FAULT_STACK_SIZE];

    let stack_start = VirtAddr::from_ptr(&DOUBLE_FAULT_STACK);
    let stack_end = stack_start + DOUBLE_FAULT_STACK_SIZE as u64;
    tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = stack_end;
    tss
});

struct Selectors {
    code_selector: SegmentSelector,
    data_selector: SegmentSelector,
    tss_selector: SegmentSelector,
}

static GDT: LazyLock<(GlobalDescriptorTable, Selectors)> = LazyLock::new(|| {
    let mut gdt = GlobalDescriptorTable::new();
    let code_selector = gdt.append(Descriptor::kernel_code_segment());
    let data_selector = gdt.append(Descriptor::kernel_data_segment());
    let tss_selector = gdt.append(Descriptor::tss_segment(&TSS));

    (
        gdt,
        Selectors {
            code_selector,
            data_selector,
            tss_selector,
        },
    )
});

pub fn init() {
    let (gdt, selectors) = &*GDT;

    gdt.load();

    unsafe {
        CS::set_reg(selectors.code_selector);
        DS::set_reg(selectors.data_selector);
        ES::set_reg(selectors.data_selector);
        SS::set_reg(selectors.data_selector);
        load_tss(selectors.tss_selector);
    }
}
