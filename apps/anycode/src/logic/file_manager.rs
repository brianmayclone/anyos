use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;
use crate::util::path;

/// Data model for an open file (no UI dependencies).
pub struct OpenFileData {
    pub path: String,
    pub modified: bool,
    pub is_untitled: bool,
}

/// Manages the list of open files and the active tab index.
pub struct FileManager {
    pub files: Vec<OpenFileData>,
    pub active: usize,
    untitled_counter: u32,
}

impl FileManager {
    pub fn new() -> Self {
        Self {
            files: Vec::new(),
            active: 0,
            untitled_counter: 0,
        }
    }

    /// Check if a file is already open. Returns its index if found.
    pub fn find_open(&self, file_path: &str) -> Option<usize> {
        self.files.iter().position(|f| f.path == file_path)
    }

    /// Add a new file to the open list. Returns the new index.
    pub fn add_file(&mut self, file_path: &str) -> usize {
        self.files.push(OpenFileData {
            path: String::from(file_path),
            modified: false,
            is_untitled: false,
        });
        self.files.len() - 1
    }

    /// Add a new untitled file. Returns the (index, path).
    pub fn add_untitled(&mut self) -> (usize, String) {
        self.untitled_counter += 1;
        let p = format!("/tmp/untitled-{}.txt", self.untitled_counter);
        let idx = self.files.len();
        self.files.push(OpenFileData {
            path: p.clone(),
            modified: false,
            is_untitled: true,
        });
        (idx, p)
    }

    /// Set the active tab index.
    pub fn set_active(&mut self, index: usize) {
        if index < self.files.len() {
            self.active = index;
        }
    }

    /// Remove a file at the given index. Returns the new active index.
    pub fn remove(&mut self, index: usize) -> usize {
        if index < self.files.len() {
            self.files.remove(index);
        }
        if self.files.is_empty() {
            self.active = 0;
        } else if self.active >= self.files.len() {
            self.active = self.files.len() - 1;
        }
        self.active
    }

    /// Mark a file as modified.
    pub fn mark_modified(&mut self, index: usize) {
        if let Some(f) = self.files.get_mut(index) {
            f.modified = true;
        }
    }

    /// Mark a file as saved (not modified).
    pub fn mark_saved(&mut self, index: usize) {
        if let Some(f) = self.files.get_mut(index) {
            f.modified = false;
        }
    }

    /// Get the active file data, if any.
    pub fn active_file(&self) -> Option<&OpenFileData> {
        self.files.get(self.active)
    }

    /// Build the tab label string ("file1.c|file2.rs *|...").
    pub fn tab_labels(&self) -> String {
        let mut labels = String::new();
        for (i, f) in self.files.iter().enumerate() {
            if i > 0 {
                labels.push('|');
            }
            labels.push_str(path::basename(&f.path));
            if f.modified {
                labels.push_str(" *");
            }
        }
        labels
    }

    /// Get the count of open files.
    pub fn count(&self) -> usize {
        self.files.len()
    }

    /// Check if there are any modified files.
    pub fn has_modified(&self) -> bool {
        self.files.iter().any(|f| f.modified)
    }
}

/// Read a file's contents into a Vec<u8>.
pub fn read_file(file_path: &str) -> Option<Vec<u8>> {
    anyos_std::fs::read_to_vec(file_path).ok()
}

/// Write data to a file.
pub fn write_file(file_path: &str, data: &[u8]) -> bool {
    anyos_std::fs::write_bytes(file_path, data).is_ok()
}
