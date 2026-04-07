//! File crawler and content indexer.
//!
//! Recursively walks configured directories, classifies each entry by
//! extension and content heuristics, and stores metadata + text content
//! in the search database.

use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;
use libdb_client::Database;
use crate::config::Config;
use crate::schema;

// ── Directories to index ────────────────────────────────────────────────────

/// Standard directories to crawl.
const INDEX_DIRS: &[&str] = &[
    "/Users",
    "/Applications",
    "/System/bin",
    "/System/etc",
    "/System/lib",
    "/System/fonts",
    "/System/share",
    "/Documents",
    "/Desktop",
    "/Downloads",
    "/tmp",
];

/// Maximum recursion depth to prevent infinite loops.
const MAX_DEPTH: u32 = 16;

/// Maximum file size to read content from (64 KiB).
const MAX_CONTENT_SIZE: u32 = 65536;

/// Chunk size for content storage (libdb TEXT limit is 255 bytes, use 240
/// to leave room for escaping).
const CHUNK_SIZE: usize = 240;

/// Tracks entry count during indexing to enforce maxEntries.
static mut ENTRY_COUNT: u32 = 0;
static mut MAX_ENTRIES: u32 = 1000000;

// ── Public API ──────────────────────────────────────────────────────────────

/// Run a full index pass over all configured directories.
pub fn index_all(db: &Database, cfg: &Config) {
    schema::clear_index(db);
    unsafe {
        ENTRY_COUNT = 0;
        MAX_ENTRIES = cfg.max_entries;
    }
    for dir in INDEX_DIRS {
        if is_excluded(dir, cfg) {
            continue;
        }
        index_directory(db, dir, 0, cfg);
    }
    let count = unsafe { ENTRY_COUNT };
    anyos_std::println!("searchd: indexed {} entries", count);
}

/// Run an incremental index pass — only re-scan directories whose
/// last_scan is older than `max_age_ms` milliseconds.
pub fn index_incremental(db: &Database, max_age_ms: u32, cfg: &Config) {
    let now = anyos_std::sys::uptime_ms();
    unsafe {
        ENTRY_COUNT = 0;
        MAX_ENTRIES = cfg.max_entries;
    }
    for dir in INDEX_DIRS {
        if is_excluded(dir, cfg) {
            continue;
        }
        if should_rescan(db, dir, now, max_age_ms) {
            delete_subtree(db, dir);
            index_directory(db, dir, 0, cfg);
        }
    }
}

/// Check if a path is in the exclude list.
fn is_excluded(path: &str, cfg: &Config) -> bool {
    for excl in &cfg.excludes {
        if path == excl.as_str() || path.starts_with(excl.as_str()) {
            return true;
        }
    }
    false
}

// ── Crawler ─────────────────────────────────────────────────────────────────

fn index_directory(db: &Database, path: &str, depth: u32, cfg: &Config) {
    if depth > MAX_DEPTH {
        return;
    }
    if unsafe { ENTRY_COUNT } >= unsafe { MAX_ENTRIES } {
        return;
    }

    let entries = match anyos_std::fs::read_dir(path) {
        Ok(rd) => {
            let mut v = Vec::new();
            for e in rd {
                v.push(e);
            }
            v
        }
        Err(_) => return,
    };

    // Index the directory itself
    insert_file(db, path, dir_name(path), schema::KIND_DIR, 0, path);

    // Update scan timestamp
    update_scan_time(db, path);

    for entry in &entries {
        if entry.name == "." || entry.name == ".." {
            continue;
        }
        if unsafe { ENTRY_COUNT } >= unsafe { MAX_ENTRIES } {
            return;
        }

        let full_path = if path.ends_with('/') {
            format!("{}{}", path, entry.name)
        } else {
            format!("{}/{}", path, entry.name)
        };

        if is_excluded(&full_path, cfg) {
            continue;
        }

        if entry.is_dir() {
            insert_file(db, &full_path, &entry.name, schema::KIND_DIR, 0, path);
            index_directory(db, &full_path, depth + 1, cfg);
        } else {
            let kind = classify(&entry.name, entry.size);
            insert_file(db, &full_path, &entry.name, kind, entry.size, path);

            // Index content for text-like files
            if should_index_content(kind) && entry.size > 0 && entry.size <= MAX_CONTENT_SIZE {
                index_file_content(db, &full_path, entry.size);
            }
        }
    }
}

