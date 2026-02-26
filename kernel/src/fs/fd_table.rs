//! Per-process file descriptor table.
//!
//! Each thread/process gets its own FdTable mapping local FD numbers (0..MAX_FDS-1)
//! to kernel resources: VFS global file slots, anonymous pipe endpoints, or nothing.
//! Fixed-size array — no heap allocation, trivially cloneable for fork().

/// Maximum number of file descriptors per process.
pub const MAX_FDS: usize = 256;

/// What kind of kernel resource a file descriptor points to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FdKind {
    /// Empty slot — no file descriptor here.
    None,
    /// A VFS open file. `global_id` is the index into the global open_files table.
    File { global_id: u32 },
    /// Read end of an anonymous pipe.
    PipeRead { pipe_id: u32 },
    /// Write end of an anonymous pipe.
    PipeWrite { pipe_id: u32 },
    /// Terminal I/O — uses legacy stdout_pipe / stdin_pipe on the Thread.
    /// Reserves fd 0/1/2 so pipe()/open() start at fd 3.
    Tty,
}

/// Per-FD flags (POSIX).
#[derive(Debug, Clone, Copy)]
pub struct FdFlags {
    /// Close this FD on exec().
    pub cloexec: bool,
}

impl Default for FdFlags {
    fn default() -> Self {
        FdFlags { cloexec: false }
    }
}

/// A single entry in the per-process FD table.
#[derive(Debug, Clone, Copy)]
pub struct FdEntry {
    pub kind: FdKind,
    pub flags: FdFlags,
}

impl FdEntry {
    pub const EMPTY: FdEntry = FdEntry {
        kind: FdKind::None,
        flags: FdFlags { cloexec: false },
    };
}

/// Per-process file descriptor table. Fixed-size array, no heap allocation.
/// Size: MAX_FDS * size_of::<FdEntry>() = 256 * 12 = 3072 bytes.
#[derive(Clone)]
pub struct FdTable {
    pub entries: [FdEntry; MAX_FDS],
}

impl FdTable {
    /// Create an empty FD table (all slots FdKind::None).
    pub const fn new() -> Self {
        FdTable {
            entries: [FdEntry::EMPTY; MAX_FDS],
        }
    }

    /// Allocate the lowest available FD and assign it the given kind.
    /// Returns the FD number, or None if the table is full.
    pub fn alloc(&mut self, kind: FdKind) -> Option<u32> {
        for (i, entry) in self.entries.iter_mut().enumerate() {
            if matches!(entry.kind, FdKind::None) {
                entry.kind = kind;
                entry.flags = FdFlags::default();
                return Some(i as u32);
            }
        }
        None
    }

    /// Allocate the lowest available FD >= min_fd and assign it the given kind.
    /// Returns the FD number, or None if no slot is available.
    pub fn alloc_above(&mut self, min_fd: u32, kind: FdKind) -> Option<u32> {
        let start = min_fd as usize;
        if start >= MAX_FDS {
            return None;
        }
        for i in start..MAX_FDS {
            if matches!(self.entries[i].kind, FdKind::None) {
                self.entries[i].kind = kind;
                self.entries[i].flags = FdFlags::default();
                return Some(i as u32);
            }
        }
        None
    }

    /// Place a resource at a specific FD slot. Returns true on success.
    /// If the slot is already occupied, the caller must close it first.
    pub fn alloc_at(&mut self, fd: u32, kind: FdKind) -> bool {
        if (fd as usize) >= MAX_FDS {
            return false;
        }
        self.entries[fd as usize].kind = kind;
        self.entries[fd as usize].flags = FdFlags::default();
        true
    }

    /// Close an FD slot. Returns the old FdKind for cleanup (decref etc.),
    /// or None if the slot was already empty or out of range.
    pub fn close(&mut self, fd: u32) -> Option<FdKind> {
        if (fd as usize) >= MAX_FDS {
            return None;
        }
        let old = self.entries[fd as usize].kind;
        if matches!(old, FdKind::None) {
            return None;
        }
        self.entries[fd as usize] = FdEntry::EMPTY;
        Some(old)
    }

    /// Get the entry for a given FD. Returns None if empty or out of range.
    pub fn get(&self, fd: u32) -> Option<&FdEntry> {
        if (fd as usize) >= MAX_FDS {
            return None;
        }
        let entry = &self.entries[fd as usize];
        if matches!(entry.kind, FdKind::None) {
            None
        } else {
            Some(entry)
        }
    }

    /// Duplicate old_fd to new_fd. The caller must handle closing new_fd first
    /// if it was open, and incrementing refcounts on the underlying resource.
    /// Returns true on success, false if old_fd is invalid.
    pub fn dup2(&mut self, old_fd: u32, new_fd: u32) -> bool {
        if (old_fd as usize) >= MAX_FDS || (new_fd as usize) >= MAX_FDS {
            return false;
        }
        if matches!(self.entries[old_fd as usize].kind, FdKind::None) {
            return false;
        }
        // Copy kind, but dup2 clears CLOEXEC on the new FD per POSIX
        self.entries[new_fd as usize].kind = self.entries[old_fd as usize].kind;
        self.entries[new_fd as usize].flags = FdFlags::default();
        true
    }

    /// Set or clear the CLOEXEC flag on an FD.
    pub fn set_cloexec(&mut self, fd: u32, cloexec: bool) {
        if (fd as usize) < MAX_FDS {
            self.entries[fd as usize].flags.cloexec = cloexec;
        }
    }

    /// Close all FDs with CLOEXEC set. Returns a list of old FdKinds for cleanup.
    /// Uses a fixed-size buffer to avoid heap allocation.
    pub fn close_cloexec(&mut self, out: &mut [FdKind; MAX_FDS]) -> usize {
        let mut count = 0;
        for entry in self.entries.iter_mut() {
            if entry.flags.cloexec && !matches!(entry.kind, FdKind::None) {
                if count < MAX_FDS {
                    out[count] = entry.kind;
                    count += 1;
                }
                *entry = FdEntry::EMPTY;
            }
        }
        count
    }

    /// Close all open FDs. Returns a list of old FdKinds for cleanup.
    pub fn close_all(&mut self, out: &mut [FdKind; MAX_FDS]) -> usize {
        let mut count = 0;
        for entry in self.entries.iter_mut() {
            if !matches!(entry.kind, FdKind::None) {
                if count < MAX_FDS {
                    out[count] = entry.kind;
                    count += 1;
                }
                *entry = FdEntry::EMPTY;
            }
        }
        count
    }

    /// Iterate all non-empty entries (for fork refcount incrementing).
    pub fn iter_open(&self) -> impl Iterator<Item = &FdEntry> {
        self.entries.iter().filter(|e| !matches!(e.kind, FdKind::None))
    }
}
