use super::*;

pub(super) fn linux_brk(new_brk: u64) -> u64 {
    let current = crate::task::scheduler::current_thread_brk();
    if new_brk == 0 {
        crate::serial_verbose_println!("licof linux brk: query -> {:#x}", current);
        return current;
    }
    let delta = new_brk as i64 - current as i64;
    let old = handlers::sys_sbrk_u64(delta);
    if old == u64::MAX {
        crate::serial_verbose_println!(
            "licof linux brk: failed current={:#x} requested={:#x} delta={}",
            current,
            new_brk,
            delta
        );
        current
    } else {
        let updated = crate::task::scheduler::current_thread_brk();
        crate::serial_verbose_println!(
            "licof linux brk: current={:#x} requested={:#x} delta={} -> {:#x}",
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
            "licof linux mmap: reject zero len addr={:#x} prot={:#x} flags={:#x} fd={:#x} off={:#x}",
            addr,
            prot,
            flags,
            fd,
            offset
        );
        return linux_err(EINVAL);
    }
    let anonymous = (flags & LINUX_MAP_ANONYMOUS) != 0;
    let private = (flags & LINUX_MAP_PRIVATE) != 0;
    let fixed = (flags & LINUX_MAP_FIXED) != 0;
    if !private {
        crate::serial_verbose_println!(
            "licof linux mmap: reject !private addr={:#x} len={:#x} prot={:#x} flags={:#x} fd={:#x} off={:#x}",
            addr,
            len,
            prot,
            flags,
            fd,
            offset
        );
        return linux_err(ENOSYS);
    }
    if anonymous && !linux_fd_is_minus_one(fd) {
        crate::serial_verbose_println!(
            "licof linux mmap: reject anon fd addr={:#x} len={:#x} prot={:#x} flags={:#x} fd={:#x} off={:#x}",
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
            "licof linux mmap: reject file fd addr={:#x} len={:#x} prot={:#x} flags={:#x} fd={:#x} off={:#x}",
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
                "licof linux mmap: reject fixed-null len={:#x} prot={:#x} flags={:#x} fd={:#x} off={:#x}",
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
                    "licof linux mmap: fixed failed addr={:#x} len={:#x} prot={:#x} flags={:#x} fd={:#x} off={:#x}",
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
        handlers::sys_mmap_u64(len)
    };
    if mapped == u64::MAX {
        crate::serial_verbose_println!(
            "licof linux mmap: alloc failed addr={:#x} len={:#x} prot={:#x} flags={:#x} fd={:#x} off={:#x}",
            addr,
            len,
            prot,
            flags,
            fd,
            offset
        );
        return linux_err(ENOMEM);
    }

    if !anonymous {
        if let Err(errno) = linux_fill_mapping_from_fd(fd as u32, mapped, len, offset) {
            let _ = handlers::sys_munmap_u64(mapped, len);
            crate::serial_verbose_println!(
                "licof linux mmap: fill failed mapped={:#x} len={:#x} prot={:#x} flags={:#x} fd={:#x} off={:#x} errno={}",
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
    }
    crate::serial_verbose_println!(
        "licof linux mmap: ok addr={:#x} len={:#x} prot={:#x} flags={:#x} fd={:#x} off={:#x} -> {:#x}",
        addr,
        len,
        prot,
        flags,
        fd,
        offset,
        mapped
    );
    mapped
}

pub(super) fn linux_fd_is_minus_one(fd: u64) -> bool {
    fd == u64::MAX || fd == u32::MAX as u64
}

pub(super) fn linux_munmap(addr: u64, len: u64) -> u64 {
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

pub(super) fn linux_mprotect(_addr: u64, len: u64, _prot: u64) -> u64 {
    if len == 0 {
        linux_err(EINVAL)
    } else {
        0
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
                "licof linux arch_prctl: tid={} SET_FS {:#x}",
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

pub(super) fn linux_fill_mapping_from_fd(
    fd: u32,
    addr: u64,
    len: u64,
    offset: u64,
) -> Result<(), i32> {
    let mut copied = 0usize;
    let len = len as usize;
    let mut tmp = [0u8; 4096];
    while copied < len {
        let want = (len - copied).min(tmp.len());
        let n = linux_read_fd_at(fd, tmp.as_mut_ptr() as u64, want, offset + copied as u64)?;
        if n == 0 {
            break;
        }
        unsafe {
            core::ptr::copy_nonoverlapping(tmp.as_ptr(), (addr as usize + copied) as *mut u8, n);
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
    if offset > i32::MAX as u64 {
        return Err(EINVAL);
    }
    let entry = crate::task::scheduler::current_fd_get(fd).ok_or(EBADF)?;
    let global_id = match entry.kind {
        crate::fs::fd_table::FdKind::File { global_id } => global_id,
        _ => return Err(EBADF),
    };
    let (_file_type, _size, old_pos, _mtime) =
        crate::fs::vfs::fstat(global_id).map_err(fs_errno)?;
    crate::fs::vfs::lseek(global_id, offset as i32, 0).map_err(fs_errno)?;
    let read_result = unsafe {
        let out = core::slice::from_raw_parts_mut(buf_ptr as *mut u8, len);
        crate::fs::vfs::read(global_id, out).map_err(fs_errno)
    };
    let _ = crate::fs::vfs::lseek(global_id, old_pos as i32, 0);
    read_result
}
