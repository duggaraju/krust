extern crate alloc;

use alloc::vec::Vec;
use core::ptr;

use x86_64::{
    VirtAddr,
    registers::control::Cr3,
    structures::paging::{
        Mapper, OffsetPageTable, Page, PageSize, PageTable, PageTableFlags, PhysFrame, Size4KiB,
    },
};

use super::{allocate_frame, deallocate_frame, physical_memory_offset, with_frame_allocator};
use crate::process::binfmt::{LoadSegment, SegmentPermissions};

const PAGE_SIZE: usize = Size4KiB::SIZE as usize;
#[derive(Debug, Clone, Copy)]
pub enum MmError {
    OutOfFrames,
    MapFailed,
    InvalidAddress,
    IoFailed,
}

#[derive(Debug, Clone, Copy)]
struct Mapping {
    page: Page<Size4KiB>,
    frame: PhysFrame<Size4KiB>,
    flags: PageTableFlags,
}

pub struct AddressSpace {
    root_frame: PhysFrame<Size4KiB>,
    mappings: Vec<Mapping>,
}

impl Clone for AddressSpace {
    fn clone(&self) -> Self {
        self.clone_for_fork()
            .expect("failed to clone address space")
    }
}

impl Drop for AddressSpace {
    fn drop(&mut self) {
        for mapping in self.mappings.drain(..) {
            deallocate_frame(mapping.frame);
        }
        deallocate_frame(self.root_frame);
    }
}

impl AddressSpace {
    pub fn kernel() -> Result<Self, MmError> {
        let root_frame = allocate_frame().ok_or(MmError::OutOfFrames)?;
        zero_frame(root_frame);
        copy_kernel_half_from_active(root_frame)?;
        Ok(Self {
            root_frame,
            mappings: Vec::new(),
        })
    }

    pub fn root_paddr(&self) -> u64 {
        self.root_frame.start_address().as_u64()
    }

    pub fn activate(&self) {
        unsafe {
            Cr3::write(self.root_frame, Cr3::read().1);
        }
    }

    pub fn clone_for_fork(&self) -> Result<Self, MmError> {
        let mut cloned = Self::kernel()?;

        for mapping in &self.mappings {
            let page = mapping.page;
            let frame = cloned.map_page(page, mapping.flags)?;
            copy_frame_contents(mapping.frame, frame)?;
        }

        Ok(cloned)
    }

    pub fn map_segment<F>(
        &mut self,
        segment: &LoadSegment,
        mut read_chunk: F,
    ) -> Result<(), MmError>
    where
        F: FnMut(usize, &mut [u8]) -> Result<usize, MmError>,
    {
        let start = segment.virtual_address;
        let end = start
            .checked_add(segment.memory_size)
            .ok_or(MmError::InvalidAddress)?;
        let mut cursor = start;
        let mut page_buf = alloc::vec![0u8; PAGE_SIZE];

        while cursor < end {
            let page = Page::containing_address(VirtAddr::new(cursor as u64));
            let frame = match self.map_page(page, page_flags_from_segment(segment.flags)) {
                Ok(frame) => frame,
                Err(err) => {
                    log::warn!(
                        "map_segment: map_page failed vaddr=0x{:x} cursor=0x{:x} err={:?}",
                        page.start_address().as_u64(),
                        cursor,
                        err
                    );
                    return Err(err);
                }
            };
            let page_start = page.start_address().as_u64() as usize;
            let page_offset = cursor.saturating_sub(page_start);
            let remaining_in_page = PAGE_SIZE - page_offset;
            let remaining_segment = end - cursor;
            let chunk_len = core::cmp::min(remaining_in_page, remaining_segment);
            let file_offset = cursor.saturating_sub(start).min(segment.file_size);
            let file_len = core::cmp::min(chunk_len, segment.file_size.saturating_sub(file_offset));

            page_buf.fill(0);
            if file_len > 0 {
                let range = &mut page_buf[page_offset..page_offset + file_len];
                let read = match read_chunk(segment.file_offset.saturating_add(file_offset), range) {
                    Ok(read) => read,
                    Err(err) => {
                        log::warn!(
                            "map_segment: read_chunk failed file_off=0x{:x} len=0x{:x} err={:?}",
                            segment.file_offset.saturating_add(file_offset),
                            file_len,
                            err
                        );
                        return Err(err);
                    }
                };
                if read != file_len {
                    log::warn!(
                        "map_segment: short read file_off=0x{:x} want=0x{:x} got=0x{:x}",
                        segment.file_offset.saturating_add(file_offset),
                        file_len,
                        read
                    );
                    return Err(MmError::IoFailed);
                }
            }

            if let Err(err) = write_frame_bytes(frame, 0, &page_buf) {
                log::warn!(
                    "map_segment: write_frame_bytes failed frame=0x{:x} err={:?}",
                    frame.start_address().as_u64(),
                    err
                );
                return Err(err);
            }
            cursor += chunk_len;
        }

        Ok(())
    }

    pub fn map_zeroed_region(
        &mut self,
        start: usize,
        size: usize,
        flags: PageTableFlags,
    ) -> Result<usize, MmError> {
        let mut cursor = start;
        let end = start.checked_add(size).ok_or(MmError::InvalidAddress)?;
        let mut first_page = start;

        while cursor < end {
            let page = Page::containing_address(VirtAddr::new(cursor as u64));
            let _frame = self.map_page(page, flags)?;
            if cursor == start {
                first_page = page.start_address().as_u64() as usize;
            }
            cursor = page.start_address().as_u64() as usize + PAGE_SIZE;
        }

        Ok(first_page)
    }

