use super::*;

pub(super) fn write_linux_uts_field(base: u64, index: u64, value: &[u8]) {
    let len = value.len().min(64);
    unsafe {
        core::ptr::copy_nonoverlapping(value.as_ptr(), (base + index * 65) as *mut u8, len);
    }
}

pub(super) fn write_linux_stat(
    stat_ptr: u64,
    dev: u64,
    ino: u64,
    anyos_type: u32,
    size: u64,
    uid: u32,
    gid: u32,
    mode: u32,
    mtime: u32,
) {
    let file_type_bits = match anyos_type {
        1 => 0o040000,
        2 => 0o020000,
        3 => 0o120000,
        _ => 0o100000,
    };
    let full_mode = file_type_bits | (mode & 0o7777);
    unsafe {
        core::ptr::write_bytes(stat_ptr as *mut u8, 0, 144);
        write_u64(stat_ptr, 0, dev);
        write_u64(stat_ptr, 8, ino.max(1));
        write_u64(stat_ptr, 16, 1);
        write_u32(stat_ptr, 24, full_mode);
        write_u32(stat_ptr, 28, uid);
        write_u32(stat_ptr, 32, gid);
        write_u64(stat_ptr, 40, 0);
        write_u64(stat_ptr, 48, size);
        write_u64(stat_ptr, 56, 4096);
        write_u64(stat_ptr, 64, (size + 511) / 512);
        write_u64(stat_ptr, 72, mtime as u64);
        write_u64(stat_ptr, 88, mtime as u64);
        write_u64(stat_ptr, 104, mtime as u64);
    }
}

pub(super) fn write_linux_statx(
    statx_ptr: u64,
    dev: u64,
    ino: u64,
    anyos_type: u32,
    size: u64,
    uid: u32,
    gid: u32,
    mode: u32,
    mtime: u32,
) {
    const STATX_TYPE: u32 = 0x0001;
    const STATX_MODE: u32 = 0x0002;
    const STATX_NLINK: u32 = 0x0004;
    const STATX_UID: u32 = 0x0008;
    const STATX_GID: u32 = 0x0010;
    const STATX_ATIME: u32 = 0x0020;
    const STATX_MTIME: u32 = 0x0040;
    const STATX_CTIME: u32 = 0x0080;
    const STATX_INO: u32 = 0x0100;
    const STATX_SIZE: u32 = 0x0200;
    const STATX_BLOCKS: u32 = 0x0400;
    const STATX_BASIC_STATS: u32 = STATX_TYPE
        | STATX_MODE
        | STATX_NLINK
        | STATX_UID
        | STATX_GID
        | STATX_ATIME
        | STATX_MTIME
        | STATX_CTIME
        | STATX_INO
        | STATX_SIZE
        | STATX_BLOCKS;

    let file_type_bits = match anyos_type {
        1 => 0o040000,
        2 => 0o020000,
        3 => 0o120000,
        _ => 0o100000,
    };
    let full_mode = file_type_bits | (mode & 0o7777);
    let blocks = (size + 511) / 512;

    unsafe {
        core::ptr::write_bytes(statx_ptr as *mut u8, 0, 256);
        write_u32(statx_ptr, 0, STATX_BASIC_STATS);
        write_u32(statx_ptr, 4, 4096);
        write_u64(statx_ptr, 8, 0);
        write_u32(statx_ptr, 16, 1);
        write_u32(statx_ptr, 20, uid);
        write_u32(statx_ptr, 24, gid);
        write_u16(statx_ptr, 28, full_mode as u16);
        write_u64(statx_ptr, 32, ino.max(1));
        write_u64(statx_ptr, 40, size);
        write_u64(statx_ptr, 48, blocks);
        write_u64(statx_ptr, 56, 0);
        write_linux_statx_timestamp(statx_ptr + 64, mtime);
        write_linux_statx_timestamp(statx_ptr + 96, mtime);
        write_linux_statx_timestamp(statx_ptr + 112, mtime);
        write_u32(statx_ptr, 136, (dev >> 32) as u32);
        write_u32(statx_ptr, 140, dev as u32);
    }
}

unsafe fn write_linux_statx_timestamp(ptr: u64, sec: u32) {
    write_u64(ptr, 0, sec as u64);
    write_u32(ptr, 8, 0);
    write_u32(ptr, 12, 0);
}

pub(super) fn anyos_file_type(file_type: crate::fs::file::FileType) -> u32 {
    match file_type {
        crate::fs::file::FileType::Regular => 0,
        crate::fs::file::FileType::Directory => 1,
        crate::fs::file::FileType::Device => 2,
    }
}

pub(super) unsafe fn write_u32(base: u64, offset: u64, value: u32) {
    *((base + offset) as *mut u32) = value;
}

pub(super) unsafe fn write_u16(base: u64, offset: u64, value: u16) {
    *((base + offset) as *mut u16) = value;
}

pub(super) unsafe fn write_u64(base: u64, offset: u64, value: u64) {
    *((base + offset) as *mut u64) = value;
}

pub(super) unsafe fn read_u64(base: u64, offset: u64) -> u64 {
    *((base + offset) as *const u64)
}

pub(super) fn linux_dir_type(file_type: crate::fs::file::FileType) -> u8 {
    match file_type {
        crate::fs::file::FileType::Regular => 8,   // DT_REG
        crate::fs::file::FileType::Directory => 4, // DT_DIR
        crate::fs::file::FileType::Device => 2,    // DT_CHR
    }
}

pub(super) fn align_up(value: usize, alignment: usize) -> usize {
    (value + alignment - 1) & !(alignment - 1)
}

pub(super) fn map_open_flags(flags: u64) -> u32 {
    let mut out = 0u32;
    let accmode = flags & 0x3;
    if accmode == 1 || accmode == 2 {
        out |= 1;
    }
    if (flags & 0x40) != 0 {
        out |= 4;
    }
    if (flags & 0x200) != 0 {
        out |= 8;
    }
    if (flags & 0x400) != 0 {
        out |= 2;
    }
    if (flags & 0x80000) != 0 {
        out |= 0x10;
    }
    out
}

pub(super) fn anyos_u32_ret(ret: u32) -> u64 {
    if (ret as i32) < 0 {
        (ret as i32 as i64) as u64
    } else {
        ret as u64
    }
}

pub(super) fn linux_err(errno: i32) -> u64 {
    (-(errno as i64)) as u64
}
