use super::handlers;

const HASH_MAX: usize = 64 * 1024;
const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

pub(super) fn path_interesting(path: &str) -> bool {
    path.contains("/var/lib/apt/lists")
        || path.contains("/tmp/lxediag")
        || path.contains("InRelease")
        || path.contains("Translation-")
        || path.contains("/Packages")
}

pub(super) fn trace_open(
    linux_path: &str,
    resolved_path: &str,
    fd: u32,
    global_id: u32,
    flags: u64,
) {
    if !path_interesting(linux_path) && !path_interesting(resolved_path) {
        return;
    }
    let (size, pos) = crate::fs::vfs::fstat(global_id)
        .map(|(_, size, pos, _)| (size, pos))
        .unwrap_or((0, 0));
    crate::serial_println!(
        "[lxe-trace] open tid={} fd={} global={} flags={:#x} size={} pos={} linux='{}' resolved='{}'",
        crate::task::scheduler::current_tid(),
        fd,
        global_id,
        flags,
        size,
        pos,
        linux_path,
        resolved_path
    );
}

pub(super) fn trace_file_io(
    op: &str,
    fd: u32,
    global_id: u32,
    requested: u64,
    ret: u64,
    buf_ptr: u64,
) {
    let path = match crate::fs::vfs::get_fd_path(global_id) {
        Ok(path) => path,
        Err(_) => return,
    };
    if !path_interesting(&path) {
        return;
    }

    let signed = ret as i64;
    let (size, pos) = crate::fs::vfs::fstat(global_id)
        .map(|(_, size, pos, _)| (size, pos))
        .unwrap_or((0, 0));
    if signed < 0 {
        crate::serial_println!(
            "[lxe-trace] {} tid={} fd={} global={} req={} ret={} size={} pos={} path='{}'",
            op,
            crate::task::scheduler::current_tid(),
            fd,
            global_id,
            requested,
            signed,
            size,
            pos,
            path
        );
        return;
    }

    let n = (ret as usize).min(requested.min(usize::MAX as u64) as usize);
    let (hash_len, hash) = user_hash(buf_ptr, n).unwrap_or((0, 0));
    crate::serial_println!(
        "[lxe-trace] {} tid={} fd={} global={} req={} ret={} size={} pos={} hash_len={} fnv={:016x} path='{}'",
        op,
        crate::task::scheduler::current_tid(),
        fd,
        global_id,
        requested,
        ret,
        size,
        pos,
        hash_len,
        hash,
        path
    );
}

pub(super) fn trace_fsync(op: &str, fd: u32, global_id: u32, ret: u64) {
    let path = match crate::fs::vfs::get_fd_path(global_id) {
        Ok(path) => path,
        Err(_) => return,
    };
    if !path_interesting(&path) {
        return;
    }
    crate::serial_println!(
        "[lxe-trace] {} tid={} fd={} global={} ret={} path='{}'",
        op,
        crate::task::scheduler::current_tid(),
        fd,
        global_id,
        ret as i64,
        path
    );
}

pub(super) fn trace_path_op(op: &str, old_path: &str, new_path: Option<&str>, ret: u64) {
    let interesting = path_interesting(old_path) || new_path.map(path_interesting).unwrap_or(false);
    if !interesting {
        return;
    }
    match new_path {
        Some(new_path) => crate::serial_println!(
            "[lxe-trace] {} tid={} ret={} old='{}' new='{}'",
            op,
            crate::task::scheduler::current_tid(),
            ret as i64,
            old_path,
            new_path
        ),
        None => crate::serial_println!(
            "[lxe-trace] {} tid={} ret={} path='{}'",
            op,
            crate::task::scheduler::current_tid(),
            ret as i64,
            old_path
        ),
    }
}

pub(super) fn trace_socket_io(
    op: &str,
    fd: u32,
    socket_id: u32,
    requested: u64,
    ret: u64,
    buf_ptr: u64,
) {
    let signed = ret as i64;
    if signed <= 0 {
        return;
    }
    let n = (ret as usize).min(requested.min(usize::MAX as u64) as usize);
    let (hash_len, hash) = user_hash(buf_ptr, n).unwrap_or((0, 0));
    crate::serial_println!(
        "[lxe-trace] {} tid={} fd={} sock={} req={} ret={} hash_len={} fnv={:016x}",
        op,
        crate::task::scheduler::current_tid(),
        fd,
        socket_id,
        requested,
        ret,
        hash_len,
        hash
    );
}

fn user_hash(ptr: u64, len: usize) -> Option<(usize, u64)> {
    if len == 0 {
        return Some((0, FNV_OFFSET));
    }
    let hash_len = len.min(HASH_MAX);
    if !handlers::helpers::is_user_range_accessible(ptr, hash_len as u64) {
        return None;
    }
    let data = unsafe { core::slice::from_raw_parts(ptr as *const u8, hash_len) };
    Some((hash_len, fnv(data)))
}

fn fnv(data: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET;
    for &b in data {
        hash ^= b as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}
