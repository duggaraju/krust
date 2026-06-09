extern crate alloc;

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::cmp::min;

use spin::Mutex;

use super::vfs::{DirEntry, FileSystem, FileType, FsError, Inode};

const DIR_ENTRY_SIZE: usize = 32;
const ATTR_READ_ONLY: u8 = 0x01;
const ATTR_HIDDEN: u8 = 0x02;
const ATTR_SYSTEM: u8 = 0x04;
const ATTR_VOLUME_ID: u8 = 0x08;
const ATTR_DIRECTORY: u8 = 0x10;
const ATTR_ARCHIVE: u8 = 0x20;
const ATTR_LFN: u8 = ATTR_READ_ONLY | ATTR_HIDDEN | ATTR_SYSTEM | ATTR_VOLUME_ID;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FatType {
    Fat12,
    Fat16,
    Fat32,
}

#[derive(Debug, Clone, Copy)]
struct BiosParameterBlock {
    bytes_per_sector: u16,
    sectors_per_cluster: u8,
    reserved_sectors: u16,
    num_fats: u8,
    root_entry_count: u16,
    total_sectors: u32,
    fat_size: u32,
    root_cluster: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InodeStorage {
    ClusterChain,
    FixedRoot,
}

#[derive(Debug, Clone, Copy)]
struct FatInodeState {
    first_cluster: u32,
    size: u32,
    dir_entry_offset: Option<usize>,
    storage: InodeStorage,
}

#[derive(Debug, Clone)]
struct ParsedDirEntry {
    name: String,
    file_type: FileType,
    first_cluster: u32,
    size: u32,
    entry_offset: usize,
}

struct FatFsInner {
    image: &'static mut [u8],
    bpb: BiosParameterBlock,
    fat_type: FatType,
    root_dir_sectors: u32,
    root_dir_sector: u32,
    first_fat_sector: u32,
    first_data_sector: u32,
    total_clusters: u32,
    next_free_hint: u32,
}

pub struct FatFs {
    inner: Arc<Mutex<FatFsInner>>,
    root: Arc<FatInode>,
}

pub struct FatInode {
    fs: Arc<Mutex<FatFsInner>>,
    file_type: FileType,
    state: Mutex<FatInodeState>,
}

impl FatFs {
    pub fn mount(image: &'static mut [u8]) -> Result<Self, FsError> {
        if image.len() < 512 {
            return Err(FsError::IoError);
        }

        let bpb = parse_bpb(&image[..512])?;
        validate_bpb(&bpb, image.len())?;

        let root_dir_sectors = ((u32::from(bpb.root_entry_count) * 32)
            + (u32::from(bpb.bytes_per_sector) - 1))
            / u32::from(bpb.bytes_per_sector);
        let first_fat_sector = u32::from(bpb.reserved_sectors);
        let root_dir_sector = first_fat_sector + (u32::from(bpb.num_fats) * bpb.fat_size);
        let first_data_sector = root_dir_sector + root_dir_sectors;
        let data_sectors = bpb
            .total_sectors
            .checked_sub(first_data_sector)
            .ok_or(FsError::IoError)?;
        let total_clusters = data_sectors / u32::from(bpb.sectors_per_cluster);
        let fat_type = if total_clusters < 4085 {
            FatType::Fat12
        } else if total_clusters < 65525 {
            FatType::Fat16
        } else {
            FatType::Fat32
        };

        let root_state = FatInodeState {
            first_cluster: if fat_type == FatType::Fat32 {
                bpb.root_cluster
            } else {
                0
            },
            size: if fat_type == FatType::Fat32 {
                0
            } else {
                usize_to_u32(usize::from(bpb.root_entry_count) * DIR_ENTRY_SIZE)?
            },
            dir_entry_offset: None,
            storage: if fat_type == FatType::Fat32 {
                InodeStorage::ClusterChain
            } else {
                InodeStorage::FixedRoot
            },
        };

        let inner = Arc::new(Mutex::new(FatFsInner {
            image,
            bpb,
            fat_type,
            root_dir_sectors,
            root_dir_sector,
            first_fat_sector,
            first_data_sector,
            total_clusters,
            next_free_hint: 2,
        }));

        let root = Arc::new(FatInode {
            fs: inner.clone(),
            file_type: FileType::Directory,
            state: Mutex::new(root_state),
        });

        Ok(Self { inner, root })
    }

