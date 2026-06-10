use super::*;

const FILE_MAP_COPY_CHUNK: usize = 64 * 1024;
const MREMAP_COPY_CHUNK: usize = 64 * 1024;

pub(super) fn linux_brk(new_brk: u64) -> u64 {
    let current = crate::task::scheduler::current_thread_brk();
    if new_brk == 0 {
        crate::serial_verbose_println!("lxe linux brk: query -> {:#x}", current);
        return current;
    }
    let delta = new_brk as i64 - current as i64;
    let old = handlers::sys_sbrk_u64(delta);
    if old == u64::MAX {
        crate::serial_verbose_println!(
            "lxe linux brk: failed current={:#x} requested={:#x} delta={}",
            current,
            new_brk,
            delta
        );
        current
    } else {
        let updated = crate::task::scheduler::current_thread_brk();
        crate::serial_verbose_println!(
            "lxe linux brk: current={:#x} requested={:#x} delta={} -> {:#x}",
            current,
            new_brk,
            delta,
            updated
        );
        updated
    }
}

pub(super) fn linux_mmap(addr: u64, len: u64, prot: u64, flags: u64, fd: u64, offset: u64) -> u64 {
    if len == 0 {
        crate::serial_verbose_println!(
            "lxe linux mmap: reject zero len addr={:#x} prot={:#x} flags={:#x} fd={:#x} off={:#x}",
            addr,
            prot,
            flags,
            fd,
            offset
        );
        return linux_err(EINVAL);
    }
    let anonymous = (flags & LINUX_MAP_ANONYMOUS) != 0;
    let map_type = flags & 0x3;
    let private = map_type == LINUX_MAP_PRIVATE;
    let shared = map_type == LINUX_MAP_SHARED;
    let fixed = (flags & LINUX_MAP_FIXED) != 0;
    if !private && !shared {
        crate::serial_verbose_println!(
            "lxe linux mmap: reject map-type addr={:#x} len={:#x} prot={:#x} flags={:#x} fd={:#x} off={:#x}",
            addr,
            len,
            prot,
            flags,
            fd,
            offset
        );
        return linux_err(ENOSYS);
    }
    if !anonymous && fd <= u32::MAX as u64 {
        if let Some(entry) = crate::task::scheduler::current_fd_get(fd as u32) {
            if let crate::fs::fd_table::FdKind::LinuxFramebuffer { .. } = entry.kind {
                return linux_fb_mmap(addr, len, prot, flags, offset);
            }
        }
    }
    // Shared file mappings are backed by anonymous pages plus dirty-page
    // write-back (see shared_mmap.rs); the file path is required for that.
    // Shared anonymous mappings work correctly between CLONE_VM threads
    // (same address space); a fork() child silently degrades to a private
    // copy — logged so the gap is visible.
    let shared_file_path = if shared && !anonymous && fd <= u32::MAX as u64 {
        crate::task::scheduler::current_fd_get(fd as u32).and_then(|entry| match entry.kind {
            crate::fs::fd_table::FdKind::File { global_id } => {
                crate::fs::vfs::get_fd_path(global_id).ok()
            }
            _ => None,
        })
    } else {
        None
    };
    if shared && !anonymous && (prot & LINUX_PROT_WRITE) != 0 && shared_file_path.is_none() {
        crate::serial_verbose_println!(
            "lxe linux mmap: reject shared-write without file path addr={:#x} len={:#x} prot={:#x} flags={:#x} fd={:#x} off={:#x}",
            addr,
            len,
            prot,
            flags,
            fd,
            offset
        );
        return linux_err(ENOSYS);
    }
    if shared && anonymous && (prot & LINUX_PROT_WRITE) != 0 {
        crate::serial_verbose_println!(
            "lxe linux mmap: shared-anon mapping (thread-shared only, not fork-shared) addr={:#x} len={:#x}",
            addr,
            len
        );
    }
    if anonymous && !linux_fd_is_minus_one(fd) {
        crate::serial_verbose_println!(
            "lxe linux mmap: reject anon fd addr={:#x} len={:#x} prot={:#x} flags={:#x} fd={:#x} off={:#x}",
            addr,
            len,
            prot,
            flags,
            fd,
            offset
        );
        return linux_err(EINVAL);
    }
    if !anonymous && linux_fd_is_minus_one(fd) {
        crate::serial_verbose_println!(
            "lxe linux mmap: reject file fd addr={:#x} len={:#x} prot={:#x} flags={:#x} fd={:#x} off={:#x}",
            addr,
            len,
            prot,
            flags,
            fd,
            offset
        );
        return linux_err(EBADF);
    }

    let mapped = if fixed {
        if addr == 0 {
            crate::serial_verbose_println!(
                "lxe linux mmap: reject fixed-null len={:#x} prot={:#x} flags={:#x} fd={:#x} off={:#x}",
                len,
                prot,
                flags,
                fd,
                offset
            );
            return linux_err(EINVAL);
        }
        match linux_map_fixed(addr, len) {
            Some(addr) => addr,
            None => {
                crate::serial_verbose_println!(
                    "lxe linux mmap: fixed failed addr={:#x} len={:#x} prot={:#x} flags={:#x} fd={:#x} off={:#x}",
                    addr,
                    len,
                    prot,
                    flags,
                    fd,
                    offset
                );
                return linux_err(ENOMEM);
            }
        }
    } else {
        let no_access = (prot & (LINUX_PROT_READ | LINUX_PROT_WRITE | LINUX_PROT_EXEC)) == 0;
        let large_anon = anonymous && len >= 16 * 1024 * 1024;
        if anonymous && (no_access || large_anon) {
            handlers::sys_mmap_reserve_u64(len)
        } else {
            handlers::sys_mmap_u64(len)
        }
    };
    if mapped == u64::MAX {
        crate::serial_verbose_println!(
            "lxe linux mmap: alloc failed addr={:#x} len={:#x} prot={:#x} flags={:#x} fd={:#x} off={:#x}",
            addr,
            len,
            prot,
            flags,
            fd,
            offset
        );
        return linux_err(ENOMEM);
    }

    let mapped_path = if !anonymous {
        crate::task::scheduler::current_fd_get(fd as u32).and_then(|entry| match entry.kind {
            crate::fs::fd_table::FdKind::File { global_id } => {
                crate::fs::vfs::get_fd_path(global_id).ok()
            }
            _ => None,
        })
    } else {
        None
    };

    if !anonymous {
        if let Err(errno) = linux_fill_mapping_from_fd(fd as u32, mapped, len, offset) {
            let _ = handlers::sys_munmap_u64(mapped, len);
            crate::serial_verbose_println!(
                "lxe linux mmap: fill failed mapped={:#x} len={:#x} prot={:#x} flags={:#x} fd={:#x} off={:#x} errno={}",
                mapped,
                len,
                prot,
                flags,
                fd,
                offset,
                errno
            );
            return linux_err(errno);
        }
        if let Some(path) = shared_file_path {
            super::shared_mmap::register_shared_file_mapping(mapped, len, path, offset);
            super::shared_mmap::note_initial_fill_complete(mapped, len);
        }
    }
    if let Some(path) = mapped_path {
        crate::serial_verbose_println!(
            "lxe linux mmap: ok addr={:#x} len={:#x} prot={:#x} flags={:#x} fd={:#x} off={:#x} -> {:#x} path='{}'",
            addr,
            len,
            prot,
            flags,
            fd,
            offset,
            mapped,
            path
        );
    } else {
        crate::serial_verbose_println!(
            "lxe linux mmap: ok addr={:#x} len={:#x} prot={:#x} flags={:#x} fd={:#x} off={:#x} -> {:#x}",
            addr,
            len,
            prot,
            flags,
            fd,
            offset,
            mapped
        );
    }
    mapped
}