// ── Classification ──────────────────────────────────────────────────────────

/// Classify a file by its extension and name.
fn classify(name: &str, _size: u32) -> &'static str {
    let lower_name = to_lower(name);
    let ext = extension(&lower_name);

    // URL / bookmark files
    if ext == "url" || ext == "webloc" || ext == "desktop" {
        return schema::KIND_URL;
    }

    // Symlinks (detected by fs, but we can also check .lnk)
    if ext == "lnk" {
        return schema::KIND_LINK;
    }

    // Text documents
    if matches!(ext.as_str(),
        "txt" | "md" | "markdown" | "rst" | "org" | "adoc" | "tex" | "rtf"
        | "csv" | "tsv" | "log" | "nfo" | "readme"
    ) {
        return schema::KIND_DOCUMENT;
    }

    // Source code / scripts
    if matches!(ext.as_str(),
        "rs" | "c" | "h" | "cpp" | "cxx" | "hpp" | "cc"
        | "js" | "ts" | "jsx" | "tsx" | "mjs"
        | "py" | "rb" | "pl" | "lua" | "go" | "java" | "kt" | "swift"
        | "cs" | "vb" | "fs" | "hs" | "ml" | "ex" | "exs" | "erl"
        | "zig" | "nim" | "d" | "v" | "asm" | "s" | "S"
        | "html" | "htm" | "xhtml" | "xml" | "xsl" | "xslt"
        | "css" | "scss" | "sass" | "less"
        | "sql" | "graphql" | "gql"
        | "sh" | "bash" | "zsh" | "fish" | "bat" | "cmd" | "ps1"
        | "php" | "r" | "m" | "mm"
    ) {
        return schema::KIND_SCRIPT;
    }

    // Config files
    if matches!(ext.as_str(),
        "conf" | "cfg" | "ini" | "toml" | "yaml" | "yml" | "json" | "jsonc"
        | "env" | "properties" | "plist" | "reg"
    ) {
        return schema::KIND_CONFIG;
    }

    // Images
    if matches!(ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "ico" | "svg" | "webp"
        | "tiff" | "tif" | "psd" | "xcf" | "raw" | "cr2" | "nef"
    ) {
        return schema::KIND_IMAGE;
    }

    // Audio
    if matches!(ext.as_str(),
        "mp3" | "wav" | "flac" | "ogg" | "aac" | "wma" | "m4a" | "opus" | "mid" | "midi"
    ) {
        return schema::KIND_AUDIO;
    }

    // Video
    if matches!(ext.as_str(),
        "mp4" | "avi" | "mkv" | "mov" | "wmv" | "flv" | "webm" | "m4v" | "mpg" | "mpeg"
    ) {
        return schema::KIND_VIDEO;
    }

    // Archives
    if matches!(ext.as_str(),
        "zip" | "tar" | "gz" | "bz2" | "xz" | "7z" | "rar" | "cab" | "iso" | "img" | "dmg"
    ) {
        return schema::KIND_ARCHIVE;
    }

    // Fonts
    if matches!(ext.as_str(), "ttf" | "otf" | "woff" | "woff2" | "fon" | "bdf") {
        return schema::KIND_FONT;
    }

    // Libraries / shared objects
    if ext == "so" || ext == "dll" || ext == "dylib" || ext == "a" || ext == "lib" {
        return schema::KIND_LIBRARY;
    }

    // Database files
    if ext == "db" || ext == "sqlite" || ext == "sqlite3" {
        return schema::KIND_DATABASE;
    }

    // Executables (no extension or known binary extensions)
    if ext == "elf" || ext == "exe" || ext == "app" || ext == "bin" || ext == "com" {
        return schema::KIND_EXECUTABLE;
    }

    // Well-known filenames without extension
    let base = basename(&lower_name);
    if matches!(base.as_str(),
        "makefile" | "cmakelists.txt" | "dockerfile" | "vagrantfile"
        | "rakefile" | "gemfile" | "procfile" | "justfile"
        | "license" | "licence" | "readme" | "changelog" | "authors"
        | "todo" | "notes" | "history"
    ) {
        return schema::KIND_DOCUMENT;
    }

    // Files without extension in /System/bin are likely executables
    if ext.is_empty() {
        return schema::KIND_EXECUTABLE;
    }

    schema::KIND_BINARY
}