    pub fn read_fat_entry(&self, cluster: u32) -> u32 {
        self.inner.lock().read_fat_entry(cluster)
    }

    pub fn write_fat_entry(&mut self, cluster: u32, value: u32) {
        self.inner.lock().write_fat_entry(cluster, value);
    }

    pub fn allocate_cluster(&mut self) -> Result<u32, FsError> {
        self.inner.lock().allocate_cluster()
    }

    pub fn cluster_to_offset(&self, cluster: u32) -> usize {
        self.inner.lock().cluster_to_offset(cluster)
    }
}

impl FileSystem for FatFs {
    fn name(&self) -> &str {
        "fat"
    }

    fn root_inode(&self) -> Arc<dyn Inode> {
        self.root.clone()
    }
}

impl FatInode {
    fn new(
        fs: Arc<Mutex<FatFsInner>>,
        file_type: FileType,
        first_cluster: u32,
        size: u32,
        dir_entry_offset: Option<usize>,
        storage: InodeStorage,
    ) -> Self {
        Self {
            fs,
            file_type,
            state: Mutex::new(FatInodeState {
                first_cluster,
                size,
                dir_entry_offset,
                storage,
            }),
        }
    }

    fn snapshot(&self) -> FatInodeState {
        *self.state.lock()
    }
}

impl Inode for FatInode {
    fn read(&self, offset: usize, buf: &mut [u8]) -> Result<usize, FsError> {
        if self.file_type == FileType::Directory {
            return Err(FsError::IsADirectory);
        }

        let state = self.snapshot();
        if offset >= state.size as usize || buf.is_empty() {
            return Ok(0);
        }

        let inner = self.fs.lock();
        let chain = inner.collect_chain(state.first_cluster)?;
        if chain.is_empty() {
            return Ok(0);
        }

        let cluster_size = inner.cluster_size();
        let mut copied = 0;
        let limit = min(buf.len(), state.size as usize - offset);
        let mut cluster_index = offset / cluster_size;
        let mut cluster_offset = offset % cluster_size;

        while copied < limit && cluster_index < chain.len() {
            let cluster = chain[cluster_index];
            let image_offset = inner.cluster_to_offset(cluster);
            let chunk = min(limit - copied, cluster_size - cluster_offset);
            let end = image_offset
                .checked_add(cluster_offset + chunk)
                .ok_or(FsError::IoError)?;
            if end > inner.image.len() {
                return Err(FsError::IoError);
            }

            buf[copied..copied + chunk].copy_from_slice(
                &inner.image[image_offset + cluster_offset..image_offset + cluster_offset + chunk],
            );
            copied += chunk;
            cluster_index += 1;
            cluster_offset = 0;
        }

        Ok(copied)
    }

