//! Swap area registry and slot allocator.
//!
//! This module is the backing-store half of swap. It deliberately keeps the
//! policy bits (page reclaim, victim selection, COW integration) out of the
//! first slice so callers can enable and inspect swap safely before pages are
//! ever evicted to it.

use crate::fs::file::{FileFlags, FileType};
use crate::memory::FRAME_SIZE;
use crate::sync::mutex::Mutex;
use crate::sync::spinlock::Spinlock;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

const MAX_SWAP_AREAS: usize = 8;
const PAGE_SIZE: u64 = FRAME_SIZE as u64;
const MAX_SWAP_FILE_BYTES: u64 = i32::MAX as u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwapError {
    Invalid,
    NotFound,
    AlreadyEnabled,
    TooManyAreas,
    NotRegular,
    TooSmall,
    TooLarge,
    OpenFailed,
    Io,
    Busy,
    NoSpace,
    BadSlot,
}

#[derive(Debug, Clone, Copy)]
pub struct SwapStats {
    pub areas: u32,
    pub total_pages: u64,
    pub free_pages: u64,
    pub used_pages: u64,
}

/// Stable identifier for a swap page.
///
/// Upper 16 bits: swap area index. Lower 32 bits: page index inside that file.
/// Page index 0 is reserved so future on-disk metadata / Linux mkswap headers
/// are never overwritten.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwapSlot(u64);

impl SwapSlot {
    fn new(area: u16, page: u32) -> Self {
        Self(((area as u64) << 32) | page as u64)
    }

    fn area(self) -> usize {
        ((self.0 >> 32) & 0xffff) as usize
    }

    fn page(self) -> u32 {
        self.0 as u32
    }
}

struct SwapArea {
    path: String,
    fd: u32,
    page_count: u32,
    total_slots: u32,
    free_slots: u32,
    io_refs: u32,
    bitmap: Vec<u64>,
}

static SWAP_AREAS: Spinlock<Vec<Option<SwapArea>>> = Spinlock::new(Vec::new());
static SWAP_WRITE_LOCK: Mutex<()> = Mutex::new(());

pub fn swapon_path(path: &str, _flags: u32) -> Result<(), SwapError> {
    if path.is_empty() {
        return Err(SwapError::Invalid);
    }

    let stat = crate::fs::vfs::stat(path).map_err(|_| SwapError::NotFound)?;
    if stat.file_type != FileType::Regular || stat.is_symlink {
        return Err(SwapError::NotRegular);
    }
    if stat.size as u64 > MAX_SWAP_FILE_BYTES {
        return Err(SwapError::TooLarge);
    }
    let page_count = stat.size as u64 / PAGE_SIZE;
    if page_count < 2 || page_count > u32::MAX as u64 {
        return Err(SwapError::TooSmall);
    }

    let fd =
        crate::fs::vfs::open(path, FileFlags::READ_WRITE).map_err(|_| SwapError::OpenFailed)?;

    let mut bitmap = Vec::new();
    let words = ((page_count as usize) + 63) / 64;
    bitmap
        .try_reserve_exact(words)
        .map_err(|_| SwapError::NoSpace)?;
    for _ in 0..words {
        bitmap.push(0);
    }
    set_bit(&mut bitmap, 0);

    let area = SwapArea {
        path: path.to_string(),
        fd,
        page_count: page_count as u32,
        total_slots: (page_count as u32).saturating_sub(1),
        free_slots: (page_count as u32).saturating_sub(1),
        io_refs: 0,
        bitmap,
    };

    let mut areas = SWAP_AREAS.lock();
    if areas
        .iter()
        .flatten()
        .any(|existing| existing.path.as_str() == path)
    {
        drop(areas);
        let _ = crate::fs::vfs::close(fd);
        return Err(SwapError::AlreadyEnabled);
    }

    if let Some(slot) = areas.iter_mut().find(|slot| slot.is_none()) {
        *slot = Some(area);
        return Ok(());
    }
    if areas.len() >= MAX_SWAP_AREAS {
        drop(areas);
        let _ = crate::fs::vfs::close(fd);
        return Err(SwapError::TooManyAreas);
    }
    areas.push(Some(area));
    Ok(())
}

pub fn swapoff_path(path: &str) -> Result<(), SwapError> {
    if path.is_empty() {
        return Err(SwapError::Invalid);
    }

    let fd = {
        let mut areas = SWAP_AREAS.lock();
        let idx = areas
            .iter()
            .position(|area| {
                area.as_ref()
                    .map_or(false, |area| area.path.as_str() == path)
            })
            .ok_or(SwapError::NotFound)?;
        let area = areas[idx].as_ref().ok_or(SwapError::NotFound)?;
        if area.free_slots != area.total_slots || area.io_refs != 0 {
            return Err(SwapError::Busy);
        }
        areas[idx].take().ok_or(SwapError::NotFound)?.fd
    };

    crate::fs::vfs::close(fd).map_err(|_| SwapError::Io)?;
    Ok(())
}