/// Determine if a file kind should have its content indexed.
fn should_index_content(kind: &str) -> bool {
    matches!(kind,
        schema::KIND_TEXT
        | schema::KIND_DOCUMENT
        | schema::KIND_SCRIPT
        | schema::KIND_CONFIG
        | schema::KIND_URL
    )
}

// ── Content indexing ────────────────────────────────────────────────────────

/// Read file content and store searchable text chunks in the database.
fn index_file_content(db: &Database, path: &str, size: u32) {
    let fd = anyos_std::fs::open(path, 0);
    if fd == u32::MAX {
        return;
    }

    let read_size = if size > MAX_CONTENT_SIZE { MAX_CONTENT_SIZE } else { size };
    let mut buf = alloc::vec![0u8; read_size as usize];
    let n = anyos_std::fs::read(fd, &mut buf);
    anyos_std::fs::close(fd);

    if n == 0 || n == u32::MAX {
        return;
    }

    let data = &buf[..n as usize];

    // Check if content looks like text (no null bytes in first 512 bytes)
    let check_len = if data.len() > 512 { 512 } else { data.len() };
    if data[..check_len].iter().any(|&b| b == 0) {
        return; // Likely binary, skip
    }

    // Convert to string, replacing invalid UTF-8
    let text = match core::str::from_utf8(data) {
        Ok(s) => s,
        Err(_) => return, // Not valid text
    };

    // Store in chunks (libdb TEXT max is 255 bytes)
    let escaped_path = sql_escape(path);
    let mut chunk_idx: u32 = 0;
    let mut pos = 0;
    let bytes = text.as_bytes();

    while pos < bytes.len() {
        let end = if pos + CHUNK_SIZE > bytes.len() {
            bytes.len()
        } else {
            // Try to break at a word boundary
            let mut brk = pos + CHUNK_SIZE;
            while brk > pos && bytes[brk - 1] != b' ' && bytes[brk - 1] != b'\n' {
                brk -= 1;
            }
            if brk == pos { pos + CHUNK_SIZE } else { brk }
        };

        let chunk = match core::str::from_utf8(&bytes[pos..end]) {
            Ok(s) => s,
            Err(_) => {
                pos = end;
                continue;
            }
        };

        // Normalize whitespace for better search
        let normalized = normalize_text(chunk);
        if !normalized.is_empty() {
            let escaped_body = sql_escape(&normalized);
            let sql = format!(
                "INSERT INTO content (path, chunk, body) VALUES ('{}', {}, '{}')",
                escaped_path, chunk_idx, escaped_body
            );
            let _ = db.exec(&sql);
            chunk_idx += 1;
        }

        pos = end;
    }
}

// ── Database helpers ────────────────────────────────────────────────────────

fn insert_file(db: &Database, path: &str, name: &str, kind: &str, size: u32, parent: &str) {
    let now = anyos_std::sys::uptime_ms();
    let sql = format!(
        "INSERT INTO files (path, name, kind, size, modified, parent) VALUES ('{}', '{}', '{}', {}, {}, '{}')",
        sql_escape(path),
        sql_escape(name),
        kind,
        size,
        now,
        sql_escape(parent),
    );
    match db.exec(&sql) {
        Ok(_) => {
            unsafe { ENTRY_COUNT += 1; }
        }
        Err(e) => {
            // Log only the first few errors to avoid flooding
            let count = unsafe { ENTRY_COUNT };
            if count == 0 {
                anyos_std::println!("searchd: INSERT error for '{}': {}", path, e);
                anyos_std::println!("searchd: SQL len={}", sql.len());
            }
        }
    }
}