    fn write(&self, offset: usize, buf: &[u8]) -> Result<usize, FsError> {
        if self.file_type == FileType::Directory {
            return Err(FsError::IsADirectory);
        }
        if buf.is_empty() {
            return Ok(0);
        }

        let mut state = self.snapshot();
        let end = offset.checked_add(buf.len()).ok_or(FsError::IoError)?;

        {
            let mut inner = self.fs.lock();
            let cluster_size = inner.cluster_size();
            let needed_clusters = div_ceil(end, cluster_size);
            let (first_cluster, chain) =
                inner.ensure_chain_len(state.first_cluster, needed_clusters)?;

            let mut written = 0;
            let mut cluster_index = offset / cluster_size;
            let mut cluster_offset = offset % cluster_size;

            while written < buf.len() && cluster_index < chain.len() {
                let cluster = chain[cluster_index];
                let image_offset = inner.cluster_to_offset(cluster);
                let chunk = min(buf.len() - written, cluster_size - cluster_offset);
                let write_end = image_offset
                    .checked_add(cluster_offset + chunk)
                    .ok_or(FsError::IoError)?;
                if write_end > inner.image.len() {
                    return Err(FsError::IoError);
                }

                inner.image[image_offset + cluster_offset..image_offset + cluster_offset + chunk]
                    .copy_from_slice(&buf[written..written + chunk]);
                written += chunk;
                cluster_index += 1;
                cluster_offset = 0;
            }

            state.first_cluster = first_cluster;
            state.size = state.size.max(usize_to_u32(end)?);
            if let Some(entry_offset) = state.dir_entry_offset {
                update_dir_entry_metadata(
                    &mut inner,
                    entry_offset,
                    state.first_cluster,
                    state.size,
                )?;
            }
        }

        *self.state.lock() = state;
        Ok(buf.len())
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>, FsError> {
        if self.file_type != FileType::Directory {
            return Err(FsError::NotADirectory);
        }

        let state = self.snapshot();
        let inner = self.fs.lock();
        let entries = read_directory_entries(&inner, state)?;

        let entry = entries
            .into_iter()
            .find(|entry| entry.name.eq_ignore_ascii_case(name))
            .ok_or(FsError::NotFound)?;

        let inode: Arc<dyn Inode> = Arc::new(FatInode::new(
            self.fs.clone(),
            entry.file_type,
            entry.first_cluster,
            entry.size,
            Some(entry.entry_offset),
            InodeStorage::ClusterChain,
        ));
        Ok(inode)
    }

    fn create(&self, name: &str, file_type: FileType) -> Result<Arc<dyn Inode>, FsError> {
        if self.file_type != FileType::Directory {
            return Err(FsError::NotADirectory);
        }
        if !matches!(file_type, FileType::Regular | FileType::Directory) {
            return Err(FsError::NotSupported);
        }

        let short_name = encode_short_name(name)?;
        let parent_state = self.snapshot();
        let mut child_state = FatInodeState {
            first_cluster: 0,
            size: 0,
            dir_entry_offset: None,
            storage: InodeStorage::ClusterChain,
        };

        {
            let mut inner = self.fs.lock();
            let entries = read_directory_entries(&inner, parent_state)?;
            if entries
                .iter()
                .any(|entry| entry.name.eq_ignore_ascii_case(name))
            {
                return Err(FsError::AlreadyExists);
            }

            let dir_entry_offset = find_free_dir_entry(&mut inner, parent_state)?;
            let first_cluster = inner.allocate_cluster()?;

            let attr = match file_type {
                FileType::Regular => ATTR_ARCHIVE,
                FileType::Directory => ATTR_DIRECTORY,
                _ => return Err(FsError::NotSupported),
            };
            write_raw_dir_entry(
                &mut inner,
                dir_entry_offset,
                &short_name,
                attr,
                first_cluster,
                0,
            )?;

            if file_type == FileType::Directory {
                let parent_cluster = if parent_state.storage == InodeStorage::FixedRoot {
                    0
                } else {
                    parent_state.first_cluster
                };
                initialize_directory_cluster(&mut inner, first_cluster, parent_cluster)?;
            }

            child_state.first_cluster = first_cluster;
            child_state.dir_entry_offset = Some(dir_entry_offset);
        }

        let inode: Arc<dyn Inode> = Arc::new(FatInode::new(
            self.fs.clone(),
            file_type,
            child_state.first_cluster,
            child_state.size,
            child_state.dir_entry_offset,
            child_state.storage,
        ));
        Ok(inode)
    }

    fn file_type(&self) -> FileType {
        self.file_type
    }

    fn size(&self) -> usize {
        let state = self.snapshot();
        if self.file_type != FileType::Directory {
            return state.size as usize;
        }

        if state.storage == InodeStorage::FixedRoot {
            return state.size as usize;
        }

        let inner = self.fs.lock();
        match inner.collect_chain(state.first_cluster) {
            Ok(chain) => chain.len() * inner.cluster_size(),
            Err(_) => 0,
        }
    }

    fn readdir(&self) -> Result<Vec<DirEntry>, FsError> {
        if self.file_type != FileType::Directory {
            return Err(FsError::NotADirectory);
        }

        let state = self.snapshot();
        let inner = self.fs.lock();
        let entries = read_directory_entries(&inner, state)?;

        Ok(entries
            .into_iter()
            .filter(|entry| entry.name != "." && entry.name != "..")
            .map(|entry| DirEntry {
                name: entry.name,
                file_type: entry.file_type,
            })
            .collect())
    }
}

impl FatFsInner {
    fn cluster_size(&self) -> usize {
        usize::from(self.bpb.bytes_per_sector) * usize::from(self.bpb.sectors_per_cluster)
    }

    fn fat_offset(&self, fat_index: u32) -> usize {
        let fat_sector = self.first_fat_sector + fat_index * self.bpb.fat_size;
        fat_sector as usize * usize::from(self.bpb.bytes_per_sector)
    }