pub(super) fn linux_fd_is_minus_one(fd: u64) -> bool {
    fd == u64::MAX || fd == u32::MAX as u64
}

pub(super) fn linux_munmap(addr: u64, len: u64) -> u64 {
    if linux_fb_munmap(addr, len) {
        return 0;
    }
    // Shared file mappings must reach the file before the pages disappear.
    super::shared_mmap::writeback_before_munmap(addr, len);
    let ret = if addr <= u32::MAX as u64 {
        handlers::sys_munmap(addr as u32, len as u32) as u64
    } else {
        handlers::sys_munmap_u64(addr, len)
    };
    if ret == u64::MAX {
        linux_err(EINVAL)
    } else {
        0
    }
}

pub(super) fn linux_mremap(
    old_addr: u64,
    old_size: u64,
    new_size: u64,
    flags: u64,
    new_addr: u64,
) -> u64 {
    const PAGE_SIZE: u64 = 4096;
    const MREMAP_MAYMOVE: u64 = 1;
    const MREMAP_FIXED: u64 = 2;
    const MREMAP_DONTUNMAP: u64 = 4;
    const VALID_FLAGS: u64 = MREMAP_MAYMOVE | MREMAP_FIXED | MREMAP_DONTUNMAP;

    let may_move = (flags & MREMAP_MAYMOVE) != 0;
    let fixed = (flags & MREMAP_FIXED) != 0;

    if old_addr == 0
        || old_size == 0
        || new_size == 0
        || (old_addr & (PAGE_SIZE - 1)) != 0
        || (flags & !VALID_FLAGS) != 0
        || (flags & MREMAP_DONTUNMAP) != 0
        || (fixed && (!may_move || new_addr == 0 || (new_addr & (PAGE_SIZE - 1)) != 0))
    {
        crate::serial_verbose_println!(
            "lxe linux mremap: reject old={:#x} old_size={:#x} new_size={:#x} flags={:#x} new={:#x}",
            old_addr,
            old_size,
            new_size,
            flags,
            new_addr
        );
        return linux_err(EINVAL);
    }

    let old_aligned = match align_page_up(old_size) {
        Some(size) => size,
        None => return linux_err(EINVAL),
    };
    let new_aligned = match align_page_up(new_size) {
        Some(size) => size,
        None => return linux_err(EINVAL),
    };

    if !handlers::helpers::is_user_range_accessible(old_addr, old_aligned) {
        crate::serial_verbose_println!(
            "lxe linux mremap: old range inaccessible old={:#x} old_size={:#x} aligned={:#x}",
            old_addr,
            old_size,
            old_aligned
        );
        return linux_err(EFAULT);
    }

    if fixed && new_addr == old_addr {
        if new_aligned <= old_aligned {
            return linux_mremap_shrink_in_place(
                old_addr,
                old_size,
                old_aligned,
                new_size,
                new_aligned,
            );
        }
        return linux_err(ENOMEM);
    }

    if !fixed && new_aligned <= old_aligned {
        return linux_mremap_shrink_in_place(
            old_addr,
            old_size,
            old_aligned,
            new_size,
            new_aligned,
        );
    }

    if !may_move {
        crate::serial_verbose_println!(
            "lxe linux mremap: grow needs move old={:#x} old_size={:#x} new_size={:#x}",
            old_addr,
            old_size,
            new_size
        );
        return linux_err(ENOMEM);
    }

    let target = if fixed {
        let old_end = match old_addr.checked_add(old_aligned) {
            Some(end) => end,
            None => return linux_err(EINVAL),
        };
        let new_end = match new_addr.checked_add(new_aligned) {
            Some(end) => end,
            None => return linux_err(EINVAL),
        };
        if ranges_overlap(old_addr, old_end, new_addr, new_end) {
            crate::serial_verbose_println!(
                "lxe linux mremap: reject overlapping fixed move old={:#x}-{:#x} new={:#x}-{:#x}",
                old_addr,
                old_end,
                new_addr,
                new_end
            );
            return linux_err(EINVAL);
        }
        match linux_map_fixed(new_addr, new_aligned) {
            Some(addr) => addr,
            None => return linux_err(ENOMEM),
        }
    } else {
        let addr = handlers::sys_mmap_u64(new_aligned);
        if addr == u64::MAX {
            return linux_err(ENOMEM);
        }
        addr
    };

    let copy_len = old_aligned.min(new_aligned);
    if !handlers::helpers::is_user_range_accessible(target, copy_len) {
        let _ = linux_munmap(target, new_aligned);
        return linux_err(EFAULT);
    }

    let mut copied = 0u64;
    while copied < copy_len {
        let chunk_len = ((copy_len - copied) as usize).min(MREMAP_COPY_CHUNK);
        let src = old_addr.wrapping_add(copied);
        let dst = target.wrapping_add(copied);
        let Some(chunk) = handlers::helpers::copy_user_bytes(src, chunk_len, MREMAP_COPY_CHUNK)
        else {
            let _ = linux_munmap(target, new_aligned);
            return linux_err(EFAULT);
        };
        if !handlers::helpers::copy_to_user_bytes(dst, &chunk, MREMAP_COPY_CHUNK) {
            let _ = linux_munmap(target, new_aligned);
            return linux_err(EFAULT);
        }
        copied += chunk_len as u64;
    }

    // Move a shared file mapping's registry entry to the new address BEFORE
    // the old range is unmapped — linux_munmap would otherwise write back
    // and drop the entry, losing write-back for the moved mapping. The copy
    // above marked the target pages dirty, so nothing is missed.
    super::shared_mmap::mremap_update(old_addr, target, new_aligned);

    let ret = linux_munmap(old_addr, old_aligned);
    if linux_is_err(ret) {
        let _ = linux_munmap(target, new_aligned);
        crate::serial_verbose_println!(
            "lxe linux mremap: old munmap failed old={:#x} old_size={:#x} target={:#x} ret={:#x}",
            old_addr,
            old_aligned,
            target,
            ret
        );
        return ret;
    }

    crate::serial_verbose_println!(
        "lxe linux mremap: move old={:#x} old_size={:#x} new_size={:#x} flags={:#x} -> {:#x}",
        old_addr,
        old_size,
        new_size,
        flags,
        target
    );
    target
}

