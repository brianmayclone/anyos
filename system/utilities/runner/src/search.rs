// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! Search and category grouping — combines app list with searchd results.

use crate::apps::AppEntry;
use crate::searchd;
use anyos_std::{String, Vec};

pub enum SearchResult {
    App {
        app_idx: usize,
    },
    File {
        name: String,
        path: String,
        kind: String,
        size: u32,
    },
}

impl SearchResult {
    pub fn display_name<'a>(&'a self, apps: &'a [AppEntry]) -> &'a str {
        match self {
            SearchResult::App { app_idx } => &apps[*app_idx].name,
            SearchResult::File { name, .. } => name,
        }
    }

    pub fn category(&self) -> Category {
        match self {
            SearchResult::App { .. } => Category::Apps,
            SearchResult::File { kind, .. } => Category::from_kind(kind),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Apps,
    Documents,
    Images,
    Folders,
    Other,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Category::Apps => "Applications",
            Category::Documents => "Documents",
            Category::Images => "Images",
            Category::Folders => "Folders",
            Category::Other => "Other",
        }
    }

    fn from_kind(kind: &str) -> Category {
        match kind {
            "document" | "text" | "script" | "config" => Category::Documents,
            "image" => Category::Images,
            "directory" => Category::Folders,
            _ => Category::Other,
        }
    }

    fn order(self) -> u8 {
        match self {
            Category::Apps => 0,
            Category::Documents => 1,
            Category::Images => 2,
            Category::Folders => 3,
            Category::Other => 4,
        }
    }
}

const MAX_APPS: usize = 5;
const MAX_PER_FILE_CAT: usize = 4;

/// Fast local-only filter (never blocks).
pub fn filter_apps(apps: &[AppEntry], query: &str) -> Vec<SearchResult> {
    let qb = query.as_bytes();
    let mut results = Vec::new();
    let mut count = 0;
    for (i, app) in apps.iter().enumerate() {
        if count >= MAX_APPS {
            break;
        }
        if query.is_empty() || contains_ci(app.name.as_bytes(), qb) {
            results.push(SearchResult::App { app_idx: i });
            count += 1;
        }
    }
    results
}

/// Full filter: apps + blocking searchd query.
pub fn filter_all(apps: &[AppEntry], query: &str) -> Vec<SearchResult> {
    let mut results = filter_apps(apps, query);

    if query.len() >= 2 {
        let file_results = searchd::search(query);
        let mut doc_count = 0usize;
        let mut img_count = 0usize;
        let mut dir_count = 0usize;
        let mut other_count = 0usize;

        for fr in file_results {
            let cat = Category::from_kind(&fr.kind);
            let count = match cat {
                Category::Documents => &mut doc_count,
                Category::Images => &mut img_count,
                Category::Folders => &mut dir_count,
                _ => &mut other_count,
            };
            if *count >= MAX_PER_FILE_CAT {
                continue;
            }
            *count += 1;
            results.push(SearchResult::File {
                name: fr.name,
                path: fr.path,
                kind: fr.kind,
                size: fr.size,
            });
        }
    }

    results.sort_by(|a, b| a.category().order().cmp(&b.category().order()));
    results
}

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