    fn root_dir_offset(&self) -> usize {
        self.root_dir_sector as usize * usize::from(self.bpb.bytes_per_sector)
    }

    fn max_cluster(&self) -> u32 {
        self.total_clusters + 1
    }

    fn eoc_marker(&self) -> u32 {
        match self.fat_type {
            FatType::Fat12 => 0x0FFF,
            FatType::Fat16 => 0xFFFF,
            FatType::Fat32 => 0x0FFF_FFFF,
        }
    }

    fn is_eoc(&self, value: u32) -> bool {
        match self.fat_type {
            FatType::Fat12 => value >= 0x0FF8,
            FatType::Fat16 => value >= 0xFFF8,
            FatType::Fat32 => (value & 0x0FFF_FFFF) >= 0x0FFF_FFF8,
        }
    }

    fn read_fat_entry(&self, cluster: u32) -> u32 {
        let fat = self.fat_offset(0);
        match self.fat_type {
            FatType::Fat12 => {
                let offset = fat + (cluster as usize * 3 / 2);
                if offset + 1 >= self.image.len() {
                    return 0;
                }
                let value = u16::from_le_bytes([self.image[offset], self.image[offset + 1]]);
                if cluster & 1 == 0 {
                    u32::from(value & 0x0FFF)
                } else {
                    u32::from(value >> 4)
                }
            }
            FatType::Fat16 => {
                let offset = fat + cluster as usize * 2;
                if offset + 1 >= self.image.len() {
                    return 0;
                }
                u16::from_le_bytes([self.image[offset], self.image[offset + 1]]) as u32
            }
            FatType::Fat32 => {
                let offset = fat + cluster as usize * 4;
                if offset + 3 >= self.image.len() {
                    return 0;
                }
                u32::from_le_bytes([
                    self.image[offset],
                    self.image[offset + 1],
                    self.image[offset + 2],
                    self.image[offset + 3],
                ]) & 0x0FFF_FFFF
            }
        }
    }

    fn write_fat_entry(&mut self, cluster: u32, value: u32) {
        for fat_index in 0..u32::from(self.bpb.num_fats) {
            let fat = self.fat_offset(fat_index);
            match self.fat_type {
                FatType::Fat12 => {
                    let offset = fat + (cluster as usize * 3 / 2);
                    if offset + 1 >= self.image.len() {
                        continue;
                    }
                    let current = u16::from_le_bytes([self.image[offset], self.image[offset + 1]]);
                    let next = if cluster & 1 == 0 {
                        (current & 0xF000) | (value as u16 & 0x0FFF)
                    } else {
                        (current & 0x000F) | (((value as u16) & 0x0FFF) << 4)
                    };
                    let bytes = next.to_le_bytes();
                    self.image[offset] = bytes[0];
                    self.image[offset + 1] = bytes[1];
                }
                FatType::Fat16 => {
                    let offset = fat + cluster as usize * 2;
                    if offset + 1 >= self.image.len() {
                        continue;
                    }
                    let bytes = (value as u16).to_le_bytes();
                    self.image[offset] = bytes[0];
                    self.image[offset + 1] = bytes[1];
                }
                FatType::Fat32 => {
                    let offset = fat + cluster as usize * 4;
                    if offset + 3 >= self.image.len() {
                        continue;
                    }
                    let current = u32::from_le_bytes([
                        self.image[offset],
                        self.image[offset + 1],
                        self.image[offset + 2],
                        self.image[offset + 3],
                    ]);
                    let next = (current & 0xF000_0000) | (value & 0x0FFF_FFFF);
                    let bytes = next.to_le_bytes();
                    self.image[offset..offset + 4].copy_from_slice(&bytes);
                }
            }
        }
    }

    fn allocate_cluster(&mut self) -> Result<u32, FsError> {
        let start = self.next_free_hint.max(2);
        let max_cluster = self.max_cluster();

        for range in [start..=max_cluster, 2..=start.saturating_sub(1)] {
            for cluster in range {
                if self.read_fat_entry(cluster) == 0 {
                    self.write_fat_entry(cluster, self.eoc_marker());
                    self.zero_cluster(cluster)?;
                    self.next_free_hint = cluster.saturating_add(1);
                    return Ok(cluster);
                }
            }
        }

        Err(FsError::IoError)
    }

