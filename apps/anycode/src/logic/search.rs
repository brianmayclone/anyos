use alloc::string::String;
use alloc::vec::Vec;
use crate::util::path;

/// A single search result.
pub struct SearchResult {
    pub file_path: String,
    pub line_number: u32,
    pub line_text: String,
}

/// Search for a query string in all files under root.
/// Returns up to `max_results` matches.
pub fn search_in_project(root: &str, query: &str, max_results: u32) -> Vec<SearchResult> {
    let mut results = Vec::new();
    if query.is_empty() {
        return results;
    }
    search_dir(root, query, max_results, &mut results);
    results
}

fn search_dir(dir: &str, query: &str, max_results: u32, results: &mut Vec<SearchResult>) {
    if results.len() >= max_results as usize {
        return;
    }

    let entries = match anyos_std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return,
    };

    for entry in entries {
        if entry.name == "." || entry.name == ".." {
            continue;
        }
        if results.len() >= max_results as usize {
            return;
        }

        let full_path = path::join(dir, &entry.name);

        if entry.is_dir() {
            search_dir(&full_path, query, max_results, results);
        } else if entry.is_file() {
            if entry.size > 512 * 1024 {
                continue;
            }
            search_file(&full_path, query, max_results, results);
        }
    }
}

fn search_file(file_path: &str, query: &str, max_results: u32, results: &mut Vec<SearchResult>) {
    let data = match anyos_std::fs::read_to_vec(file_path) {
        Ok(d) => d,
        Err(_) => return,
    };

    // Skip binary files
    let check_len = data.len().min(512);
    if data[..check_len].contains(&0) {
        return;
    }

    let text = match core::str::from_utf8(&data) {
        Ok(s) => s,
        Err(_) => return,
    };

    let query_lower = ascii_lowercase(query);
    for (line_no, line) in text.split('\n').enumerate() {
        if results.len() >= max_results as usize {
            return;
        }
        let line_lower = ascii_lowercase(line);
        if line_lower.contains(&query_lower) {
            let trimmed = if line.len() > 120 { &line[..120] } else { line };
            results.push(SearchResult {
                file_path: String::from(file_path),
                line_number: line_no as u32 + 1,
                line_text: String::from(trimmed),
            });
        }
    }
}

fn ascii_lowercase(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c >= 'A' && c <= 'Z' {
            out.push((c as u8 + 32) as char);
        } else {
            out.push(c);
        }
    }
    out
}