fn delete_subtree(db: &Database, dir: &str) {
    let escaped = sql_escape(dir);
    // Delete content for files in this subtree
    // libdb doesn't support LIKE or subqueries, so we query files first
    // then delete content by exact path match. For simplicity, we do a
    // broad delete from content where the path starts with dir.
    // Since libdb doesn't support LIKE, we delete all content and re-index.
    // This is acceptable because incremental indexing only triggers for
    // stale directories.
    let query_sql = format!("SELECT path FROM files");
    if let Ok(result) = db.query(&query_sql) {
        for row in 0..result.row_count() {
            if let Some(path) = result.get_text(row, 0) {
                if path.starts_with(dir) {
                    let ep = sql_escape(&path);
                    let _ = db.exec(&format!("DELETE FROM content WHERE path = '{}'", ep));
                    let _ = db.exec(&format!("DELETE FROM files WHERE path = '{}'", ep));
                }
            }
        }
    }
}

fn update_scan_time(db: &Database, dir: &str) {
    let now = anyos_std::sys::uptime_ms();
    let escaped = sql_escape(dir);
    // Try update first, then insert if no rows affected
    let updated = db.exec(&format!(
        "UPDATE state SET last_scan = {} WHERE dir = '{}'", now, escaped
    ));
    match updated {
        Ok(0) => {
            let _ = db.exec(&format!(
                "INSERT INTO state (dir, last_scan) VALUES ('{}', {})", escaped, now
            ));
        }
        _ => {}
    }
}

fn should_rescan(db: &Database, dir: &str, now: u32, max_age_ms: u32) -> bool {
    let escaped = sql_escape(dir);
    let sql = format!("SELECT last_scan FROM state WHERE dir = '{}'", escaped);
    match db.query(&sql) {
        Ok(result) => {
            if result.row_count() == 0 {
                return true; // Never scanned
            }
            match result.get_int(0, 0) {
                Some(last) => now.wrapping_sub(last as u32) >= max_age_ms,
                None => true,
            }
        }
        Err(_) => true,
    }
}

// ── String utilities ────────────────────────────────────────────────────────

/// Escape single quotes for SQL.
fn sql_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch == '\'' {
            out.push('\'');
            out.push('\'');
        } else {
            out.push(ch);
        }
    }
    out
}

/// Get the file extension (lowercase, without dot).
fn extension(name: &str) -> String {
    if let Some(dot) = name.rfind('.') {
        if dot + 1 < name.len() {
            return String::from(&name[dot + 1..]);
        }
    }
    String::new()
}

/// Get the basename (filename without directories).
fn basename(path: &str) -> String {
    if let Some(slash) = path.rfind('/') {
        String::from(&path[slash + 1..])
    } else {
        String::from(path)
    }
}

/// Get directory name from path.
fn dir_name(path: &str) -> &str {
    if let Some(slash) = path.rfind('/') {
        if slash + 1 < path.len() {
            return &path[slash + 1..];
        }
    }
    path
}

/// Convert ASCII to lowercase.
fn to_lower(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b >= b'A' && b <= b'Z' {
            out.push((b + 32) as char);
        } else {
            out.push(b as char);
        }
    }
    out
}

/// Normalize text: collapse whitespace, trim, lowercase.
fn normalize_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_space = true;
    for ch in s.chars() {
        if ch.is_ascii_whitespace() || ch == '\r' || ch == '\n' || ch == '\t' {
            if !last_space && out.len() < CHUNK_SIZE {
                out.push(' ');
                last_space = true;
            }
        } else {
            if out.len() >= CHUNK_SIZE {
                break;
            }
            // Lowercase ASCII
            if ch >= 'A' && ch <= 'Z' {
                out.push((ch as u8 + 32) as char);
            } else {
                out.push(ch);
            }
            last_space = false;
        }
    }
    // Trim trailing space
    if out.ends_with(' ') {
        out.pop();
    }
    out
}
