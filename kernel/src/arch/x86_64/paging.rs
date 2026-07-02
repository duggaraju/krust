use bootloader_api::BootInfo;
use spin::Once;
use x86_64::{VirtAddr, registers::control::Cr3};

pub use x86_64::structures::paging::{
    FrameAllocator, Mapper, OffsetPageTable, Page, PageSize, PageTable, PageTableFlags, PhysFrame,
    Size4KiB, Translate,
};

static PHYSICAL_MEMORY_OFFSET: Once<u64> = Once::new();

pub fn init(boot_info: &BootInfo) {
    let _ = boot_info;
}

pub fn set_physical_memory_offset(physical_memory_offset: u64) {
    PHYSICAL_MEMORY_OFFSET.call_once(|| physical_memory_offset);
}

pub fn physical_memory_offset() -> Option<u64> {
    PHYSICAL_MEMORY_OFFSET.get().copied()
}

pub fn phys_to_virt_addr(physical_address: u64) -> Option<u64> {
    let offset = *PHYSICAL_MEMORY_OFFSET.get()?;
    Some(offset.saturating_add(physical_address))
}

pub fn virt_to_phys_addr(virtual_address: u64) -> Option<u64> {
    let phys_offset = *PHYSICAL_MEMORY_OFFSET.get()?;
    let virt = VirtAddr::new(virtual_address);

    let (level_4_table_frame, _) = Cr3::read();
    let mut table_phys = level_4_table_frame.start_address().as_u64();

    let p4_index = u64::from(virt.p4_index());
    let p3_index = u64::from(virt.p3_index());
    let p2_index = u64::from(virt.p2_index());
    let p1_index = u64::from(virt.p1_index());

    let p4_entry = read_page_table_entry(phys_offset, table_phys, p4_index)?;
    if p4_entry & 1 == 0 {
        return None;
    }
    table_phys = p4_entry & 0x000f_ffff_ffff_f000;

    let p3_entry = read_page_table_entry(phys_offset, table_phys, p3_index)?;
    if p3_entry & 1 == 0 {
        return None;
    }
    if p3_entry & (1 << 7) != 0 {
        let frame = p3_entry & 0x000f_ffff_c000_0000;
        let offset = virtual_address & 0x3fff_ffff;
        return Some(frame + offset);
    }
    table_phys = p3_entry & 0x000f_ffff_ffff_f000;

    let p2_entry = read_page_table_entry(phys_offset, table_phys, p2_index)?;
    if p2_entry & 1 == 0 {
        return None;
    }
    if p2_entry & (1 << 7) != 0 {
        let frame = p2_entry & 0x000f_ffff_ffe0_0000;
        let offset = virtual_address & 0x1f_ffff;
        return Some(frame + offset);
    }
    table_phys = p2_entry & 0x000f_ffff_ffff_f000;

    let p1_entry = read_page_table_entry(phys_offset, table_phys, p1_index)?;
    if p1_entry & 1 == 0 {
        return None;
    }
    let frame = p1_entry & 0x000f_ffff_ffff_f000;
    Some(frame + (virtual_address & 0xfff))
}

pub unsafe fn init_mapper(physical_memory_offset: u64) -> OffsetPageTable<'static> {
    let physical_memory_offset = VirtAddr::new(physical_memory_offset);
    let level_4_table = unsafe { active_level_4_table(physical_memory_offset) };
    unsafe { OffsetPageTable::new(level_4_table, physical_memory_offset) }
}

fn read_page_table_entry(phys_offset: u64, table_phys: u64, index: u64) -> Option<u64> {
    let entry_phys = table_phys.checked_add(index.checked_mul(8)?)?;
    let entry_virt = phys_offset.checked_add(entry_phys)?;
    let entry_ptr = entry_virt as *const u64;
    Some(unsafe { core::ptr::read_volatile(entry_ptr) })
}

unsafe fn active_level_4_table(physical_memory_offset: VirtAddr) -> &'static mut PageTable {
    let (level_4_table_frame, _) = Cr3::read();
    let physical_address = level_4_table_frame.start_address();
    let virtual_address = physical_memory_offset + physical_address.as_u64();
    let page_table_ptr: *mut PageTable = virtual_address.as_mut_ptr();

    // SAFETY: the bootloader maps all physical memory at `physical_memory_offset`,
    // so the active level 4 frame is accessible through this virtual address.
    unsafe { &mut *page_table_ptr }
}
