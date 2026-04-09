//! .gitignore pattern matching.
//!
//! Supports standard gitignore patterns:
//! - Simple glob patterns (*, ?)
//! - Directory patterns (ending with /)
//! - Negation patterns (starting with !)
//! - Comments (starting with #)
//! - Double-star wildcard (**)

use alloc::string::String;
use alloc::vec::Vec;

/// A compiled gitignore rule.
#[derive(Debug, Clone)]
struct IgnoreRule {
    pattern: String,
    is_negation: bool,
    is_dir_only: bool,
    is_anchored: bool,
}

/// Gitignore matcher.
#[derive(Debug)]
pub struct GitIgnore {
    rules: Vec<IgnoreRule>,
}

impl GitIgnore {
    /// Create an empty ignore matcher.
    pub fn new() -> Self {
        GitIgnore { rules: Vec::new() }
    }

    /// Parse a .gitignore file.
    pub fn parse(content: &str) -> Self {
        let mut rules = Vec::new();

        for line in content.lines() {
            let trimmed = line.trim();

            // Skip empty lines and comments
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            let mut pattern = String::from(trimmed);
            let is_negation = pattern.starts_with('!');
            if is_negation {
                pattern = String::from(&pattern[1..]);
            }

            // Remove trailing spaces (unless escaped)
            while pattern.ends_with(' ') && !pattern.ends_with("\\ ") {
                pattern.pop();
            }

            let is_dir_only = pattern.ends_with('/');
            if is_dir_only {
                pattern.pop();
            }

            let is_anchored = pattern.contains('/');

            rules.push(IgnoreRule {
                pattern,
                is_negation,
                is_dir_only,
                is_anchored,
            });
        }

        GitIgnore { rules }
    }

    /// Load .gitignore from a repository.
    pub fn load(repo: &crate::repo::Repository) -> Self {
        let path = repo.workdir_path(".gitignore");
        match std::fs::read_to_string(&path) {
            Ok(content) => Self::parse(&content),
            Err(_) => Self::new(),
        }
    }

    /// Check if a path should be ignored.
    /// `is_dir` indicates whether the path is a directory.
    pub fn is_ignored(&self, path: &str, is_dir: bool) -> bool {
        let mut ignored = false;

        for rule in &self.rules {
            if rule.is_dir_only && !is_dir {
                continue;
            }

            let matches = if rule.is_anchored {
                glob_match(&rule.pattern, path)
            } else {
                // Non-anchored: match against basename or full path
                let basename = path.rsplit('/').next().unwrap_or(path);
                glob_match(&rule.pattern, basename) || glob_match(&rule.pattern, path)
            };

            if matches {
                ignored = !rule.is_negation;
            }
        }

        ignored
    }

    /// Add default ignores (always ignored regardless of .gitignore).
    pub fn with_defaults(mut self) -> Self {
        self.rules.insert(0, IgnoreRule {
            pattern: String::from(".git"),
            is_negation: false,
            is_dir_only: true,
            is_anchored: false,
        });
        self
    }
}

/// Simple glob pattern matching.
/// Supports: * (any chars), ? (single char), ** (any path segments)
fn glob_match(pattern: &str, text: &str) -> bool {
    glob_match_impl(pattern.as_bytes(), text.as_bytes())
}

fn glob_match_impl(pattern: &[u8], text: &[u8]) -> bool {
    let mut pi = 0;
    let mut ti = 0;
    let mut star_pi = usize::MAX;
    let mut star_ti = 0;

    while ti < text.len() {
        if pi < pattern.len() {
            // Handle **
            if pi + 1 < pattern.len() && pattern[pi] == b'*' && pattern[pi + 1] == b'*' {
                // ** matches any number of path segments
                pi += 2;
                if pi < pattern.len() && pattern[pi] == b'/' {
                    pi += 1;
                }
                // Try matching rest against every suffix
                for k in ti..=text.len() {
                    if glob_match_impl(&pattern[pi..], &text[k..]) {
                        return true;
                    }
                }
                return false;
            }

            if pattern[pi] == b'*' {
                star_pi = pi;
                star_ti = ti;
                pi += 1;
                continue;
            }

            if pattern[pi] == b'?' || pattern[pi] == text[ti] {
                pi += 1;
                ti += 1;
                continue;
            }
        }

        if star_pi != usize::MAX {
            pi = star_pi + 1;
            star_ti += 1;
            ti = star_ti;
            continue;
        }

        return false;
    }

    // Skip trailing *
    while pi < pattern.len() && pattern[pi] == b'*' {
        pi += 1;
    }

    pi == pattern.len()
}
