//! Panel state — one of the two file panels shown side-by-side in AC.
//!
//! Each panel maintains:
//! - current directory path
//! - list of directory entries (up to MAX_ENTRIES)
//! - scroll offset and cursor position
//! - sort mode and reverse flag
//! - a set of selected ("marked") entries

use crate::fs::{self, DirEntry, SortMode, MAX_ENTRIES, MAX_PATH, path_join, path_parent};

// ─── Panel ───────────────────────────────────────────────────────────────────

pub struct Panel {
    /// Current directory (null-terminated, stored as raw bytes).
    path_buf: [u8; MAX_PATH],
    path_len: usize,

    /// Directory entries.
    pub entries: [DirEntry; MAX_ENTRIES],
    pub entry_count: usize,

    /// Cursor position (index into `entries`).
    pub cursor: usize,

    /// Scroll offset (top-most visible entry index).
    pub scroll: usize,

    /// Sort configuration.
    pub sort_mode: SortMode,
    pub sort_rev:  bool,

    /// Marked entries (bit-field would be ideal but we use a simple bool array).
    pub marked: [bool; MAX_ENTRIES],

    /// Whether this panel currently has keyboard focus.
    pub focused: bool,
}

impl Panel {
    pub fn new(path: &str, focused: bool) -> Self {
        let mut p = Self {
            path_buf: [0u8; MAX_PATH],
            path_len: 0,
            entries: [DirEntry::EMPTY; MAX_ENTRIES],
            entry_count: 0,
            cursor: 0,
            scroll: 0,
            sort_mode: SortMode::Name,
            sort_rev: false,
            marked: [false; MAX_ENTRIES],
            focused,
        };
        p.set_path(path);
        p.reload();
        p
    }

    // ── Path ─────────────────────────────────────────────────────────────────

    fn set_path(&mut self, path: &str) {
        let b = path.as_bytes();
        let n = b.len().min(MAX_PATH);
        self.path_buf[..n].copy_from_slice(&b[..n]);
        self.path_len = n;
    }

    pub fn path(&self) -> &str {
        core::str::from_utf8(&self.path_buf[..self.path_len]).unwrap_or("/")
    }

    // ── Listing ──────────────────────────────────────────────────────────────

    /// Reload the directory listing from disk.
    pub fn reload(&mut self) {
        // Copy path to stack first to avoid simultaneous immutable + mutable borrow of self.
        let mut path_copy = [0u8; MAX_PATH];
        let plen = self.path_len;
        path_copy[..plen].copy_from_slice(&self.path_buf[..plen]);
        let path_str = core::str::from_utf8(&path_copy[..plen]).unwrap_or("/");
        self.entry_count = fs::read_dir(path_str, &mut self.entries);
        fs::sort_entries(&mut self.entries, self.entry_count, self.sort_mode, self.sort_rev);
        self.marked = [false; MAX_ENTRIES];
        self.clamp();
    }

    // ── Cursor movement ───────────────────────────────────────────────────────

    pub fn move_up(&mut self) {
        if self.cursor > 0 { self.cursor -= 1; }
        self.ensure_visible();
    }

    pub fn move_down(&mut self) {
        if self.entry_count > 0 && self.cursor + 1 < self.entry_count {
            self.cursor += 1;
        }
        self.ensure_visible();
    }

    pub fn page_up(&mut self, page_size: usize) {
        self.cursor = self.cursor.saturating_sub(page_size);
        self.ensure_visible();
    }

    pub fn page_down(&mut self, page_size: usize) {
        if self.entry_count > 0 {
            self.cursor = (self.cursor + page_size).min(self.entry_count - 1);
        }
        self.ensure_visible();
    }

    pub fn move_home(&mut self) {
        self.cursor = 0;
        self.scroll = 0;
    }

    pub fn move_end(&mut self) {
        if self.entry_count > 0 { self.cursor = self.entry_count - 1; }
        self.ensure_visible();
    }

    // ── Entry access ──────────────────────────────────────────────────────────

    pub fn current(&self) -> Option<&DirEntry> {
        if self.entry_count > 0 && self.cursor < self.entry_count {
            Some(&self.entries[self.cursor])
        } else {
            None
        }
    }

    /// Full path to the currently selected entry.
    pub fn current_path(&self) -> ([u8; MAX_PATH], usize) {
        match self.current() {
            None => {
                let mut buf = [0u8; MAX_PATH];
                let b = self.path().as_bytes();
                let n = b.len().min(MAX_PATH);
                buf[..n].copy_from_slice(&b[..n]);
                (buf, n)
            }
            Some(e) => path_join(self.path(), e.name_str()),
        }
    }

    // ── Navigation ────────────────────────────────────────────────────────────

