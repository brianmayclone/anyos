use super::*;

use crate::sync::spinlock::Spinlock;

const DEFAULT_FB_WIDTH: u32 = 1024;
const DEFAULT_FB_HEIGHT: u32 = 768;
const BYTES_PER_PIXEL: u32 = 4;
const LXEFB_FORMAT_XRGB8888: u32 = 1;
const FB_COPY_CHUNK: usize = 16 * 1024;

const FBIOGET_VSCREENINFO: u64 = 0x4600;
const FBIOPUT_VSCREENINFO: u64 = 0x4601;
const FBIOGET_FSCREENINFO: u64 = 0x4602;
const FBIOGETCMAP: u64 = 0x4604;
const FBIOPUTCMAP: u64 = 0x4605;
const FBIOPAN_DISPLAY: u64 = 0x4606;
const FBIOBLANK: u64 = 0x4611;
const FBIO_WAITFORVSYNC: u64 = 0x4680;

/// anyOS-private helper for a future lxefb host/Wayland bridge.
///
/// Writes:
///   magic/version/shm_id/width/height/stride/size/format as u32[8].
pub(super) const LXEFB_GET_SHM_INFO: u64 = 0x46F0;

#[derive(Clone, Copy)]
pub(super) struct LinuxFbInfo {
    pub shm_id: u32,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub size: u32,
    pub format: u32,
}

static LINUX_FB: Spinlock<Option<LinuxFbInfo>> = Spinlock::new(None);

pub(super) fn linux_fb_is_path(path: &str) -> bool {
    path == "/dev/fb0" || path == "/dev/fb/0"
}

pub(super) fn linux_fb_open(linux_flags: u64) -> u64 {
    const LINUX_O_DIRECTORY: u64 = 0o200000;
    const LINUX_O_CLOEXEC: u64 = 0o2000000;

    if (linux_flags & LINUX_O_DIRECTORY) != 0 {
        return linux_err(ENOTDIR);
    }
    if ensure_fb().is_none() {
        return linux_err(ENOMEM);
    }

    let fd = match crate::task::scheduler::current_fd_alloc(
        crate::fs::fd_table::FdKind::LinuxFramebuffer { position: 0 },
    ) {
        Some(fd) => fd,
        None => return linux_err(EBADF),
    };
    if (linux_flags & LINUX_O_CLOEXEC) != 0 {
        crate::task::scheduler::current_fd_set_cloexec(fd, true);
    }
    crate::serial_verbose_println!("lxe linux fb: open /dev/fb0 -> fd={}", fd);
    fd as u64
}

pub(super) fn linux_fb_stat(stat_ptr: u64) -> u64 {
    let Some(info) = ensure_fb() else {
        return linux_err(ENOENT);
    };
    write_linux_stat(stat_ptr, 0, 0xFB0, 2, info.size as u64, 0, 0, 0o666, 0);
    0
}

pub(super) fn linux_fb_statx(statx_ptr: u64) -> u64 {
    let Some(info) = ensure_fb() else {
        return linux_err(ENOENT);
    };
    write_linux_statx(statx_ptr, 0, 0xFB0, 2, info.size as u64, 0, 0, 0o666, 0);
    0
}

pub(super) fn linux_fb_size() -> u64 {
    ensure_fb().map(|info| info.size as u64).unwrap_or(0)
}

pub(super) fn linux_fb_read(fd: u32, position: u32, buf_ptr: u64, len: u64) -> u64 {
    let n = match linux_fb_copy_to_user(position as u64, buf_ptr, len) {
        Ok(n) => n,
        Err(errno) => return linux_err(errno),
    };
    let next = position.saturating_add(n as u32);
    crate::task::scheduler::current_fd_set_linux_fb_position(fd, next);
    n as u64
}

pub(super) fn linux_fb_write(fd: u32, position: u32, buf_ptr: u64, len: u64) -> u64 {
    let n = match linux_fb_copy_from_user(position as u64, buf_ptr, len) {
        Ok(n) => n,
        Err(errno) => return linux_err(errno),
    };
    let next = position.saturating_add(n as u32);
    crate::task::scheduler::current_fd_set_linux_fb_position(fd, next);
    n as u64
}

pub(super) fn linux_fb_pread(buf_ptr: u64, len: u64, offset: u64) -> u64 {
    match linux_fb_copy_to_user(offset, buf_ptr, len) {
        Ok(n) => n as u64,
        Err(errno) => linux_err(errno),
    }
}

pub(super) fn linux_fb_pwrite(buf_ptr: u64, len: u64, offset: u64) -> u64 {
    match linux_fb_copy_from_user(offset, buf_ptr, len) {
        Ok(n) => n as u64,
        Err(errno) => linux_err(errno),
    }
}

