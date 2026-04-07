// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! Host-mode i18n — identity translation (returns key as-is).

pub fn init() {}

pub fn t(key: &str) -> &str {
    key
}

pub fn lang() -> &'static str {
    "en"
}