fn linux_mremap_shrink_in_place(
    old_addr: u64,
    old_size: u64,
    old_aligned: u64,
    new_size: u64,
    new_aligned: u64,
) -> u64 {
    if new_aligned <= old_aligned {
        if new_aligned < old_aligned {
            let tail = old_addr + new_aligned;
            let tail_len = old_aligned - new_aligned;
            let ret = linux_munmap(tail, tail_len);
            if linux_is_err(ret) {
                crate::serial_verbose_println!(
                    "lxe linux mremap: shrink tail munmap failed old={:#x} tail={:#x} tail_len={:#x} ret={:#x}",
                    old_addr,
                    tail,
                    tail_len,
                    ret
                );
                return ret;
            }
        }
        super::shared_mmap::mremap_update(old_addr, old_addr, new_aligned);
        crate::serial_verbose_println!(
            "lxe linux mremap: shrink/in-place old={:#x} old_size={:#x} new_size={:#x} -> {:#x}",
            old_addr,
            old_size,
            new_size,
            old_addr
        );
        return old_addr;
    }
    linux_err(EINVAL)
}

pub(super) fn linux_mprotect(_addr: u64, len: u64, _prot: u64) -> u64 {
    if len == 0 {
        linux_err(EINVAL)
    } else {
        0
    }
}

