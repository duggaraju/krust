use crate::arch::{ArchPageFlags, USER_ADDRESS_END, USER_PAGE_SIZE};
use alloc::string::String;
use alloc::vec::Vec;

fn visit_range(
    address: usize,
    length: usize,
    write: bool,
    visit: impl FnMut(usize, usize, usize),
) -> Result<(), isize> {
    crate::process::with_current_task_mut(|task| {
        visit_task_range(task, address, length, write, visit)
    })
    .ok_or(-14isize)?
}

fn visit_task_range(
    task: &crate::process::task::Task,
    address: usize,
    length: usize,
    write: bool,
    mut visit: impl FnMut(usize, usize, usize),
) -> Result<(), isize> {
    let end = address.checked_add(length).ok_or(-14isize)?;
    if address == 0 || end > USER_ADDRESS_END {
        return Err(-14);
    }
    let mut current = address;
    while current < end {
        let page = current & !(USER_PAGE_SIZE - 1);
        let Some((_, physical, flags)) = task
            .address_space
            .mappings()
            .find(|(virtual_address, _, _)| *virtual_address == page)
        else {
            log::warn!(
                "user copy: unmapped address 0x{:x}, pid={}",
                current,
                task.pid
            );
            return Err(-14);
        };
        let required = ArchPageFlags::PRESENT | ArchPageFlags::USER_ACCESSIBLE;
        if !flags.contains(required) || (write && !flags.contains(ArchPageFlags::WRITABLE)) {
            return Err(-14);
        }
        let offset = current - page;
        let count = (USER_PAGE_SIZE - offset).min(end - current);
        let pointer = crate::arch::phys_to_virt_addr((physical + offset) as u64).ok_or(-14isize)?;
        visit(pointer as usize, current - address, count);
        current += count;
    }
    Ok(())
}

pub fn validate_write(address: usize, length: usize) -> Result<(), isize> {
    visit_range(address, length, true, |_, _, _| {})
}

pub fn copy_from(address: usize, output: &mut [u8]) -> Result<(), isize> {
    visit_range(
        address,
        output.len(),
        false,
        |source, offset, count| unsafe {
            core::ptr::copy_nonoverlapping(
                source as *const u8,
                output.as_mut_ptr().add(offset),
                count,
            );
        },
    )
}

pub fn copy_to(address: usize, bytes: &[u8]) -> Result<(), isize> {
    crate::process::with_current_task_mut(|task| copy_to_task(task, address, bytes))
        .ok_or(-14isize)?
}

pub(crate) fn copy_to_task(
    task: &crate::process::task::Task,
    address: usize,
    bytes: &[u8],
) -> Result<(), isize> {
    visit_task_range(task, address, bytes.len(), true, |_, _, _| {})?;
    visit_task_range(
        task,
        address,
        bytes.len(),
        true,
        |destination, offset, count| unsafe {
            core::ptr::copy_nonoverlapping(
                bytes.as_ptr().add(offset),
                destination as *mut u8,
                count,
            );
        },
    )
}

pub fn cstring(address: usize, limit: usize) -> Result<String, isize> {
    let mut bytes = Vec::new();
    while bytes.len() < limit {
        let current = address.checked_add(bytes.len()).ok_or(-14isize)?;
        let count = (USER_PAGE_SIZE - (current & (USER_PAGE_SIZE - 1))).min(limit - bytes.len());
        let mut chunk = [0; USER_PAGE_SIZE];
        copy_from(current, &mut chunk[..count])?;
        if let Some(end) = chunk[..count].iter().position(|byte| *byte == 0) {
            bytes.extend_from_slice(&chunk[..end]);
            return String::from_utf8(bytes).map_err(|_| -22);
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    Err(-7) // E2BIG
}

pub fn string_vector(address: usize, budget: &mut usize) -> Result<Vec<String>, isize> {
    let mut strings = Vec::new();
    if address == 0 {
        return Ok(strings);
    }
    for index in 0..=1024 {
        let slot = address
            .checked_add(index * core::mem::size_of::<usize>())
            .ok_or(-14isize)?;
        let mut pointer = [0; core::mem::size_of::<usize>()];
        copy_from(slot, &mut pointer)?;
        let pointer = usize::from_ne_bytes(pointer);
        if pointer == 0 {
            return Ok(strings);
        }
        if index == 1024 || *budget == 0 {
            return Err(-7);
        }
        let value = cstring(pointer, *budget)?;
        *budget -= value.len() + 1;
        strings.push(value);
    }
    unreachable!()
}
