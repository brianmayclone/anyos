//! Filesystem helpers for AC — thin wrappers around `anyos_std::fs`
//! plus higher-level operations (copy, move, delete tree, mkdir).

// ─── Directory entry ─────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Dir,
    File,
    Symlink,
    Unknown,
}

#[derive(Clone, Copy)]
pub struct DirEntry {
    /// File/dir name (raw bytes, up to 56 chars).
    pub name: [u8; 56],
    pub name_len: usize,
    pub kind: EntryKind,
    /// File size in bytes (0 for directories).
    pub size: u64,
    /// True if the file has any execute bit set (mode & 0o111 != 0).
    pub is_exec: bool,
}

impl DirEntry {
    pub const EMPTY: Self = Self {
        name: [0u8; 56],
        name_len: 0,
        kind: EntryKind::Unknown,
        size: 0,
        is_exec: false,
    };

    pub fn name_str(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("???")
    }

    pub fn is_dir(&self) -> bool {
        matches!(self.kind, EntryKind::Dir)
    }

    pub fn is_dotdot(&self) -> bool {
        &self.name[..self.name_len] == b".."
    }

    pub fn is_dot(&self) -> bool {
        &self.name[..self.name_len] == b"."
    }
}

// ─── Path utilities ───────────────────────────────────────────────────────────

/// Maximum path length.
pub const MAX_PATH: usize = 512;

/// Join two path components. Returns a stack-allocated ([u8; MAX_PATH], len).
pub fn path_join(base: &str, name: &str) -> ([u8; MAX_PATH], usize) {
    let mut buf = [0u8; MAX_PATH];
    let mut len = 0usize;

    let base = base.trim_end_matches('/');
    for &b in base.as_bytes() {
        if len < MAX_PATH { buf[len] = b; len += 1; }
    }
    if len < MAX_PATH { buf[len] = b'/'; len += 1; }
    for &b in name.as_bytes() {
        if len < MAX_PATH { buf[len] = b; len += 1; }
    }
    (buf, len)
}

/// Return parent directory of a path.
pub fn path_parent(path: &str) -> &str {
    let p = path.trim_end_matches('/');
    match p.rfind('/') {
        Some(0) => "/",
        Some(i) => &p[..i],
        None    => ".",
    }
}

/// Return the file name component of a path.
pub fn path_basename(path: &str) -> &str {
    let p = path.trim_end_matches('/');
    match p.rfind('/') {
        Some(i) => &p[i + 1..],
        None    => p,
    }
}

/// Convert (buf, len) pair from `path_join` to &str.
pub fn buf_to_str(buf: &[u8; MAX_PATH], len: usize) -> &str {
    core::str::from_utf8(&buf[..len]).unwrap_or("/")
}

// ─── Directory listing ────────────────────────────────────────────────────────

pub const MAX_ENTRIES: usize = 512;

/// Raw readdir buffer: each entry is 64 bytes.
/// Layout per entry: [type:u8, name_len:u8, pad:u16, size:u32, name:56bytes]
const RAW_ENTRY_SIZE: usize = 64;
// 2048 entries × 64 bytes = 131072 bytes. Too large for stack.
// Use a smaller initial chunk: 256 entries.
const RAW_CHUNK: usize = MAX_ENTRIES * RAW_ENTRY_SIZE; // 512×64 = 32 KB

/// Read a directory into `out[0..count]`. Returns count.
pub fn read_dir(path: &str, out: &mut [DirEntry; MAX_ENTRIES]) -> usize {
    // Use a static buffer to avoid a large stack allocation.
    // SAFETY: AC is single-threaded; this function is never re-entered.
    static mut RAW_BUF: [u8; RAW_CHUNK] = [0u8; RAW_CHUNK];
    let raw: &mut [u8] = unsafe { &mut RAW_BUF };
    let n = anyos_std::fs::readdir(path, raw);
    if n == 0 || n == u32::MAX { return 0; }

    // Prepend ".." for all directories except root "/"
    let add_dotdot = path != "/" && !path.is_empty();
    let start = if add_dotdot {
        out[0] = {
            let mut e = DirEntry::EMPTY;
            e.name[0] = b'.';
            e.name[1] = b'.';
            e.name_len = 2;
            e.kind = EntryKind::Dir;
            e
        };
        1
    } else {
        0
    };

    let count = (n as usize).min(MAX_ENTRIES - start).min(RAW_CHUNK / RAW_ENTRY_SIZE);
    for i in 0..count {
        let off = i * RAW_ENTRY_SIZE;
        // Kernel layout: 0=Regular, 1=Directory, 2=Device
        // Symlink flag is in byte [off+2], bit 0
        let is_symlink = raw[off + 2] & 1 != 0;
        let kind = if is_symlink {
            EntryKind::Symlink
        } else {
            match raw[off] {
                1 => EntryKind::Dir,
                0 => EntryKind::File,
                _ => EntryKind::Unknown,
            }
        };
        let name_len = raw[off + 1] as usize;
        let name_len = name_len.min(55);
        let size = u32::from_le_bytes([raw[off+4], raw[off+5], raw[off+6], raw[off+7]]) as u64;

        let mut entry = DirEntry::EMPTY;
        entry.name[..name_len].copy_from_slice(&raw[off + 8..off + 8 + name_len]);
        entry.name_len = name_len;
        entry.kind = kind;
        entry.size = size;

        // Detect executable bit for regular files via stat.
        if matches!(kind, EntryKind::File) {
            let (jp, jl) = path_join(path, entry.name_str());
            let jpath = buf_to_str(&jp, jl);
            let mut sbuf = [0u32; 7];
            if anyos_std::fs::stat(jpath, &mut sbuf) == 0 {
                let mode = sbuf[5];
                entry.is_exec = mode & 0o111 != 0;
            }
        }

        out[start + i] = entry;
    }
    start + count
}