pub(super) fn linux_mincore(addr: u64, len: u64, vec_ptr: u64) -> u64 {
    use crate::memory::address::VirtAddr;

    const PAGE_SIZE: u64 = 4096;
    const MAX_MINCORE_PAGES: u64 = 1 << 20;

    if (addr & (PAGE_SIZE - 1)) != 0 {
        return linux_err(EINVAL);
    }
    if len == 0 {
        return 0;
    }
    if !handlers::helpers::is_valid_user_ptr(addr, len) {
        return linux_err(ENOMEM);
    }

    let pages = match len.checked_add(PAGE_SIZE - 1) {
        Some(size) => size / PAGE_SIZE,
        None => return linux_err(ENOMEM),
    };
    if pages == 0 {
        return 0;
    }
    if pages > MAX_MINCORE_PAGES {
        return linux_err(ENOMEM);
    }
    if !handlers::helpers::is_user_range_accessible(vec_ptr, pages) {
        return linux_err(EFAULT);
    }

    let mut out = alloc::vec![0u8; pages as usize];
    for (idx, byte) in out.iter_mut().enumerate() {
        let page = addr + (idx as u64 * PAGE_SIZE);
        if !crate::memory::virtual_mem::is_page_mapped(VirtAddr::new(page)) {
            return linux_err(ENOMEM);
        }
        *byte = 1;
    }

    if !handlers::helpers::copy_to_user_bytes(vec_ptr, &out, out.len()) {
        return linux_err(EFAULT);
    }
    0
}

pub(super) fn linux_msync(addr: u64, len: u64, flags: u64) -> u64 {
    const PAGE_SIZE: u64 = 4096;
    const MS_ASYNC: u64 = 0x1;
    const MS_INVALIDATE: u64 = 0x2;
    const MS_SYNC: u64 = 0x4;
    const VALID_FLAGS: u64 = MS_ASYNC | MS_INVALIDATE | MS_SYNC;

    if addr & (PAGE_SIZE - 1) != 0 || (flags & !VALID_FLAGS) != 0 {
        return linux_err(EINVAL);
    }
    if (flags & MS_ASYNC) != 0 && (flags & MS_SYNC) != 0 {
        return linux_err(EINVAL);
    }
    if len == 0 {
        return 0;
    }
    if !handlers::helpers::is_user_range_accessible(addr, len) {
        return linux_err(ENOMEM);
    }

    // Write modified pages of shared file mappings back to their files.
    // MS_SYNC requires the data to be durable before returning; MS_ASYNC
    // schedules the same write-back synchronously (we have no async queue)
    // but skips the device cache flush. Private/anonymous ranges have no
    // backing file and correctly fall through as a no-op.
    let durable = (flags & MS_SYNC) != 0;
    match super::shared_mmap::msync_range(addr, len, durable) {
        Ok(()) => 0,
        Err(errno) => linux_err(errno),
    }
}

