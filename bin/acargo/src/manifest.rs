//! Cargo.toml manifest parsing and data model.

use crate::prelude::*;
use crate::toml::{self, Table, Value};

#[derive(Debug, Clone)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub edition: String,
    pub dependencies: Vec<Dependency>,
    pub crate_type: CrateKind,
    pub bin_name: Option<String>,
    pub bin_path: Option<String>,
    pub lib_name: Option<String>,
    /// Optimization level from [profile.dev] or [profile.release]
    pub opt_level_dev: u32,
    pub opt_level_release: u32,
    /// Panic strategy
    pub panic: String,
    /// Workspace members (if this is a workspace root)
    pub workspace_members: Vec<String>,
    /// Workspace excludes
    pub workspace_excludes: Vec<String>,
    /// Features defined in [features]
    pub features: Vec<(String, Vec<String>)>,
}

#[derive(Debug, Clone)]
pub struct Dependency {
    pub name: String,
    pub path: Option<String>,
    pub version: Option<String>,
    pub optional: bool,
    pub features: Vec<String>,
    pub default_features: bool,
    pub workspace: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrateKind {
    Bin,
    Lib,
}

pub fn parse(toml: &Table) -> Manifest {
    let pkg = toml.get("package").and_then(|v| v.as_table());
    let name = pkg.and_then(|p| p.get("name")).and_then(|v| v.as_str())
        .unwrap_or("unknown").to_string();
    let version = pkg.and_then(|p| p.get("version")).and_then(|v| v.as_str())
        .unwrap_or("0.1.0").to_string();
    let edition = pkg.and_then(|p| p.get("edition")).and_then(|v| v.as_str())
        .unwrap_or("2021").to_string();

    // Dependencies
    let dependencies = parse_dependencies(toml);

    // Crate type
    let mut crate_type = CrateKind::Bin;
    let mut bin_name = None;
    let mut bin_path = None;
    let mut lib_name = None;

    if let Some(Value::Array(bins)) = toml.get("bin") {
        if let Some(Value::Table(bin)) = bins.first() {
            bin_name = bin.get("name").and_then(|v| v.as_str()).map(|s| s.to_string());
            bin_path = bin.get("path").and_then(|v| v.as_str()).map(|s| s.to_string());
        }
    }

    if let Some(lib_table) = toml.get("lib").and_then(|v| v.as_table()) {
        crate_type = CrateKind::Lib;
        lib_name = lib_table.get("name").and_then(|v| v.as_str()).map(|s| s.to_string());
    }

    // Profiles
    let opt_level_dev = parse_profile_opt(toml, "dev").unwrap_or(0);
    let opt_level_release = parse_profile_opt(toml, "release").unwrap_or(2);
    let panic = parse_profile_panic(toml, "dev").unwrap_or_else(|| String::from("unwind"));

    // Workspace
    let mut workspace_members = Vec::new();
    let mut workspace_excludes = Vec::new();
    if let Some(ws) = toml.get("workspace").and_then(|v| v.as_table()) {
        if let Some(members) = ws.get("members").and_then(|v| v.as_array()) {
            for m in members {
                if let Some(s) = m.as_str() {
                    workspace_members.push(s.to_string());
                }
            }
        }
        if let Some(excludes) = ws.get("exclude").and_then(|v| v.as_array()) {
            for e in excludes {
                if let Some(s) = e.as_str() {
                    workspace_excludes.push(s.to_string());
                }
            }
        }
    }

    // Features
    let mut features = Vec::new();
    if let Some(feat_table) = toml.get("features").and_then(|v| v.as_table()) {
        for (feat_name, feat_val) in feat_table.iter() {
            let mut deps = Vec::new();
            if let Some(arr) = feat_val.as_array() {
                for item in arr {
                    if let Some(s) = item.as_str() {
                        deps.push(s.to_string());
                    }
                }
            }
            features.push((feat_name.clone(), deps));
        }
    }

    Manifest {
        name,
        version,
        edition,
        dependencies,
        crate_type,
        bin_name,
        bin_path,
        lib_name,
        opt_level_dev,
        opt_level_release,
        panic,
        workspace_members,
        workspace_excludes,
        features,
    }
}

fn parse_dependencies(toml: &Table) -> Vec<Dependency> {
    let mut deps = Vec::new();

    let dep_table = toml.get("dependencies").and_then(|v| v.as_table());
    if let Some(dt) = dep_table {
        for (dep_name, dep_val) in dt.iter() {
            let dep = parse_single_dep(dep_name, dep_val);
            push_dependency(&mut deps, dep);
        }
    }

    if let Some(targets) = toml.get("target").and_then(|v| v.as_table()) {
        for (_target_cfg, target_val) in targets.iter() {
            let Some(target_table) = target_val.as_table() else {
                continue;
            };
            let Some(dep_table) = target_table.get("dependencies").and_then(|v| v.as_table()) else {
                continue;
            };
            for (dep_name, dep_val) in dep_table.iter() {
                push_dependency(&mut deps, parse_single_dep(dep_name, dep_val));
            }
        }
    }

    deps
}

fn push_dependency(deps: &mut Vec<Dependency>, dep: Dependency) {
    if !deps.iter().any(|existing| existing.name == dep.name) {
        deps.push(dep);
    }
}

pub fn parse_workspace_dependencies(toml: &Table) -> Vec<Dependency> {
    let mut deps = Vec::new();
    let Some(workspace) = toml.get("workspace").and_then(|v| v.as_table()) else {
        return deps;
    };
    let Some(dep_table) = workspace.get("dependencies").and_then(|v| v.as_table()) else {
        return deps;
    };
    for (dep_name, dep_val) in dep_table.iter() {
        deps.push(parse_single_dep(dep_name, dep_val));
    }
    deps
}

fn parse_single_dep(name: &str, val: &Value) -> Dependency {
    match val {
        Value::String(v) => Dependency {
            name: name.to_string(),
            path: None,
            version: Some(v.clone()),
            optional: false,
            features: Vec::new(),
            default_features: true,
            workspace: false,
        },
        Value::Table(t) => {
            let path = t.get("path").and_then(|v| v.as_str()).map(|s| s.to_string());
            let version = t.get("version").and_then(|v| v.as_str()).map(|s| s.to_string());
            let optional = t.get("optional").and_then(|v| v.as_bool()).unwrap_or(false);
            let default_features = t
                .get("default-features")
                .or_else(|| t.get("default_features"))
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let workspace = t.get("workspace").and_then(|v| v.as_bool()).unwrap_or(false);
            let mut features = Vec::new();
            if let Some(arr) = t.get("features").and_then(|v| v.as_array()) {
                for f in arr {
                    if let Some(s) = f.as_str() {
                        features.push(s.to_string());
                    }
                }
            }
            Dependency {
                name: name.to_string(),
                path,
                version,
                optional,
                features,
                default_features,
                workspace,
            }
        }
        _ => Dependency {
            name: name.to_string(),
            path: None,
            version: None,
            optional: false,
            features: Vec::new(),
            default_features: true,
            workspace: false,
        },
    }
}

fn parse_profile_opt(toml: &Table, profile: &str) -> Option<u32> {
    let profiles = toml.get("profile").and_then(|v| v.as_table())?;
    let p = profiles.get(profile).and_then(|v| v.as_table())?;
    // Try both "opt-level" and "opt_level"
    let val = p.get("opt-level").or_else(|| p.get("opt_level"))?;
    val.as_i64().map(|n| n as u32)
}

fn parse_profile_panic(toml: &Table, profile: &str) -> Option<String> {
    let profiles = toml.get("profile").and_then(|v| v.as_table())?;
    let p = profiles.get(profile).and_then(|v| v.as_table())?;
    p.get("panic").and_then(|v| v.as_str()).map(|s| s.to_string())
}

/// Determine the source file for a crate given its manifest and directory.
pub fn source_file(manifest: &Manifest, manifest_dir: &str) -> String {
    if manifest.crate_type == CrateKind::Lib {
        format!("{}/src/lib.rs", manifest_dir)
    } else if let Some(ref path) = manifest.bin_path {
        format!("{}/{}", manifest_dir, path)
    } else {
        format!("{}/src/main.rs", manifest_dir)
    }
}

pub fn infer_crate_layout(manifest: &mut Manifest, manifest_dir: &str) {
    let lib_rs = format!("{}/src/lib.rs", manifest_dir);
    let main_rs = format!("{}/src/main.rs", manifest_dir);
    let has_lib = crate::fs::file_exists(&lib_rs);
    let has_main = crate::fs::file_exists(&main_rs);

    if manifest.lib_name.is_some() {
        manifest.crate_type = CrateKind::Lib;
        return;
    }

    if has_lib && !has_main {
        manifest.crate_type = CrateKind::Lib;
        if manifest.lib_name.is_none() {
            manifest.lib_name = Some(manifest.name.replace('-', "_"));
        }
    } else if has_main {
        manifest.crate_type = CrateKind::Bin;
    }
}
