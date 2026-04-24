pub use anyos_std::path::{basename, extension, join, parent};

/// Check if a file or directory exists.
pub fn exists(path: &str) -> bool {
    let mut stat = [0u32; 7];
    anyos_std::fs::stat(path, &mut stat) != u32::MAX
}

/// Check if a path is a directory.
pub fn is_directory(path: &str) -> bool {
    let mut stat = [0u32; 7];
    if anyos_std::fs::stat(path, &mut stat) == u32::MAX {
        return false;
    }
    stat[0] == 1
}