    pub fn write_bytes(&self, vaddr: usize, bytes: &[u8]) -> Result<(), MmError> {
        let mut written = 0usize;
        let mut cursor = vaddr;
        while written < bytes.len() {
            let (frame, offset) = self.translate(cursor).ok_or(MmError::InvalidAddress)?;
            let chunk = core::cmp::min(PAGE_SIZE - offset, bytes.len() - written);
            write_frame_bytes(frame, offset, &bytes[written..written + chunk])?;
            written += chunk;
            cursor += chunk;
        }
        Ok(())
    }

    pub fn translate(&self, vaddr: usize) -> Option<(PhysFrame<Size4KiB>, usize)> {
        let page = Page::containing_address(VirtAddr::new(vaddr as u64));
        self.mappings
            .iter()
            .find(|mapping| mapping.page == page)
            .map(|mapping| {
                (
                    mapping.frame,
                    vaddr - page.start_address().as_u64() as usize,
                )
            })
    }

    pub fn mappings(&self) -> impl Iterator<Item = (usize, usize, PageTableFlags)> + '_ {
        self.mappings.iter().map(|mapping| {
            (
                mapping.page.start_address().as_u64() as usize,
                mapping.frame.start_address().as_u64() as usize,
                mapping.flags,
            )
        })
    }

    fn map_page(
        &mut self,
        page: Page<Size4KiB>,
        flags: PageTableFlags,
    ) -> Result<PhysFrame<Size4KiB>, MmError> {
        let frame = allocate_frame().ok_or(MmError::OutOfFrames)?;
        zero_frame(frame);
        let map_result = with_frame_allocator(|frame_allocator| {
            let mut mapper = unsafe { mapper_for_frame(self.root_frame) };
            let target_flags = flags | PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
            let mut mapped = unsafe { mapper.map_to(page, frame, target_flags, frame_allocator) };
            if mapped.is_err() {
                if let Ok((_, flush)) = mapper.unmap(page) {
                    flush.flush();
                    mapped = unsafe { mapper.map_to(page, frame, target_flags, frame_allocator) };
                }
            }

            let mapped = mapped.map_err(|_| MmError::MapFailed)?;
            mapped.flush();
            Ok::<(), MmError>(())
        })
        .ok_or(MmError::OutOfFrames)?;
        if let Err(err) = map_result {
            deallocate_frame(frame);
            return Err(err);
        }

        self.mappings.push(Mapping { page, frame, flags });
        Ok(frame)
    }
}

fn copy_kernel_half_from_active(root_frame: PhysFrame<Size4KiB>) -> Result<(), MmError> {
    let phys_offset = physical_memory_offset().ok_or(MmError::InvalidAddress)?;
    let (active_root, _) = Cr3::read();
    let active_ptr =
        VirtAddr::new(phys_offset + active_root.start_address().as_u64()).as_ptr::<PageTable>();
    let new_ptr =
        VirtAddr::new(phys_offset + root_frame.start_address().as_u64()).as_mut_ptr::<PageTable>();

    unsafe {
        ptr::copy_nonoverlapping(active_ptr as *const u8, new_ptr as *mut u8, PAGE_SIZE);
    }

    Ok(())
}

fn copy_frame_contents(src: PhysFrame<Size4KiB>, dst: PhysFrame<Size4KiB>) -> Result<(), MmError> {
    let phys_offset = physical_memory_offset().ok_or(MmError::InvalidAddress)?;
    let src_ptr = (phys_offset + src.start_address().as_u64()) as *const u8;
    let dst_ptr = (phys_offset + dst.start_address().as_u64()) as *mut u8;

    unsafe {
        ptr::copy_nonoverlapping(src_ptr, dst_ptr, PAGE_SIZE);
    }
    Ok(())
}

fn page_flags_from_segment(flags: SegmentPermissions) -> PageTableFlags {
    let mut out = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
    if flags.contains(SegmentPermissions::WRITE) {
        out |= PageTableFlags::WRITABLE;
    }
    out
}

fn write_frame_bytes(
    frame: PhysFrame<Size4KiB>,
    offset: usize,
    bytes: &[u8],
) -> Result<(), MmError> {
    let phys_offset = physical_memory_offset().ok_or(MmError::InvalidAddress)?;
    let ptr = (phys_offset + frame.start_address().as_u64()) as *mut u8;
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), ptr.add(offset), bytes.len());
    }
    Ok(())
}

fn zero_frame(frame: PhysFrame<Size4KiB>) {
    if let Some(phys_offset) = physical_memory_offset() {
        let ptr = (phys_offset + frame.start_address().as_u64()) as *mut u8;
        unsafe {
            ptr::write_bytes(ptr, 0, PAGE_SIZE);
        }
    }
}

unsafe fn mapper_for_frame(root_frame: PhysFrame<Size4KiB>) -> OffsetPageTable<'static> {
    let phys_offset = VirtAddr::new(physical_memory_offset().expect("mm not initialised"));
    let root = (phys_offset + root_frame.start_address().as_u64()).as_mut_ptr();
    unsafe { OffsetPageTable::new(&mut *root, phys_offset) }
}
