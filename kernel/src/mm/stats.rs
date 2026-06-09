use bootloader_api::info::{MemoryRegion, MemoryRegionKind};
use spin::Once;

#[derive(Debug, Clone, Copy)]
pub struct MemoryStats {
    pub total_bytes: u64,
    pub usable_bytes: u64,
}

static MEMORY_STATS: Once<MemoryStats> = Once::new();

pub fn init(memory_regions: &'static [MemoryRegion]) {
    let total_bytes = memory_regions
        .iter()
        .map(|region| region.end.saturating_sub(region.start))
        .sum();
    let usable_bytes = memory_regions
        .iter()
        .filter(|region| region.kind == MemoryRegionKind::Usable)
        .map(|region| region.end.saturating_sub(region.start))
        .sum();

    let _ = MEMORY_STATS.call_once(|| MemoryStats {
        total_bytes,
        usable_bytes,
    });
}

pub fn get() -> Option<&'static MemoryStats> {
    MEMORY_STATS.get()
}
