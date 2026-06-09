use x86_64::{PhysAddr, VirtAddr};

pub type PhysicalAddress = u64;
pub type VirtualAddress = u64;

#[inline]
pub const fn to_phys_addr(address: PhysicalAddress) -> PhysAddr {
    PhysAddr::new(address)
}

#[inline]
pub const fn from_phys_addr(address: PhysAddr) -> PhysicalAddress {
    address.as_u64()
}

#[inline]
pub fn to_virt_addr(address: VirtualAddress) -> VirtAddr {
    VirtAddr::new(address)
}

#[inline]
pub const fn from_virt_addr(address: VirtAddr) -> VirtualAddress {
    address.as_u64()
}
