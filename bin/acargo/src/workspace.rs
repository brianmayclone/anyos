//! Workspace discovery and management.
//!
//! Handles Cargo workspace resolution: finding workspace root, discovering
//! member crates, and resolving workspace-level dependencies.

use crate::prelude::*;
use crate::fs;
use crate::toml;
use crate::manifest;
use anyos_std::println;

/// A resolved workspace with all member paths.
#[derive(Debug, Clone)]
pub struct Workspace {
    /// Root directory of the workspace.
    pub root_dir: String,
    /// Manifest of the workspace root.
    pub root_manifest: manifest::Manifest,
    /// Resolved member directories (absolute paths).
    pub members: Vec<String>,
}

/// Discover a workspace starting from the given directory.
/// Walks upward to find a workspace root (Cargo.toml with [workspace] section).
/// If no workspace root is found, returns None.
pub fn discover(start_dir: &str) -> Option<Workspace> {
    // First check the start directory itself
    if let Some(ws) = try_workspace_root(start_dir) {
        return Some(ws);
    }

    // Walk up parent directories
    let mut dir = String::from(start_dir);
    loop {
        if let Some(pos) = dir.rfind('/') {
            if pos == 0 {
                break; // reached filesystem root
            }
            dir = dir[..pos].to_string();
            if let Some(ws) = try_workspace_root(&dir) {
                return Some(ws);
            }
        } else {
            break;
        }
    }

    None
}

fn try_workspace_root(dir: &str) -> Option<Workspace> {
    let cargo_path = format!("{}/Cargo.toml", dir);
    let toml_src = fs::read_file(&cargo_path)?;
    let table = toml::parse(&toml_src);
    let mf = manifest::parse(&table);

    if mf.workspace_members.is_empty() {
        return None;
    }

    let mut members = Vec::new();
    for pattern in &mf.workspace_members {
        // Simple glob: "crates/*", "libs/*", or specific paths like "kernel"
        if pattern.ends_with("/*") {
            let base = &pattern[..pattern.len() - 2];
            let base_dir = format!("{}/{}", dir, base);
            discover_members_in_dir(&base_dir, &mut members);
        } else {
            let member_dir = fs::resolve_path(dir, pattern);
            let member_cargo = format!("{}/Cargo.toml", member_dir);
            if fs::file_exists(&member_cargo) {
                members.push(member_dir);
            }
        }
    }

    // Remove excluded members
    for exclude in &mf.workspace_excludes {
        let excluded_dir = fs::resolve_path(dir, exclude);
        members.retain(|m| m != &excluded_dir);
    }

    Some(Workspace {
        root_dir: dir.to_string(),
        root_manifest: mf,
        members,
    })
}

fn discover_members_in_dir(dir: &str, members: &mut Vec<String>) {
    let mut buf = [0u8; 64 * 128];
    let count = anyos_std::fs::readdir(dir, &mut buf);
    if count == u32::MAX {
        return;
    }

    let entry_size = 64;
    for i in 0..count as usize {
        let off = i * entry_size;
        let file_type = buf[off];
        let name_len = buf[off + 1] as usize;
        if name_len == 0 { continue; }
        let name_bytes = &buf[off + 8..off + 8 + name_len];
        if let Ok(name) = core::str::from_utf8(name_bytes) {
            if name == "." || name == ".." { continue; }
            if file_type == 1 { // directory
                let child_dir = format!("{}/{}", dir, name);
                let cargo_path = format!("{}/Cargo.toml", child_dir);
                if fs::file_exists(&cargo_path) {
                    members.push(child_dir);
                }
            }
        }
    }
}

/// Resolve features for a crate based on requested features and manifest defaults.
pub fn resolve_features(
    manifest: &manifest::Manifest,
    requested: &[String],
) -> Vec<String> {
    let mut active = Vec::new();

    let use_defaults = !requested.iter().any(|f| f == "__no_default__");
    let all_features = requested.iter().any(|f| f == "__all__");

    // Add default features
    if use_defaults {
        for (feat_name, _) in &manifest.features {
            if feat_name == "default" {
                // Expand default feature dependencies
                for (_, deps) in &manifest.features {
                    if feat_name == "default" {
                        for dep in deps {
                            if !active.contains(dep) {
                                active.push(dep.clone());
                            }
                        }
                    }
                }
            }
        }
    }

    if all_features {
        // Enable all features
        for (feat_name, _) in &manifest.features {
            if feat_name != "default" && !active.contains(feat_name) {
                active.push(feat_name.clone());
            }
        }
    } else {
        // Add explicitly requested features
        for feat in requested {
            if feat.starts_with("__") { continue; } // skip internal markers
            if !active.contains(feat) {
                active.push(feat.clone());
            }
            // Expand transitive feature dependencies
            for (fname, fdeps) in &manifest.features {
                if fname == feat {
                    for dep in fdeps {
                        if !active.contains(dep) {
                            active.push(dep.clone());
                        }
                    }
                }
            }
        }
    }

    active
}
