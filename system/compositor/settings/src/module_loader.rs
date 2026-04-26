// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! Built-in page registration and user identity resolution.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::i18n;

use crate::types::*;

// ── Built-in page definitions ───────────────────────────────────────────────

pub fn builtin_pages() -> Vec<PageEntry> {
    let mut m = Vec::new();
    let b = |id: BuiltinId, name: &str, icon: &str, cat: &str, order: i64| PageEntry {
        id,
        name: String::from(name),
        icon: String::from(icon),
        category: String::from(cat),
        order,
        panel_id: 0,
    };
    m.push(b(BuiltinId::Dashboard, i18n::t("Dashboard"), "home.ico", i18n::t("System"), 0));
    m.push(b(BuiltinId::General, i18n::t("General"), "dev_cpu.ico", i18n::t("System"), 1));
    m.push(b(BuiltinId::Profile, i18n::t("Profile"), "dev_cpu.ico", i18n::t("System"), 1));
    m.push(b(BuiltinId::Display, i18n::t("Display"), "display.ico", i18n::t("System"), 2));
    m.push(b(BuiltinId::Dock, i18n::t("Dock"), "dock.ico", i18n::t("System"), 3));
    m.push(b(BuiltinId::Sound, i18n::t("Sound"), "dev_disk.ico", i18n::t("System"), 4));
    m.push(b(BuiltinId::Apps, i18n::t("Apps"), "dev_disk.ico", i18n::t("System"), 5));
    m.push(b(BuiltinId::Devices, i18n::t("Devices"), "devices.ico", i18n::t("System"), 5));
    m.push(b(BuiltinId::Keyboard, i18n::t("Keyboard Shortcuts"), "dev_cpu.ico", i18n::t("System"), 6));
    m.push(b(BuiltinId::Network, i18n::t("Network"), "dev_network.ico", i18n::t("System"), 7));
    m.push(b(BuiltinId::Update, i18n::t("Update"), "update.ico", i18n::t("System"), 8));
    m
}

// ── User identity ───────────────────────────────────────────────────────────

/// Resolve the current user's identity (uid, username, home directory).
pub fn resolve_user() -> (u16, String, String) {
    let uid = anyos_std::process::getuid();

    let mut name_buf = [0u8; 64];
    let nlen = anyos_std::process::getusername(uid, &mut name_buf);
    let username = if nlen != u32::MAX && nlen > 0 {
        core::str::from_utf8(&name_buf[..nlen as usize])
            .unwrap_or("root")
            .into()
    } else {
        String::from("root")
    };

    let mut home_buf = [0u8; 256];
    let hlen = anyos_std::env::get("HOME", &mut home_buf);
    let home = if hlen != u32::MAX && hlen > 0 {
        core::str::from_utf8(&home_buf[..hlen as usize])
            .unwrap_or("/tmp")
            .into()
    } else {
        format!("/Users/{}", username)
    };

    (uid, username, home)
}
