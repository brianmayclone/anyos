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
        for (feat_name, deps) in &manifest.features {
            if feat_name == "default" {
                for dep in deps {
                    activate_feature(manifest, &mut active, dep);
                }
            }
        }
    }

    if all_features {
        // Enable all features
        for (feat_name, _) in &manifest.features {
            if feat_name != "default" {
                activate_feature(manifest, &mut active, feat_name);
            }
        }
    } else {
        // Add explicitly requested features
        for feat in requested {
            if feat.starts_with("__") { continue; } // skip internal markers
            activate_feature(manifest, &mut active, feat);
        }
    }

    active
}

fn activate_feature(manifest: &manifest::Manifest, active: &mut Vec<String>, feature: &str) {
    if active.iter().any(|existing| existing == feature) {
        return;
    }
    active.push(feature.to_string());

    if feature.starts_with("dep:") || feature.contains('/') {
        return;
    }

    let deps = manifest
        .features
        .iter()
        .find_map(|(name, deps)| (name == feature).then(|| deps.clone()))
        .unwrap_or_default();
    for dep in deps {
        activate_feature(manifest, active, &dep);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{CrateKind, Manifest};

    fn manifest_with_features(features: Vec<(&str, Vec<&str>)>) -> Manifest {
        Manifest {
            name: String::from("demo"),
            version: String::from("0.1.0"),
            edition: String::from("2021"),
            dependencies: Vec::new(),
            crate_type: CrateKind::Lib,
            bin_name: None,
            bin_path: None,
            lib_name: None,
            opt_level_dev: 0,
            opt_level_release: 2,
            panic: String::from("abort"),
            workspace_members: Vec::new(),
            workspace_excludes: Vec::new(),
            features: features
                .into_iter()
                .map(|(name, deps)| {
                    (
                        name.to_string(),
                        deps.into_iter().map(|dep| dep.to_string()).collect(),
                    )
                })
                .collect(),
        }
    }

    #[test]
    fn default_features_do_not_enable_every_feature_dependency() {
        let manifest = manifest_with_features(vec![
            ("default", Vec::new()),
            ("host", vec!["anyos_std/host"]),
        ]);

        let active = resolve_features(&manifest, &[]);

        assert!(!active.contains(&String::from("host")));
        assert!(!active.contains(&String::from("anyos_std/host")));
    }

    #[test]
    fn default_features_expand_only_default_dependencies() {
        let manifest = manifest_with_features(vec![
            ("default", vec!["serde"]),
            ("host", vec!["anyos_std/host"]),
        ]);

        let active = resolve_features(&manifest, &[]);

        assert!(active.contains(&String::from("serde")));
        assert!(!active.contains(&String::from("host")));
        assert!(!active.contains(&String::from("anyos_std/host")));
    }

    #[test]
    fn default_features_expand_nested_optional_dependency_features() {
        let manifest = manifest_with_features(vec![
            ("default", vec!["crypto"]),
            ("crypto", vec!["dep:chacha20poly1305"]),
        ]);

        let active = resolve_features(&manifest, &[]);

        assert!(active.contains(&String::from("crypto")));
        assert!(active.contains(&String::from("dep:chacha20poly1305")));
    }

    #[test]
    fn explicit_features_expand_nested_dependencies() {
        let manifest = manifest_with_features(vec![
            ("default", Vec::new()),
            ("std", vec!["crypto", "chacha20poly1305/getrandom"]),
            ("crypto", vec!["dep:chacha20poly1305"]),
        ]);

        let active = resolve_features(&manifest, &[String::from("std")]);

        assert!(active.contains(&String::from("std")));
        assert!(active.contains(&String::from("crypto")));
        assert!(active.contains(&String::from("dep:chacha20poly1305")));
        assert!(active.contains(&String::from("chacha20poly1305/getrandom")));
    }
}