    /// Enter the directory under the cursor (or follow symlink).
    /// Returns false if cursor is not on a directory.
    pub fn enter_dir(&mut self) -> bool {
        let entry = match self.current() {
            Some(e) => e.clone(),
            None    => return false,
        };
        if !entry.is_dir() { return false; }

        if entry.is_dotdot() {
            self.go_to_parent();
        } else {
            let (buf, len) = path_join(self.path(), entry.name_str());
            let new_path = core::str::from_utf8(&buf[..len]).unwrap_or("/");
            self.set_path(new_path);
            self.cursor = 0;
            self.scroll = 0;
            self.reload();
        }
        true
    }

    /// Navigate to the parent directory, selecting the directory we came from.
    pub fn go_to_parent(&mut self) {
        if self.path() == "/" { return; }

        // Remember the last component of the current path so we can select it.
        let current_path = self.path();
        let child_name = {
            let trimmed = current_path.trim_end_matches('/');
            match trimmed.rfind('/') {
                Some(i) => &trimmed[i + 1..],
                None    => trimmed,
            }
        };
        // Copy child name to stack before we mutate self.
        let mut child_buf = [0u8; 56];
        let child_len = child_name.len().min(56);
        child_buf[..child_len].copy_from_slice(&child_name.as_bytes()[..child_len]);

        let pp = path_parent(current_path);
        let pb = pp.as_bytes();
        let mut arr = [0u8; MAX_PATH];
        let n = pb.len().min(MAX_PATH);
        arr[..n].copy_from_slice(&pb[..n]);
        let parent_s = core::str::from_utf8(&arr[..n]).unwrap_or("/");
        self.set_path(parent_s);
        self.cursor = 0;
        self.scroll = 0;
        self.reload();

        // Find and select the child directory we came from.
        if child_len > 0 {
            for i in 0..self.entry_count {
                if self.entries[i].name_len == child_len
                    && self.entries[i].name[..child_len] == child_buf[..child_len]
                {
                    self.cursor = i;
                    self.ensure_visible();
                    break;
                }
            }
        }
    }

    /// Navigate directly to a given path.
    pub fn go_to(&mut self, path: &str) {
        self.set_path(path);
        self.cursor = 0;
        self.scroll = 0;
        self.reload();
    }

    // ── Marking ───────────────────────────────────────────────────────────────

    /// Toggle mark on the current entry and advance cursor.
    pub fn toggle_mark_and_advance(&mut self) {
        if self.entry_count == 0 { return; }
        if self.cursor < self.entry_count {
            // Don't mark ".." or "."
            if !self.entries[self.cursor].is_dotdot() && !self.entries[self.cursor].is_dot() {
                self.marked[self.cursor] = !self.marked[self.cursor];
            }
        }
        self.move_down();
    }

    /// Return the number of marked entries.
    pub fn marked_count(&self) -> usize {
        self.marked[..self.entry_count].iter().filter(|&&m| m).count()
    }

    /// Collect paths of all marked entries (or current if none marked).
    /// Returns a fixed-size array of (buf, len) pairs and the count.
    pub fn selected_paths(&self) -> ([([ u8; MAX_PATH], usize); 64], usize) {
        let mut result = [([0u8; MAX_PATH], 0usize); 64];
        let mut n = 0usize;
        let mc = self.marked_count();
        if mc == 0 {
            if let Some(_) = self.current() {
                let (buf, len) = self.current_path();
                result[0] = (buf, len);
                n = 1;
            }
        } else {
            for i in 0..self.entry_count {
                if self.marked[i] && n < 64 {
                    let p = path_join(self.path(), self.entries[i].name_str());
                    result[n] = p;
                    n += 1;
                }
            }
        }
        (result, n)
    }

    // ── Sort ─────────────────────────────────────────────────────────────────

    pub fn set_sort(&mut self, mode: SortMode, rev: bool) {
        self.sort_mode = mode;
        self.sort_rev  = rev;
        fs::sort_entries(&mut self.entries, self.entry_count, self.sort_mode, self.sort_rev);
        self.clamp();
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    fn clamp(&mut self) {
        if self.entry_count == 0 {
            self.cursor = 0;
            self.scroll = 0;
        } else {
            if self.cursor >= self.entry_count { self.cursor = self.entry_count - 1; }
        }
        self.ensure_visible();
    }

    /// Adjust scroll so that cursor is within the visible window.
    pub fn ensure_visible_with(&mut self, visible_rows: usize) {
        if visible_rows == 0 { return; }
        if self.cursor < self.scroll { self.scroll = self.cursor; }
        if self.cursor >= self.scroll + visible_rows {
            self.scroll = self.cursor + 1 - visible_rows;
        }
    }

    fn ensure_visible(&mut self) {
        // Without knowing visible_rows here, just ensure scroll <= cursor.
        if self.cursor < self.scroll { self.scroll = self.cursor; }
    }
}