    fn zero_cluster(&mut self, cluster: u32) -> Result<(), FsError> {
        let offset = self.cluster_to_offset(cluster);
        let end = offset
            .checked_add(self.cluster_size())
            .ok_or(FsError::IoError)?;
        if end > self.image.len() {
            return Err(FsError::IoError);
        }
        self.image[offset..end].fill(0);
        Ok(())
    }

    fn cluster_to_offset(&self, cluster: u32) -> usize {
        let sector =
            self.first_data_sector + (cluster - 2) * u32::from(self.bpb.sectors_per_cluster);
        sector as usize * usize::from(self.bpb.bytes_per_sector)
    }

    fn collect_chain(&self, start_cluster: u32) -> Result<Vec<u32>, FsError> {
        if start_cluster < 2 {
            return Ok(Vec::new());
        }

        let mut chain = Vec::new();
        let mut current = start_cluster;
        let mut remaining = self.total_clusters.saturating_add(2);

        while current >= 2 && current <= self.max_cluster() && remaining > 0 {
            chain.push(current);
            let next = self.read_fat_entry(current);
            if next < 2 || self.is_eoc(next) {
                break;
            }
            current = next;
            remaining -= 1;
        }

        if remaining == 0 {
            return Err(FsError::IoError);
        }

        Ok(chain)
    }