pub(super) fn linux_swapon(path_ptr: u64, flags: u64) -> u64 {
    if current_linux_id(true) != 0 {
        return linux_err(EPERM);
    }
    let raw_path = match handlers::helpers::read_user_str_safe(path_ptr) {
        Some(path) if !path.is_empty() => path,
        Some(_) => return linux_err(ENOENT),
        None => return linux_err(EFAULT),
    };
    let abs = linux_absolute_path(raw_path);
    let translated = linux_translate_absolute_path(&abs);
    let resolved = match linux_resolve_translated_path(&translated, true, false) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    match crate::memory::swap::swapon_path(&resolved, flags as u32) {
        Ok(()) => 0,
        Err(err) => linux_err(swap_errno(err)),
    }
}

pub(super) fn linux_swapoff(path_ptr: u64) -> u64 {
    if current_linux_id(true) != 0 {
        return linux_err(EPERM);
    }
    let raw_path = match handlers::helpers::read_user_str_safe(path_ptr) {
        Some(path) if !path.is_empty() => path,
        Some(_) => return linux_err(ENOENT),
        None => return linux_err(EFAULT),
    };
    let abs = linux_absolute_path(raw_path);
    let translated = linux_translate_absolute_path(&abs);
    let resolved = match linux_resolve_translated_path(&translated, true, false) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    match crate::memory::swap::swapoff_path(&resolved) {
        Ok(()) => 0,
        Err(err) => linux_err(swap_errno(err)),
    }
}

fn swap_errno(err: crate::memory::swap::SwapError) -> i32 {
    match err {
        crate::memory::swap::SwapError::Invalid => EINVAL,
        crate::memory::swap::SwapError::NotFound => ENOENT,
        crate::memory::swap::SwapError::AlreadyEnabled => EBUSY,
        crate::memory::swap::SwapError::TooManyAreas => ENOMEM,
        crate::memory::swap::SwapError::NotRegular => EINVAL,
        crate::memory::swap::SwapError::TooSmall => EINVAL,
        crate::memory::swap::SwapError::TooLarge => EFBIG,
        crate::memory::swap::SwapError::OpenFailed => EACCES,
        crate::memory::swap::SwapError::Io => EIO,
        crate::memory::swap::SwapError::Busy => EBUSY,
        crate::memory::swap::SwapError::NoSpace => ENOMEM,
        crate::memory::swap::SwapError::BadSlot => EINVAL,
    }
}

pub(super) fn linux_arch_prctl(code: u64, addr: u64) -> u64 {
    match code {
        LINUX_ARCH_SET_FS => {
            crate::task::scheduler::set_current_thread_linux_fs_base(addr);
            #[cfg(target_arch = "x86_64")]
            unsafe {
                crate::arch::x86::power::wrmsr(0xC000_0100, addr);
            }
            crate::serial_verbose_println!(
                "lxe linux arch_prctl: tid={} SET_FS {:#x}",
                crate::task::scheduler::current_tid(),
                addr
            );
            0
        }
        LINUX_ARCH_GET_FS => {
            if addr == 0 {
                return linux_err(EFAULT);
            }
            let fs_base = crate::task::scheduler::current_thread_linux_fs_base();
            unsafe {
                write_u64(addr, 0, fs_base);
            }
            0
        }
        _ => linux_err(EINVAL),
    }
}