pub(super) fn linux_fb_lseek(fd: u32, position: u32, offset: u64, whence: u64) -> u64 {
    let size = linux_fb_size() as i64;
    let base = match whence {
        0 => 0,
        1 => position as i64,
        2 => size,
        _ => return linux_err(EINVAL),
    };
    let next = base.saturating_add(offset as i64);
    if next < 0 || next > u32::MAX as i64 {
        return linux_err(EINVAL);
    }
    if !crate::task::scheduler::current_fd_set_linux_fb_position(fd, next as u32) {
        return linux_err(EBADF);
    }
    next as u64
}

pub(super) fn linux_fb_mmap(_addr: u64, len: u64, prot: u64, flags: u64, offset: u64) -> u64 {
    let map_type = flags & 0x3;
    let fixed = (flags & LINUX_MAP_FIXED) != 0;
    if fixed || map_type != LINUX_MAP_SHARED || (prot & LINUX_PROT_WRITE) == 0 {
        return linux_err(EINVAL);
    }
    if offset != 0 {
        return linux_err(EINVAL);
    }

    let Some(info) = ensure_fb() else {
        return linux_err(ENOMEM);
    };
    if len == 0 || len > info.size as u64 {
        return linux_err(EINVAL);
    }

    let mapped = crate::ipc::shared_memory::map_into_current(info.shm_id);
    if mapped == 0 {
        return linux_err(ENOMEM);
    }
    crate::serial_verbose_println!(
        "lxe linux fb: mmap shm={} len={:#x} -> {:#x}",
        info.shm_id,
        len,
        mapped
    );
    mapped
}

pub(super) fn linux_fb_munmap(addr: u64, len: u64) -> bool {
    let Some(info) = ensure_fb() else {
        return false;
    };
    crate::ipc::shared_memory::unmap_from_current_by_addr(info.shm_id, addr, len)
}

pub(super) fn linux_fb_ioctl(fd: u32, request: u64, arg: u64) -> Option<u64> {
    let entry = crate::task::scheduler::current_fd_get(fd)?;
    if !matches!(
        entry.kind,
        crate::fs::fd_table::FdKind::LinuxFramebuffer { .. }
    ) {
        return None;
    }

    Some(match request {
        FBIOGET_FSCREENINFO => {
            if arg == 0 || !handlers::helpers::is_user_range_accessible(arg, 80) {
                linux_err(EFAULT)
            } else if let Some(info) = ensure_fb() {
                write_fix_screeninfo(arg, info);
                0
            } else {
                linux_err(ENODEV)
            }
        }
        FBIOGET_VSCREENINFO | FBIOPUT_VSCREENINFO => {
            if arg == 0 || !handlers::helpers::is_user_range_accessible(arg, 160) {
                linux_err(EFAULT)
            } else if let Some(info) = ensure_fb() {
                write_var_screeninfo(arg, info);
                0
            } else {
                linux_err(ENODEV)
            }
        }
        FBIOGETCMAP | FBIOPUTCMAP | FBIOPAN_DISPLAY | FBIOBLANK | FBIO_WAITFORVSYNC => 0,
        LXEFB_GET_SHM_INFO => {
            if arg == 0 || !handlers::helpers::is_user_range_accessible(arg, 32) {
                linux_err(EFAULT)
            } else if let Some(info) = ensure_fb() {
                unsafe {
                    write_u32(arg, 0, 0x4C58_4642); // "LXFB"
                    write_u32(arg, 4, 1);
                    write_u32(arg, 8, info.shm_id);
                    write_u32(arg, 12, info.width);
                    write_u32(arg, 16, info.height);
                    write_u32(arg, 20, info.stride);
                    write_u32(arg, 24, info.size);
                    write_u32(arg, 28, info.format);
                }
                0
            } else {
                linux_err(ENODEV)
            }
        }
        _ => linux_err(ENOTTY),
    })
}

fn ensure_fb() -> Option<LinuxFbInfo> {
    let (width, height) = framebuffer_dimensions();
    let stride = width.checked_mul(BYTES_PER_PIXEL)?;
    let size = stride.checked_mul(height)?;

    let mut guard = LINUX_FB.lock();
    if let Some(info) = *guard {
        if info.width == width
            && info.height == height
            && info.stride == stride
            && info.size == size
            && crate::ipc::shared_memory::region_size(info.shm_id) == size as usize
        {
            return Some(info);
        }
    }

    let shm_id = crate::ipc::shared_memory::create_kernel(size as usize)?;
    let info = LinuxFbInfo {
        shm_id,
        width,
        height,
        stride,
        size,
        format: LXEFB_FORMAT_XRGB8888,
    };
    *guard = Some(info);
    crate::serial_verbose_println!(
        "lxe linux fb: backing shm={} {}x{} stride={} size={}",
        shm_id,
        width,
        height,
        stride,
        size
    );
    Some(info)
}

fn framebuffer_dimensions() -> (u32, u32) {
    if let Some(fb) = crate::drivers::framebuffer::info() {
        if fb.width != 0 && fb.height != 0 {
            return (fb.width.min(0xFFFF), fb.height.min(0xFFFF));
        }
    }
    (DEFAULT_FB_WIDTH, DEFAULT_FB_HEIGHT)
}