// ─── Sort ────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SortMode {
    Name,
    Size,
    Kind,
}

/// Sort entries: dirs first (except ".." always index 0), then by mode.
pub fn sort_entries(entries: &mut [DirEntry], count: usize, mode: SortMode, reverse: bool) {
    if count == 0 { return; }

    // Move ".." to position 0
    let dotdot = entries[..count].iter().position(|e| e.is_dotdot());
    if let Some(di) = dotdot {
        if di != 0 { entries.swap(0, di); }
    }
    let start = if dotdot.is_some() { 1 } else { 0 };
    if count <= start + 1 { return; }

    // Insertion sort on entries[start..count]
    for i in (start + 1)..count {
        let mut j = i;
        while j > start {
            let a_dir = entries[j - 1].is_dir();
            let b_dir = entries[j].is_dir();
            let swap = match (a_dir, b_dir) {
                (true,  false) => false,
                (false, true)  => true,
                _ => {
                    let less = entry_less(&entries[j - 1], &entries[j], mode);
                    if reverse { less } else { !less }
                }
            };
            if swap { entries.swap(j - 1, j); j -= 1; } else { break; }
        }
    }
}

fn entry_less(a: &DirEntry, b: &DirEntry, mode: SortMode) -> bool {
    match mode {
        SortMode::Name => a.name[..a.name_len] < b.name[..b.name_len],
        SortMode::Size => a.size < b.size,
        SortMode::Kind => {
            let ak = kind_ord(&a.kind);
            let bk = kind_ord(&b.kind);
            if ak != bk { ak < bk } else { a.name[..a.name_len] < b.name[..b.name_len] }
        }
    }
}

fn kind_ord(k: &EntryKind) -> u8 {
    match k {
        EntryKind::Dir     => 0,
        EntryKind::Symlink => 1,
        EntryKind::File    => 2,
        EntryKind::Unknown => 3,
    }
}

// ─── File I/O ─────────────────────────────────────────────────────────────────

/// Copy a single file from `src` to `dst`. Returns true on success.
pub fn copy_file(src: &str, dst: &str) -> bool {
    let fd_src = anyos_std::fs::open(src, 0); // O_RDONLY
    if fd_src == u32::MAX { return false; }

    // O_WRITE=1 | O_CREATE=4 | O_TRUNC=8 = 13
    let fd_dst = anyos_std::fs::open(dst, 1 | 4 | 8);
    if fd_dst == u32::MAX {
        anyos_std::fs::close(fd_src);
        return false;
    }

    let mut ok = true;
    let mut buf = [0u8; 4096];
    loop {
        let n = anyos_std::fs::read(fd_src, &mut buf);
        if n == 0 { break; }
        if n == u32::MAX { ok = false; break; }
        let w = anyos_std::fs::write(fd_dst, &buf[..n as usize]);
        if w != n { ok = false; break; }
    }

    anyos_std::fs::close(fd_src);
    anyos_std::fs::close(fd_dst);
    ok
}

/// Delete a file.
pub fn delete_file(path: &str) -> bool {
    anyos_std::fs::unlink(path) == 0
}

/// Create a directory.
pub fn make_dir(path: &str) -> bool {
    anyos_std::fs::mkdir(path) == 0
}

/// Rename / move a path.
pub fn rename(src: &str, dst: &str) -> bool {
    anyos_std::fs::rename(src, dst) == 0
}

// ─── Stat ─────────────────────────────────────────────────────────────────────

/// Minimal stat result.
pub struct StatInfo {
    pub size: u64,
    pub is_dir: bool,
    pub is_symlink: bool,
}

/// Stat a path. Returns None on error.
/// stat_buf layout: [type:u32, size:u32, flags:u32, uid:u32, gid:u32, mode:u32, mtime:u32]
/// type: 0=file, 1=dir   flags: bit0=is_symlink
pub fn stat(path: &str) -> Option<StatInfo> {
    let mut buf = [0u32; 7];
    if anyos_std::fs::stat(path, &mut buf) == u32::MAX { return None; }
    let is_dir     = buf[0] == 1;
    let size       = buf[1] as u64;
    let is_symlink = buf[2] & 1 != 0;
    Some(StatInfo { size, is_dir, is_symlink })
}

// ─── Size formatting ──────────────────────────────────────────────────────────

pub fn fmt_size<'a>(buf: &'a mut [u8; 16], size: u64) -> &'a str {
    if size == 0 {
        buf[0] = b'0';
        return core::str::from_utf8(&buf[..1]).unwrap();
    }
    let (val10, suffix) = if size >= 1024 * 1024 * 1024 {
        (size / (1024 * 1024 * 1024 / 10), b'G')
    } else if size >= 1024 * 1024 {
        (size / (1024 * 1024 / 10), b'M')
    } else if size >= 1024 {
        (size / (1024 / 10), b'K')
    } else {
        (size * 10, b'B')
    };
    let whole = val10 / 10;
    let frac  = val10 % 10;
    let mut len = 0usize;
    let mut tmp = [0u8; 8];
    let mut ti = 8usize;
    let mut w = whole;
    if w == 0 { ti -= 1; tmp[ti] = b'0'; } else {
        while w > 0 { ti -= 1; tmp[ti] = b'0' + (w % 10) as u8; w /= 10; }
    }
    for &b in &tmp[ti..] { buf[len] = b; len += 1; }
    if suffix != b'B' && frac != 0 {
        buf[len] = b'.'; len += 1;
        buf[len] = b'0' + frac as u8; len += 1;
    }
    buf[len] = suffix; len += 1;
    core::str::from_utf8(&buf[..len]).unwrap_or("?")
}