    fn ensure_chain_len(
        &mut self,
        first_cluster: u32,
        needed_clusters: usize,
    ) -> Result<(u32, Vec<u32>), FsError> {
        if needed_clusters == 0 {
            return Ok((first_cluster, Vec::new()));
        }

        let mut first = first_cluster;
        let mut chain = self.collect_chain(first_cluster)?;
        if chain.is_empty() {
            first = self.allocate_cluster()?;
            chain.push(first);
        }

        while chain.len() < needed_clusters {
            let next = self.allocate_cluster()?;
            let last = *chain.last().ok_or(FsError::IoError)?;
            self.write_fat_entry(last, next);
            self.write_fat_entry(next, self.eoc_marker());
            chain.push(next);
        }

        Ok((first, chain))
    }
}

fn parse_bpb(sector: &[u8]) -> Result<BiosParameterBlock, FsError> {
    let bytes_per_sector = read_u16(sector, 11)?;
    let sectors_per_cluster = read_u8(sector, 13)?;
    let reserved_sectors = read_u16(sector, 14)?;
    let num_fats = read_u8(sector, 16)?;
    let root_entry_count = read_u16(sector, 17)?;
    let total_sectors_16 = read_u16(sector, 19)?;
    let fat_size_16 = read_u16(sector, 22)?;
    let total_sectors_32 = read_u32(sector, 32)?;
    let fat_size_32 = read_u32(sector, 36)?;
    let root_cluster = read_u32(sector, 44)?;

    let total_sectors = if total_sectors_16 != 0 {
        u32::from(total_sectors_16)
    } else {
        total_sectors_32
    };
    let fat_size = if fat_size_16 != 0 {
        u32::from(fat_size_16)
    } else {
        fat_size_32
    };

    Ok(BiosParameterBlock {
        bytes_per_sector,
        sectors_per_cluster,
        reserved_sectors,
        num_fats,
        root_entry_count,
        total_sectors,
        fat_size,
        root_cluster: if root_cluster == 0 { 2 } else { root_cluster },
    })
}

fn validate_bpb(bpb: &BiosParameterBlock, image_len: usize) -> Result<(), FsError> {
    if bpb.bytes_per_sector == 0
        || bpb.sectors_per_cluster == 0
        || bpb.reserved_sectors == 0
        || bpb.num_fats == 0
        || bpb.total_sectors == 0
        || bpb.fat_size == 0
    {
        return Err(FsError::IoError);
    }

    let total_bytes = usize::from(bpb.bytes_per_sector)
        .checked_mul(bpb.total_sectors as usize)
        .ok_or(FsError::IoError)?;
    if total_bytes > image_len {
        return Err(FsError::IoError);
    }

    Ok(())
}

fn read_directory_entries(
    inner: &FatFsInner,
    state: FatInodeState,
) -> Result<Vec<ParsedDirEntry>, FsError> {
    let regions = directory_regions(inner, state)?;
    let mut entries = Vec::new();

    for (offset, len) in regions {
        let end = offset.checked_add(len).ok_or(FsError::IoError)?;
        if end > inner.image.len() {
            return Err(FsError::IoError);
        }

        let mut entry_offset = offset;
        while entry_offset + DIR_ENTRY_SIZE <= end {
            let entry = &inner.image[entry_offset..entry_offset + DIR_ENTRY_SIZE];
            let first_byte = entry[0];
            if first_byte == 0x00 {
                return Ok(entries);
            }
            if first_byte == 0xE5 {
                entry_offset += DIR_ENTRY_SIZE;
                continue;
            }

            let attrs = entry[11];
            if attrs == ATTR_LFN || (attrs & ATTR_VOLUME_ID) != 0 {
                entry_offset += DIR_ENTRY_SIZE;
                continue;
            }

            if let Some(name) = decode_short_name(&entry[0..11]) {
                let first_cluster = (u32::from(u16::from_le_bytes([entry[20], entry[21]])) << 16)
                    | u32::from(u16::from_le_bytes([entry[26], entry[27]]));
                let size = u32::from_le_bytes([entry[28], entry[29], entry[30], entry[31]]);
                entries.push(ParsedDirEntry {
                    name,
                    file_type: if (attrs & ATTR_DIRECTORY) != 0 {
                        FileType::Directory
                    } else {
                        FileType::Regular
                    },
                    first_cluster,
                    size,
                    entry_offset,
                });
            }

            entry_offset += DIR_ENTRY_SIZE;
        }
    }

    Ok(entries)
}

fn directory_regions(
    inner: &FatFsInner,
    state: FatInodeState,
) -> Result<Vec<(usize, usize)>, FsError> {
    match state.storage {
        InodeStorage::FixedRoot => Ok(vec![(
            inner.root_dir_offset(),
            inner.root_dir_sectors as usize * usize::from(inner.bpb.bytes_per_sector),
        )]),
        InodeStorage::ClusterChain => {
            let mut regions = Vec::new();
            for cluster in inner.collect_chain(state.first_cluster)? {
                regions.push((inner.cluster_to_offset(cluster), inner.cluster_size()));
            }
            Ok(regions)
        }
    }
}

fn find_free_dir_entry(inner: &mut FatFsInner, state: FatInodeState) -> Result<usize, FsError> {
    let regions = directory_regions(inner, state)?;
    for (offset, len) in regions {
        let end = offset.checked_add(len).ok_or(FsError::IoError)?;
        if end > inner.image.len() {
            return Err(FsError::IoError);
        }

        let mut entry_offset = offset;
        while entry_offset + DIR_ENTRY_SIZE <= end {
            let first_byte = inner.image[entry_offset];
            if first_byte == 0x00 || first_byte == 0xE5 {
                return Ok(entry_offset);
            }
            entry_offset += DIR_ENTRY_SIZE;
        }
    }

    if state.storage == InodeStorage::FixedRoot {
        return Err(FsError::IoError);
    }

    let chain = inner.collect_chain(state.first_cluster)?;
    let last = *chain.last().ok_or(FsError::IoError)?;
    let new_cluster = inner.allocate_cluster()?;
    inner.write_fat_entry(last, new_cluster);
    inner.write_fat_entry(new_cluster, inner.eoc_marker());
    Ok(inner.cluster_to_offset(new_cluster))
}

fn write_raw_dir_entry(
    inner: &mut FatFsInner,
    entry_offset: usize,
    short_name: &[u8; 11],
    attributes: u8,
    first_cluster: u32,
    size: u32,
) -> Result<(), FsError> {
    let end = entry_offset
        .checked_add(DIR_ENTRY_SIZE)
        .ok_or(FsError::IoError)?;
    if end > inner.image.len() {
        return Err(FsError::IoError);
    }

    let entry = &mut inner.image[entry_offset..end];
    entry.fill(0);
    entry[0..11].copy_from_slice(short_name);
    entry[11] = attributes;

    let hi = ((first_cluster >> 16) as u16).to_le_bytes();
    let lo = (first_cluster as u16).to_le_bytes();
    entry[20..22].copy_from_slice(&hi);
    entry[26..28].copy_from_slice(&lo);
    entry[28..32].copy_from_slice(&size.to_le_bytes());
    Ok(())
}

fn update_dir_entry_metadata(
    inner: &mut FatFsInner,
    entry_offset: usize,
    first_cluster: u32,
    size: u32,
) -> Result<(), FsError> {
    let end = entry_offset
        .checked_add(DIR_ENTRY_SIZE)
        .ok_or(FsError::IoError)?;
    if end > inner.image.len() {
        return Err(FsError::IoError);
    }

    let entry = &mut inner.image[entry_offset..end];
    let hi = ((first_cluster >> 16) as u16).to_le_bytes();
    let lo = (first_cluster as u16).to_le_bytes();
    entry[20..22].copy_from_slice(&hi);
    entry[26..28].copy_from_slice(&lo);
    entry[28..32].copy_from_slice(&size.to_le_bytes());
    Ok(())
}

fn initialize_directory_cluster(
    inner: &mut FatFsInner,
    self_cluster: u32,
    parent_cluster: u32,
) -> Result<(), FsError> {
    let offset = inner.cluster_to_offset(self_cluster);
    let end = offset
        .checked_add(inner.cluster_size())
        .ok_or(FsError::IoError)?;
    if end > inner.image.len() {
        return Err(FsError::IoError);
    }

    inner.image[offset..end].fill(0);

    let mut dot = [b' '; 11];
    dot[0] = b'.';
    write_raw_dir_entry(inner, offset, &dot, ATTR_DIRECTORY, self_cluster, 0)?;

    let mut dotdot = [b' '; 11];
    dotdot[0] = b'.';
    dotdot[1] = b'.';
    write_raw_dir_entry(
        inner,
        offset + DIR_ENTRY_SIZE,
        &dotdot,
        ATTR_DIRECTORY,
        parent_cluster,
        0,
    )?;
    Ok(())
}

fn decode_short_name(raw: &[u8]) -> Option<String> {
    if raw.len() != 11 {
        return None;
    }

    let mut name = String::new();
    for &byte in &raw[0..8] {
        if byte == b' ' {
            break;
        }
        if !byte.is_ascii() {
            return None;
        }
        name.push(byte as char);
    }

    let mut extension = String::new();
    for &byte in &raw[8..11] {
        if byte == b' ' {
            break;
        }
        if !byte.is_ascii() {
            return None;
        }
        extension.push(byte as char);
    }

    if name.is_empty() {
        return None;
    }

    if extension.is_empty() {
        Some(name)
    } else {
        name.push('.');
        name.push_str(&extension);
        Some(name)
    }
}

fn encode_short_name(name: &str) -> Result<[u8; 11], FsError> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') {
        return Err(FsError::NotSupported);
    }

