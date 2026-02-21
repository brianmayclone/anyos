use alloc::string::String;
use alloc::format;

/// Extract the last component of a path (filename or directory name).
pub fn basename(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(i) => &trimmed[i + 1..],
        None => trimmed,
    }
}

/// Extract the file extension from a path (without dot).
pub fn extension(path: &str) -> Option<&str> {
    let name = basename(path);
    let dot = name.rfind('.')?;
    if dot == 0 {
        return None; // hidden file like ".gitignore"
    }
    Some(&name[dot + 1..])
}

/// Extract the parent directory of a path.
pub fn parent(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(i) => &trimmed[..i],
        None => ".",
    }
}

/// Join a directory and a filename.
pub fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{}{}", dir, name)
    } else {
        format!("{}/{}", dir, name)
    }
}

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
