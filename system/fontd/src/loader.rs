//! Font file loader — reads a font file from disk into a SHM region.

use anyos_std::{fs, ipc};

/// Maximum font file size (16 MiB — NotoColorEmoji is ~11 MiB).
const MAX_FONT_SIZE: u32 = 16 * 1024 * 1024;

/// Read a font file from disk and place it into a new SHM region.
/// Returns (shm_id, bytes_read) on success, None on failure.
/// The SHM remains mapped in fontd so it stays alive.
pub fn load_to_shm(path: &str) -> Option<(u32, u32)> {
    let fd = fs::open(path, 0);
    if fd == u32::MAX { return None; }

    let mut stat = [0u32; 4];
    if fs::fstat(fd, &mut stat) == u32::MAX {
        fs::close(fd);
        return None;
    }
    let file_size = stat[1];
    if file_size == 0 || file_size > MAX_FONT_SIZE {
        fs::close(fd);
        return None;
    }

    let shm_id = ipc::shm_create(file_size);
    if shm_id == 0 {
        fs::close(fd);
        return None;
    }

    let addr = ipc::shm_map(shm_id);
    if addr == 0 {
        ipc::shm_destroy(shm_id);
        fs::close(fd);
        return None;
    }

    let buf = unsafe { core::slice::from_raw_parts_mut(addr as *mut u8, file_size as usize) };
    let mut total = 0usize;
    while total < file_size as usize {
        let n = fs::read(fd, &mut buf[total..]) as usize;
        if n == 0 || n > file_size as usize { break; }
        total += n;
    }
    fs::close(fd);

    if total == 0 {
        ipc::shm_unmap(shm_id);
        ipc::shm_destroy(shm_id);
        return None;
    }

    Some((shm_id, total as u32))
}