pub(super) fn linux_map_fixed(addr: u64, len: u64) -> Option<u64> {
    use crate::memory::address::VirtAddr;
    use crate::memory::{physical, virtual_mem};

    const PAGE_SIZE: u64 = 4096;
    if addr & (PAGE_SIZE - 1) != 0 {
        return None;
    }
    let aligned_size = len.checked_add(PAGE_SIZE - 1)? & !(PAGE_SIZE - 1);
    if addr <= u32::MAX as u64 {
        let _ = handlers::sys_munmap(addr as u32, aligned_size as u32);
    } else {
        let _ = handlers::sys_munmap_u64(addr, aligned_size);
    }

    let pd = crate::task::scheduler::current_thread_page_directory()?;
    if !crate::memory::vma::alloc_fixed_region64(pd, addr, aligned_size) {
        return None;
    }

    let mut mapped_until = addr;
    while mapped_until < addr + aligned_size {
        let phys = match physical::alloc_frame_with(physical::FrameAllocPolicy::Any) {
            Some(phys) => phys,
            None => {
                let _ = linux_munmap(addr, aligned_size);
                return None;
            }
        };
        if !virtual_mem::zero_frame(phys)
            || !virtual_mem::map_page(VirtAddr::new(mapped_until), phys, 0x02 | 0x04)
        {
            physical::free_frame(phys);
            let _ = linux_munmap(addr, aligned_size);
            return None;
        }
        mapped_until += PAGE_SIZE;
    }
    Some(addr)
}

fn align_page_up(size: u64) -> Option<u64> {
    const PAGE_SIZE: u64 = 4096;
    size.checked_add(PAGE_SIZE - 1)
        .map(|v| v & !(PAGE_SIZE - 1))
}

fn ranges_overlap(a_start: u64, a_end: u64, b_start: u64, b_end: u64) -> bool {
    a_start < b_end && b_start < a_end
}

fn linux_is_err(ret: u64) -> bool {
    (ret as i64) < 0
}

pub(super) fn linux_fill_mapping_from_fd(
    fd: u32,
    addr: u64,
    len: u64,
    offset: u64,
) -> Result<(), i32> {
    let mut copied = 0usize;
    let len = len as usize;
    while copied < len {
        let dst = addr + copied as u64;
        let want = (len - copied).min(FILE_MAP_COPY_CHUNK);
        let n = linux_read_fd_at(fd, dst, want, offset + copied as u64)?;
        if n == 0 {
            break;
        }
        copied += n;
    }
    Ok(())
}

pub(super) fn linux_read_fd_at(
    fd: u32,
    buf_ptr: u64,
    len: usize,
    offset: u64,
) -> Result<usize, i32> {
    // off_t is signed; positions beyond i64::MAX are invalid.
    if offset > i64::MAX as u64 {
        return Err(EINVAL);
    }
    if len == 0 {
        return Ok(0);
    }
    if !handlers::helpers::is_user_range_accessible(buf_ptr, len as u64) {
        return Err(EFAULT);
    }

    let entry = crate::task::scheduler::current_fd_get(fd).ok_or(EBADF)?;
    let global_id = match entry.kind {
        crate::fs::fd_table::FdKind::File { global_id } => global_id,
        _ => return Err(EBADF),
    };

    let mut total = 0usize;
    while total < len {
        let chunk_len = (len - total).min(FILE_MAP_COPY_CHUNK);
        let mut chunk = alloc::vec![0u8; chunk_len];
        let chunk_offset = offset.checked_add(total as u64).ok_or(EINVAL)?;
        let n = linux_read_global_chunk_at(global_id, chunk_offset, &mut chunk)?;
        if n == 0 {
            break;
        }
        if !handlers::helpers::copy_to_user_bytes(
            buf_ptr.wrapping_add(total as u64),
            &chunk[..n],
            FILE_MAP_COPY_CHUNK,
        ) {
            return if total > 0 { Ok(total) } else { Err(EFAULT) };
        }
        total += n;
        // Do NOT break on a short read: pread/mmap must fill up to `len` or
        // EOF. A short chunk from read_at (e.g. at a FAT-run boundary) is not
        // EOF; breaking here would leave a zero hole in the mapped/pread'd
        // region and corrupt data deep in the file (e.g. an ar member header).
        // The `n == 0` check above is the only legitimate stop (true EOF).
    }
    Ok(total)
}

fn linux_read_global_chunk_at(global_id: u32, offset: u64, out: &mut [u8]) -> Result<usize, i32> {
    match crate::fs::vfs::read_at(global_id, offset, out) {
        Ok(n) => return Ok(n),
        Err(crate::fs::vfs::FsError::NotSupported) => {}
        Err(e) => return Err(fs_errno(e)),
    }
    let old_pos = crate::fs::vfs::lseek(global_id, 0, 1).map_err(fs_errno)?;
    crate::fs::vfs::lseek(global_id, offset as i64, 0).map_err(fs_errno)?;
    let read_result = crate::fs::vfs::read(global_id, out).map_err(fs_errno);
    let _ = crate::fs::vfs::lseek(global_id, old_pos as i64, 0);
    read_result
}