pub fn allocate_slot() -> Result<SwapSlot, SwapError> {
    let mut areas = SWAP_AREAS.lock();
    for (area_idx, area) in areas.iter_mut().enumerate() {
        let Some(area) = area.as_mut() else {
            continue;
        };
        if area.free_slots == 0 {
            continue;
        }
        for page in 1..area.page_count {
            if !test_bit(&area.bitmap, page) {
                set_bit(&mut area.bitmap, page);
                area.free_slots = area.free_slots.saturating_sub(1);
                return Ok(SwapSlot::new(area_idx as u16, page));
            }
        }
    }
    Err(SwapError::NoSpace)
}

pub fn free_slot(slot: SwapSlot) -> Result<(), SwapError> {
    let mut areas = SWAP_AREAS.lock();
    let area = areas
        .get_mut(slot.area())
        .and_then(|area| area.as_mut())
        .ok_or(SwapError::BadSlot)?;
    let page = slot.page();
    if page == 0 || page >= area.page_count || !test_bit(&area.bitmap, page) {
        return Err(SwapError::BadSlot);
    }
    clear_bit(&mut area.bitmap, page);
    area.free_slots = area.free_slots.saturating_add(1).min(area.total_slots);
    Ok(())
}

pub fn write_page(slot: SwapSlot, page: &[u8; FRAME_SIZE]) -> Result<(), SwapError> {
    let (fd, off) = begin_slot_io(slot)?;
    let _write_guard = SWAP_WRITE_LOCK.lock();
    let result = if crate::fs::vfs::lseek(fd, off as i64, 0).is_ok() {
        match crate::fs::vfs::write(fd, page) {
            Ok(written) if written == FRAME_SIZE => Ok(()),
            Ok(_) | Err(_) => Err(SwapError::Io),
        }
    } else {
        Err(SwapError::Io)
    };
    finish_slot_io(slot);
    result
}

pub fn read_page(slot: SwapSlot, page: &mut [u8; FRAME_SIZE]) -> Result<(), SwapError> {
    let (fd, off) = begin_slot_io(slot)?;
    let result = match crate::fs::vfs::read_at(fd, off, page) {
        Ok(read) if read == FRAME_SIZE => Ok(()),
        Ok(_) | Err(_) => Err(SwapError::Io),
    };
    finish_slot_io(slot);
    result
}

pub fn stats() -> SwapStats {
    let areas = SWAP_AREAS.lock();
    let mut out = SwapStats {
        areas: 0,
        total_pages: 0,
        free_pages: 0,
        used_pages: 0,
    };
    for area in areas.iter().flatten() {
        out.areas += 1;
        out.total_pages += area.total_slots as u64;
        out.free_pages += area.free_slots as u64;
    }
    out.used_pages = out.total_pages.saturating_sub(out.free_pages);
    out
}

fn begin_slot_io(slot: SwapSlot) -> Result<(u32, u32), SwapError> {
    let mut areas = SWAP_AREAS.lock();
    let area = areas
        .get_mut(slot.area())
        .and_then(|area| area.as_mut())
        .ok_or(SwapError::BadSlot)?;
    let page_idx = slot.page();
    if page_idx == 0 || page_idx >= area.page_count || !test_bit(&area.bitmap, page_idx) {
        return Err(SwapError::BadSlot);
    }
    let off = page_idx
        .checked_mul(FRAME_SIZE as u32)
        .ok_or(SwapError::BadSlot)?;
    area.io_refs = area.io_refs.checked_add(1).ok_or(SwapError::Busy)?;
    Ok((area.fd, off))
}

fn finish_slot_io(slot: SwapSlot) {
    let mut areas = SWAP_AREAS.lock();
    if let Some(area) = areas.get_mut(slot.area()).and_then(|area| area.as_mut()) {
        area.io_refs = area.io_refs.saturating_sub(1);
    }
}

fn test_bit(bitmap: &[u64], bit: u32) -> bool {
    let word = (bit / 64) as usize;
    let shift = bit & 63;
    bitmap
        .get(word)
        .map_or(true, |value| (value & (1u64 << shift)) != 0)
}

fn set_bit(bitmap: &mut [u64], bit: u32) {
    let word = (bit / 64) as usize;
    let shift = bit & 63;
    if let Some(value) = bitmap.get_mut(word) {
        *value |= 1u64 << shift;
    }
}

fn clear_bit(bitmap: &mut [u64], bit: u32) {
    let word = (bit / 64) as usize;
    let shift = bit & 63;
    if let Some(value) = bitmap.get_mut(word) {
        *value &= !(1u64 << shift);
    }
}