fn map_fb_for_current() -> Result<(LinuxFbInfo, u64), i32> {
    let info = ensure_fb().ok_or(ENOMEM)?;
    let addr = crate::ipc::shared_memory::map_into_current(info.shm_id);
    if addr == 0 {
        return Err(ENOMEM);
    }
    Ok((info, addr))
}

fn linux_fb_copy_to_user(offset: u64, buf_ptr: u64, len: u64) -> Result<usize, i32> {
    if len == 0 {
        return Ok(0);
    }
    if buf_ptr == 0 || len > u32::MAX as u64 {
        return Err(EFAULT);
    }
    let (info, fb_addr) = map_fb_for_current()?;
    if offset >= info.size as u64 {
        return Ok(0);
    }
    let n = core::cmp::min(len, info.size as u64 - offset) as usize;
    let src = (fb_addr + offset) as *const u8;
    let bytes = unsafe { core::slice::from_raw_parts(src, n) };
    if !handlers::helpers::copy_to_user_bytes(buf_ptr, bytes, n) {
        return Err(EFAULT);
    }
    Ok(n)
}

fn linux_fb_copy_from_user(offset: u64, buf_ptr: u64, len: u64) -> Result<usize, i32> {
    if len == 0 {
        return Ok(0);
    }
    if buf_ptr == 0 || len > u32::MAX as u64 {
        return Err(EFAULT);
    }
    let (info, fb_addr) = map_fb_for_current()?;
    if offset >= info.size as u64 {
        return Ok(0);
    }

    let mut total = 0usize;
    let max_len = core::cmp::min(len, info.size as u64 - offset) as usize;
    while total < max_len {
        let chunk_len = core::cmp::min(FB_COPY_CHUNK, max_len - total);
        let Some(chunk) = handlers::helpers::copy_user_bytes(
            buf_ptr.wrapping_add(total as u64),
            chunk_len,
            FB_COPY_CHUNK,
        ) else {
            return if total > 0 { Ok(total) } else { Err(EFAULT) };
        };
        unsafe {
            core::ptr::copy_nonoverlapping(
                chunk.as_ptr(),
                (fb_addr + offset + total as u64) as *mut u8,
                chunk_len,
            );
        }
        total += chunk_len;
    }
    Ok(total)
}

fn write_fix_screeninfo(ptr: u64, info: LinuxFbInfo) {
    unsafe {
        core::ptr::write_bytes(ptr as *mut u8, 0, 80);
        let id = b"anyOS lxefb\0";
        core::ptr::copy_nonoverlapping(id.as_ptr(), ptr as *mut u8, id.len());
        write_u64(ptr, 16, 0);
        write_u32(ptr, 24, info.size);
        write_u32(ptr, 28, 0); // FB_TYPE_PACKED_PIXELS
        write_u32(ptr, 32, 0);
        write_u32(ptr, 36, 2); // FB_VISUAL_TRUECOLOR
        write_u16(ptr, 40, 0);
        write_u16(ptr, 42, 0);
        write_u16(ptr, 44, 0);
        write_u32(ptr, 48, info.stride);
        write_u64(ptr, 56, 0);
        write_u32(ptr, 64, 0);
        write_u32(ptr, 68, 0); // FB_ACCEL_NONE
        write_u16(ptr, 72, 0);
    }
}

fn write_var_screeninfo(ptr: u64, info: LinuxFbInfo) {
    unsafe {
        core::ptr::write_bytes(ptr as *mut u8, 0, 160);
        write_u32(ptr, 0, info.width);
        write_u32(ptr, 4, info.height);
        write_u32(ptr, 8, info.width);
        write_u32(ptr, 12, info.height);
        write_u32(ptr, 16, 0);
        write_u32(ptr, 20, 0);
        write_u32(ptr, 24, 32);
        write_u32(ptr, 28, 0);

        write_bitfield(ptr + 32, 16, 8);
        write_bitfield(ptr + 44, 8, 8);
        write_bitfield(ptr + 56, 0, 8);
        write_bitfield(ptr + 68, 24, 8);

        write_u32(ptr, 80, 0);
        write_u32(ptr, 84, 0);
        write_u32(ptr, 88, 0xFFFF_FFFF);
        write_u32(ptr, 92, 0xFFFF_FFFF);
        write_u32(ptr, 96, 0);
        write_u32(ptr, 100, 0);
        write_u32(ptr, 132, 0); // FB_VMODE_NONINTERLACED
        write_u32(ptr, 136, 0);
        write_u32(ptr, 140, 0);
    }
}

unsafe fn write_bitfield(ptr: u64, offset: u32, length: u32) {
    write_u32(ptr, 0, offset);
    write_u32(ptr, 4, length);
    write_u32(ptr, 8, 0);
}
