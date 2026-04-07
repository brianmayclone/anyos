// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! Search and category grouping.

use anyos_std::Vec;
use crate::apps::AppEntry;

/// A search result — index into the app list + category tag.
pub struct SearchResult {
    /// Index into `RunnerState.apps`.
    pub app_idx: usize,
    pub category: Category,
}

/// Result categories (ordered by display priority).
#[derive(Clone, Copy, PartialEq)]
pub enum Category {
    Apps,
    // Future categories from searchd:
    // Documents,
    // Files,
    // Settings,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Category::Apps => "Applications",
        }
    }
}

/// Maximum results per category.
const MAX_PER_CATEGORY: usize = 5;

/// Filter apps by query and return grouped results.
pub fn filter(apps: &[AppEntry], query: &str) -> Vec<SearchResult> {
    let qb = query.as_bytes();
    let mut results = Vec::new();
    let mut count = 0usize;

    for (i, app) in apps.iter().enumerate() {
        if count >= MAX_PER_CATEGORY {
            break;
        }
        if query.is_empty() || contains_ci(app.name.as_bytes(), qb) {
            results.push(SearchResult { app_idx: i, category: Category::Apps });
            count += 1;
        }
    }

    // Future: add results from searchd here

    results
}

/// Case-insensitive substring search.
fn contains_ci(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    if needle.len() > haystack.len() {
        return false;
    }
    for i in 0..=(haystack.len() - needle.len()) {
        let mut ok = true;
        for j in 0..needle.len() {
            if haystack[i + j].to_ascii_lowercase() != needle[j].to_ascii_lowercase() {
                ok = false;
                break;
            }
        }
        if ok {
            return true;
        }
    }
    false
}
