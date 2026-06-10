//! MAP_SHARED file-mapping write-back support for the LXE layer.
//!
//! anyOS has no unified page cache: a file mapping is backed by anonymous
//! pages filled from the file at mmap time. For MAP_SHARED|PROT_WRITE
//! mappings (apt builds its pkgcache this way, dpkg uses them for its
//! database) the modifications must reach the file again. This module
//! tracks shared file mappings per address space (page directory) and
//! writes modified pages back on msync(), munmap() and process exit.
//!
//! Modified pages are found via the hardware PTE dirty bit. The bits are
//! cleared after the initial file fill and after each write-back, so every
//! cycle only writes pages dirtied since the last one. After clearing bits
//! on user-visible pages a full TLB shootdown is issued — a remote TLB
//! entry that still carries D=1 would otherwise allow writes that never
//! re-set the PTE dirty bit.
//!
//! Known limitations (documented, not silent):
//! - A fork() child gets a deep copy of the pages; its mapping degrades to
//!   private (no write-back) because the registry is keyed by the parent's
//!   address space. Threads (CLONE_VM) share the mapping correctly.
//! - SIGKILL skips write-back (no page cache to fall back on).
//! - File offsets beyond 4 GiB are skipped until the VFS gains 64-bit
//!   offsets; a verbose log marks every skipped page.

use super::*;
use crate::memory::address::VirtAddr;
use crate::memory::virtual_mem;
use crate::sync::spinlock::Spinlock;
use core::sync::atomic::{AtomicBool, Ordering};

const PAGE_SIZE: u64 = 4096;

struct SharedFileMapping {
    pd: u64,
    addr: u64,
    len: u64,
    path: String,
    file_offset: u64,
}

static SHARED_FILE_MAPPINGS: Spinlock<Vec<SharedFileMapping>> = Spinlock::new(Vec::new());
/// Fast-path flag so non-LXE processes and LXE processes without shared
/// mappings never take the registry lock on munmap/exit.
static REGISTRY_NONEMPTY: AtomicBool = AtomicBool::new(false);

fn current_pd() -> Option<u64> {
    crate::task::scheduler::current_thread_page_directory().map(|p| p.as_u64())
}

fn page_align_down(v: u64) -> u64 {
    v & !(PAGE_SIZE - 1)
}

fn page_align_up(v: u64) -> u64 {
    (v + PAGE_SIZE - 1) & !(PAGE_SIZE - 1)
}

/// Register a MAP_SHARED file mapping for the current address space.
pub(super) fn register_shared_file_mapping(addr: u64, len: u64, path: String, file_offset: u64) {
    let Some(pd) = current_pd() else {
        return;
    };
    let mut reg = SHARED_FILE_MAPPINGS.lock();
    reg.push(SharedFileMapping {
        pd,
        addr,
        len: page_align_up(len),
        path,
        file_offset,
    });
    REGISTRY_NONEMPTY.store(true, Ordering::Release);
}

/// Clear the dirty bits the initial file fill produced. The mapping address
/// has not been returned to user space yet, so no other CPU can hold a TLB
/// entry for these pages and no shootdown is needed.
pub(super) fn note_initial_fill_complete(addr: u64, len: u64) {
    let mut page = page_align_down(addr);
    let end = page_align_up(addr + len);
    while page < end {
        let _ = virtual_mem::clear_pte_dirty(VirtAddr::new(page));
        page += PAGE_SIZE;
    }
}

/// msync(): write modified pages of shared file mappings intersecting
/// [addr, addr+len) back to their files. `durable` (MS_SYNC) additionally
/// flushes the file durably to disk before returning.
pub(super) fn msync_range(addr: u64, len: u64, durable: bool) -> Result<(), i32> {
    writeback_range(addr, len, durable, false)
}

/// munmap(): write back and drop registry entries fully covered by the
/// unmapped range; partially covered entries are written back and kept
/// (the write-back loop skips unmapped pages, so a shrunk mapping stays
/// safe).
pub(super) fn writeback_before_munmap(addr: u64, len: u64) {
    let _ = writeback_range(addr, len, false, true);
}

/// mremap(): keep the registry in sync when a shared mapping moves or is
/// resized in place. The copy performed by mremap marks every target page
/// dirty, which is conservative but correct — the next write-back rewrites
/// the file from the new location.
pub(super) fn mremap_update(old_addr: u64, new_addr: u64, new_len: u64) {
    if !REGISTRY_NONEMPTY.load(Ordering::Acquire) {
        return;
    }
    let Some(pd) = current_pd() else {
        return;
    };
    let mut reg = SHARED_FILE_MAPPINGS.lock();
    for entry in reg.iter_mut() {
        if entry.pd == pd && entry.addr == old_addr {
            entry.addr = new_addr;
            entry.len = page_align_up(new_len);
            return;
        }
    }
}

/// Process-exit hook. Must run while the exiting process' page directory is
/// still active (the pages are read through their user virtual addresses).
/// `last_thread` removes the entries; earlier-exiting sibling threads only
/// write back.
pub fn writeback_on_exit(pd: u64, last_thread: bool) {
    if !REGISTRY_NONEMPTY.load(Ordering::Acquire) {
        return;
    }
    let entries = snapshot_entries(pd, 0, u64::MAX);
    for entry in &entries {
        let _ = writeback_entry(entry, 0, u64::MAX, false);
    }
    if last_thread {
        remove_entries(pd, 0, u64::MAX);
    }
}