    let mut parts = name.split('.');
    let base = parts.next().ok_or(FsError::NotSupported)?;
    let extension = parts.next().unwrap_or("");
    if parts.next().is_some() || base.is_empty() || base.len() > 8 || extension.len() > 3 {
        return Err(FsError::NotSupported);
    }

    let mut raw = [b' '; 11];
    write_short_component(base, &mut raw[0..8])?;
    write_short_component(extension, &mut raw[8..11])?;
    Ok(raw)
}

fn write_short_component(component: &str, out: &mut [u8]) -> Result<(), FsError> {
    for (index, byte) in component.bytes().enumerate() {
        if !is_valid_short_name_char(byte) {
            return Err(FsError::NotSupported);
        }
        out[index] = byte.to_ascii_uppercase();
    }
    Ok(())
}

fn is_valid_short_name_char(byte: u8) -> bool {
    matches!(byte,
        b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'$' | b'%' | b'\'' | b'-' | b'_' | b'@' | b'~' | b'`'
            | b'!' | b'(' | b')' | b'{' | b'}' | b'^' | b'#' | b'&')
}

fn read_u8(data: &[u8], offset: usize) -> Result<u8, FsError> {
    data.get(offset).copied().ok_or(FsError::IoError)
}

fn read_u16(data: &[u8], offset: usize) -> Result<u16, FsError> {
    let end = offset.checked_add(2).ok_or(FsError::IoError)?;
    let bytes = data.get(offset..end).ok_or(FsError::IoError)?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(data: &[u8], offset: usize) -> Result<u32, FsError> {
    let end = offset.checked_add(4).ok_or(FsError::IoError)?;
    let bytes = data.get(offset..end).ok_or(FsError::IoError)?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn div_ceil(value: usize, divisor: usize) -> usize {
    value.div_ceil(divisor)
}

fn usize_to_u32(value: usize) -> Result<u32, FsError> {
    u32::try_from(value).map_err(|_| FsError::IoError)
}
