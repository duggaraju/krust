extern crate alloc;

use alloc::vec::Vec;

pub const FD_CLOEXEC: usize = 1;
pub const O_CLOEXEC: usize = 0x80000;

#[derive(Clone)]
struct Entry<T> {
    value: T,
    flags: usize,
}

/// Flags belong to table slots, not to the shared open file description.
#[derive(Clone)]
pub struct FdTable<T> {
    slots: Vec<Option<Entry<T>>>,
}

impl<T> FdTable<T> {
    pub fn new() -> Self {
        Self { slots: Vec::new() }
    }

    pub fn insert(&mut self, value: T, flags: usize) -> usize {
        self.insert_from(0, value, flags)
    }

    fn insert_from(&mut self, min: usize, value: T, flags: usize) -> usize {
        let fd = (min..self.slots.len())
            .find(|&fd| self.slots[fd].is_none())
            .unwrap_or(self.slots.len().max(min));
        self.insert_at(fd, value, flags);
        fd
    }

    pub fn insert_at(&mut self, fd: usize, value: T, flags: usize) {
        if fd >= self.slots.len() {
            self.slots.resize_with(fd + 1, || None);
        }
        self.slots[fd] = Some(Entry {
            value,
            flags: flags & FD_CLOEXEC,
        });
    }

    pub fn get(&self, fd: usize) -> Option<&T> {
        self.slots.get(fd)?.as_ref().map(|entry| &entry.value)
    }

    pub fn flags(&self, fd: usize) -> Result<usize, isize> {
        self.slots
            .get(fd)
            .and_then(Option::as_ref)
            .map(|entry| entry.flags)
            .ok_or(-9)
    }

    pub fn set_flags(&mut self, fd: usize, flags: usize) -> Result<(), isize> {
        let entry = self
            .slots
            .get_mut(fd)
            .and_then(Option::as_mut)
            .ok_or(-9isize)?;
        entry.flags = flags & FD_CLOEXEC;
        Ok(())
    }

    pub fn remove(&mut self, fd: usize) -> Option<T> {
        self.slots.get_mut(fd)?.take().map(|entry| entry.value)
    }

    pub fn iter(&self) -> impl Iterator<Item = (usize, &T)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(fd, slot)| slot.as_ref().map(|entry| (fd, &entry.value)))
    }

    pub fn close_on_exec(&mut self) -> Vec<T> {
        let mut closed = Vec::new();
        for slot in &mut self.slots {
            if slot
                .as_ref()
                .is_some_and(|entry| entry.flags & FD_CLOEXEC != 0)
            {
                closed.push(slot.take().unwrap().value);
            }
        }
        closed
    }
}

impl<T: Clone> FdTable<T> {
    pub fn duplicate(&mut self, fd: usize, min: usize, cloexec: bool) -> Result<usize, isize> {
        let value = self.get(fd).ok_or(-9isize)?.clone();
        if min > i32::MAX as usize {
            return Err(-22);
        }
        Ok(self.insert_from(min, value, if cloexec { FD_CLOEXEC } else { 0 }))
    }
}