/// Snapshot all entries of `pd` intersecting [start, end) so no lock is held
/// across file I/O.
fn snapshot_entries(pd: u64, start: u64, end: u64) -> Vec<SharedFileMapping> {
    let reg = SHARED_FILE_MAPPINGS.lock();
    reg.iter()
        .filter(|e| e.pd == pd && e.addr < end && start < e.addr.saturating_add(e.len))
        .map(|e| SharedFileMapping {
            pd: e.pd,
            addr: e.addr,
            len: e.len,
            path: e.path.clone(),
            file_offset: e.file_offset,
        })
        .collect()
}

fn remove_entries(pd: u64, start: u64, end: u64) {
    let mut reg = SHARED_FILE_MAPPINGS.lock();
    reg.retain(|e| {
        !(e.pd == pd && e.addr >= start && e.addr.saturating_add(e.len) <= end)
    });
    if reg.is_empty() {
        REGISTRY_NONEMPTY.store(false, Ordering::Release);
    }
}

fn writeback_range(addr: u64, len: u64, durable: bool, remove_covered: bool) -> Result<(), i32> {
    if !REGISTRY_NONEMPTY.load(Ordering::Acquire) {
        return Ok(());
    }
    let Some(pd) = current_pd() else {
        return Ok(());
    };
    let start = page_align_down(addr);
    let end = page_align_up(addr.saturating_add(len));
    let entries = snapshot_entries(pd, start, end);
    let mut first_err: Option<i32> = None;
    for entry in &entries {
        if let Err(errno) = writeback_entry(entry, start, end, durable) {
            first_err.get_or_insert(errno);
        }
    }
    if remove_covered && !entries.is_empty() {
        remove_entries(pd, start, end);
    }
    match first_err {
        Some(errno) => Err(errno),
        None => Ok(()),
    }
}

/// Write the dirty pages of `entry` that fall into [range_start, range_end)
/// back to the file. Opens the file lazily — a mapping without dirty pages
/// (e.g. read-only) costs only a PTE scan.
fn writeback_entry(
    entry: &SharedFileMapping,
    range_start: u64,
    range_end: u64,
    durable: bool,
) -> Result<(), i32> {
    let map_end = entry.addr.saturating_add(entry.len);
    let start = entry.addr.max(range_start);
    let end = map_end.min(range_end);
    if start >= end {
        return Ok(());
    }

    // Collect dirty, present pages first (cheap PTE reads, no file open).
    let mut dirty_pages: Vec<u64> = Vec::new();
    let mut page = page_align_down(start);
    while page < end {
        if virtual_mem::pte_is_dirty(VirtAddr::new(page)) == Some(true) {
            dirty_pages.push(page);
        }
        page += PAGE_SIZE;
    }
    if dirty_pages.is_empty() {
        return Ok(());
    }

    let flags = crate::fs::file::FileFlags {
        read: true,
        write: true,
        append: false,
        create: false,
        truncate: false,
        sync: false,
    };
    let global_id = crate::fs::vfs::open(&entry.path, flags).map_err(|_| EIO)?;
    let result = write_dirty_pages(entry, &dirty_pages, global_id, durable);
    let _ = crate::fs::vfs::close(global_id);

    // The dirty bits were cleared on pages user space can still write to;
    // make sure no other CPU keeps a stale D=1 translation (see module doc).
    #[cfg(target_arch = "x86_64")]
    {
        if virtual_mem::pcid_enabled() {
            crate::arch::x86::smp::tlb_shootdown_pcid(virtual_mem::current_pcid());
        } else {
            crate::arch::x86::smp::tlb_shootdown_full();
        }
    }

    result
}

fn write_dirty_pages(
    entry: &SharedFileMapping,
    dirty_pages: &[u64],
    global_id: u32,
    durable: bool,
) -> Result<(), i32> {
    // POSIX: stores beyond the current end of file are not carried into the
    // file by msync. Clamp every page write to the file size.
    let (_ftype, file_size, _pos, _mtime) =
        crate::fs::vfs::fstat(global_id).map_err(|_| EIO)?;
    let file_size = file_size as u64;

    let mut buf = [0u8; PAGE_SIZE as usize];
    for &page in dirty_pages {
        let file_off = entry.file_offset + (page - entry.addr);
        if file_off >= file_size {
            let _ = virtual_mem::clear_pte_dirty(VirtAddr::new(page));
            continue;
        }
        if file_off > u32::MAX as u64 {
            crate::serial_verbose_println!(
                "lxe shared-mmap: skip page {:#x} (file offset {:#x} beyond u32 VFS limit)",
                page,
                file_off
            );
            continue;
        }
        let n = (file_size - file_off).min(PAGE_SIZE) as usize;
        unsafe {
            core::ptr::copy_nonoverlapping(page as *const u8, buf.as_mut_ptr(), n);
        }
        crate::fs::vfs::lseek(global_id, file_off as i64, 0).map_err(|_| EIO)?;
        let mut written = 0usize;
        while written < n {
            match crate::fs::vfs::write(global_id, &buf[written..n]) {
                Ok(0) => return Err(EIO),
                Ok(w) => written += w,
                Err(_) => return Err(EIO),
            }
        }
        let _ = virtual_mem::clear_pte_dirty(VirtAddr::new(page));
    }

    if durable {
        crate::fs::vfs::fdatasync(global_id).map_err(|_| EIO)?;
    }
    Ok(())
}
