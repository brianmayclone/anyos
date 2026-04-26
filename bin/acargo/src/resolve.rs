//! Dependency resolution and topological sorting.
//!
//! Resolves dependencies from both local paths and the crates.io registry.
//! Path-based dependencies take priority; registry dependencies are fetched
//! and cached automatically.

use crate::fs;
use crate::lockfile::{self, LockedPackage, Lockfile};
use crate::manifest::{self, CrateKind, Manifest};
use crate::prelude::*;
use crate::registry;
use crate::toml;
use anyos_std::collections::HashMap;
use anyos_std::println;

/// A node in the dependency graph.
#[derive(Debug, Clone)]
pub struct BuildNode {
    pub name: String,
    pub manifest_dir: String,
    pub manifest: Manifest,
    pub src_file: String,
    /// Indices of dependencies in the node list.
    pub deps: Vec<usize>,
    /// Whether this crate was fetched from the registry.
    pub from_registry: bool,
    /// Cargo features active for this crate.
    pub active_features: Vec<String>,
}

/// Resolve the full dependency graph starting from a root directory.
/// Walks all path-based dependencies recursively, and fetches registry
/// dependencies from crates.io when no path is specified.
pub fn resolve(root_dir: &str, root_features: &[String]) -> Vec<BuildNode> {
    let root_dir = fs::absolutize(root_dir);

    // Load existing Cargo.lock for version pinning
    let lockfile = lockfile::read(&root_dir);

    let mut nodes: Vec<BuildNode> = Vec::new();
    let mut name_to_idx: HashMap<String, usize> = HashMap::new();

    // DFS queue: (directory, from_registry, requested features, default features)
    let mut queue: Vec<(String, bool, Vec<String>, bool)> = Vec::new();
    queue.push((root_dir.clone(), false, root_features.to_vec(), true));

    while let Some((dir, from_registry, requested_features, use_default_features)) = queue.pop() {
        let manifest_path = format!("{}/Cargo.toml", dir);
        let toml_src = match fs::read_file(&manifest_path) {
            Some(s) => s,
            None => {
                println!("ccargo: warning: cannot read {}", manifest_path);
                continue;
            }
        };
        let toml_table = toml::parse(&toml_src);
        let mut mf = manifest::parse(&toml_table);
        apply_workspace_dependencies(&mut mf, &dir);
        manifest::infer_crate_layout(&mut mf, &dir);
        inject_implicit_anyos_crates(&mut mf, &dir, dir == root_dir);
        let active_features = active_features_for(&mf, &requested_features, use_default_features);

        // Merge features if this crate was already discovered through another
        // dependency path. Cargo builds each package with the union of all
        // requested features.
        if let Some(&existing_idx) = name_to_idx.get(&mf.name) {
            let mut changed = false;
            for feature in active_features {
                if !nodes[existing_idx].active_features.contains(&feature) {
                    nodes[existing_idx].active_features.push(feature);
                    changed = true;
                }
            }
            if changed {
                let merged = nodes[existing_idx].active_features.clone();
                queue_manifest_dependencies(&mut queue, &mf, &dir, &merged, lockfile.as_ref());
            }
            continue;
        }

        let src_file = manifest::source_file(&mf, &dir);
        let idx = nodes.len();
        name_to_idx.insert(mf.name.clone(), idx);

        // Queue dependencies. Existing packages are still queued so their
        // feature set can be unified if this edge asks for more.
        queue_manifest_dependencies(&mut queue, &mf, &dir, &active_features, lockfile.as_ref());

        nodes.push(BuildNode {
            name: mf.name.clone(),
            manifest_dir: dir,
            manifest: mf,
            src_file,
            deps: Vec::new(),
            from_registry,
            active_features,
        });
    }

    // Second pass: resolve dependency indices
    for i in 0..nodes.len() {
        let mut dep_indices = Vec::new();
        for dep in &nodes[i].manifest.dependencies {
            if !dependency_is_active(dep, &nodes[i].active_features) {
                continue;
            }
            if let Some(&idx) = name_to_idx.get(&dep.name) {
                dep_indices.push(idx);
            }
        }
        nodes[i].deps = dep_indices;
    }

    // Write updated Cargo.lock if we resolved any registry dependencies
    let has_registry_deps = nodes.iter().any(|n| n.from_registry);
    if has_registry_deps {
        update_lockfile(&root_dir, &nodes);
    }

    nodes
}

fn inject_implicit_anyos_crates(
    manifest: &mut Manifest,
    manifest_dir: &str,
    is_root_manifest: bool,
) {
    // `core` and `alloc` are real anyOS source crates for ccargo/anyrc. They
    // are implicit like Rust's own core/alloc, but they are not provided by the
    // runtime linker and must be built before anyos_std or user crates.
    if manifest.name != "core" && !has_dep(manifest, "core") {
        if let Some(core_dir) = find_repo_library_dir(manifest_dir, "core") {
            manifest.dependencies.push(manifest::Dependency {
                name: String::from("core"),
                path: Some(core_dir),
                version: None,
                optional: false,
                features: Vec::new(),
                default_features: true,
                workspace: false,
            });
        }
    }

    if manifest.name != "core" && manifest.name != "alloc" && !has_dep(manifest, "alloc") {
        if let Some(alloc_dir) = find_repo_library_dir(manifest_dir, "alloc") {
            manifest.dependencies.push(manifest::Dependency {
                name: String::from("alloc"),
                path: Some(alloc_dir),
                version: None,
                optional: false,
                features: Vec::new(),
                default_features: true,
                workspace: false,
            });
        }
    }

    if is_root_manifest && manifest.name != "anyos_std" && !has_dep(manifest, "anyos_std") {
        if let Some(stdlib_dir) = find_repo_library_dir(manifest_dir, "stdlib") {
            manifest.dependencies.push(manifest::Dependency {
                name: String::from("anyos_std"),
                path: Some(stdlib_dir),
                version: None,
                optional: false,
                features: Vec::new(),
                default_features: true,
                workspace: false,
            });
        }
    }

    if is_root_manifest
        && manifest.name != "libstd"
        && manifest.name != "anyos_std"
        && !has_dep(manifest, "libstd")
    {
        if let Some(libstd_dir) = find_repo_library_dir(manifest_dir, "libstd") {
            manifest.dependencies.push(manifest::Dependency {
                name: String::from("libstd"),
                path: Some(libstd_dir),
                version: None,
                optional: true,
                features: Vec::new(),
                default_features: true,
                workspace: false,
            });
        }
    }
}

fn active_features_for(
    manifest: &Manifest,
    requested: &[String],
    use_default_features: bool,
) -> Vec<String> {
    let mut requested_with_default = requested.to_vec();
    if !use_default_features && !requested_with_default.iter().any(|f| f == "__no_default__") {
        requested_with_default.push(String::from("__no_default__"));
    }
    crate::workspace::resolve_features(manifest, &requested_with_default)
}

fn queue_manifest_dependencies(
    queue: &mut Vec<(String, bool, Vec<String>, bool)>,
    manifest: &Manifest,
    manifest_dir: &str,
    active_features: &[String],
    lockfile: Option<&Lockfile>,
) {
    for dep in &manifest.dependencies {
        if !dependency_is_active(dep, active_features) {
            continue;
        }
        let dep_requested_features = dependency_requested_features(dep, active_features);
        let dep_default_features = dep.default_features;

        if let Some(ref dep_path) = dep.path {
            let resolved_dir = fs::resolve_path(manifest_dir, dep_path);
            queue.push((
                resolved_dir,
                false,
                dep_requested_features,
                dep_default_features,
            ));
        } else if let Some(ref version_req) = dep.version {
            let actual_name = dep.name.clone();
            let resolved = if let Some(lf) = lockfile {
                if let Some(locked) = lf.find(&actual_name) {
                    let pinned_req = format!("={}", locked.version);
                    registry::get_crate(&actual_name, &pinned_req)
                } else {
                    registry::get_crate(&actual_name, version_req)
                }
            } else {
                registry::get_crate(&actual_name, version_req)
            };

            match resolved {
                Some((src_dir, _entry)) => {
                    queue.push((src_dir, true, dep_requested_features, dep_default_features));
                }
                None => {
                    println!(
                        "ccargo: error: failed to resolve `{}` {}",
                        actual_name, version_req
                    );
                }
            }
        }
    }
}

fn dependency_requested_features(
    dep: &manifest::Dependency,
    active_features: &[String],
) -> Vec<String> {
    let mut requested = dep.features.clone();
    let prefix = format!("{}/", dep.name);
    for feature in active_features {
        if let Some(dep_feature) = feature.strip_prefix(&prefix) {
            push_feature(&mut requested, dep_feature);
        }
    }
    requested
}

fn push_feature(features: &mut Vec<String>, feature: &str) {
    if !features.iter().any(|existing| existing == feature) {
        features.push(feature.to_string());
    }
}

fn dependency_is_active(dep: &manifest::Dependency, active_features: &[String]) -> bool {
    if !dep.optional {
        return true;
    }
    active_features
        .iter()
        .any(|feature| feature == &dep.name || feature == &format!("dep:{}", dep.name))
}

fn apply_workspace_dependencies(manifest: &mut Manifest, manifest_dir: &str) {
    let workspace_deps = load_workspace_dependencies(manifest_dir);
    if workspace_deps.is_empty() {
        return;
    }
    for dep in &mut manifest.dependencies {
        if !dep.workspace {
            continue;
        }
        let Some(inherited) = workspace_deps
            .iter()
            .find(|candidate| candidate.name == dep.name)
        else {
            continue;
        };
        if dep.path.is_none() {
            dep.path = inherited.path.clone();
        }
        if dep.version.is_none() {
            dep.version = inherited.version.clone();
        }
        if dep.features.is_empty() {
            dep.features = inherited.features.clone();
        } else {
            for feature in &inherited.features {
                if !dep.features.contains(feature) {
                    dep.features.push(feature.clone());
                }
            }
        }
        dep.default_features = inherited.default_features && dep.default_features;
        dep.optional = dep.optional || inherited.optional;
    }
}

fn load_workspace_dependencies(start_dir: &str) -> Vec<manifest::Dependency> {
    let mut current = fs::absolutize(start_dir);
    loop {
        let manifest_path = format!("{}/Cargo.toml", current);
        if let Some(src) = fs::read_file(&manifest_path) {
            let table = toml::parse(&src);
            let deps = manifest::parse_workspace_dependencies(&table);
            if !deps.is_empty() {
                return deps;
            }
        }
        if current == "/" || current.is_empty() {
            return Vec::new();
        }
        current = match current.rfind('/') {
            Some(0) => String::from("/"),
            Some(pos) => String::from(&current[..pos]),
            None => return Vec::new(),
        };
    }
}

fn has_dep(manifest: &Manifest, name: &str) -> bool {
    manifest.dependencies.iter().any(|dep| dep.name == name)
}

fn find_repo_library_dir(start_dir: &str, lib_name: &str) -> Option<String> {
    let mut current = fs::absolutize(start_dir);
    loop {
        let candidate = format!("{}/libs/{}/Cargo.toml", current, lib_name);
        if fs::file_exists(&candidate) {
            return Some(format!("{}/libs/{}", current, lib_name));
        }

        if current == "/" || current.is_empty() {
            return None;
        }

        current = match current.rfind('/') {
            Some(0) => String::from("/"),
            Some(pos) => String::from(&current[..pos]),
            None => return None,
        };
    }
}

/// Update or create Cargo.lock with resolved versions.
fn update_lockfile(root_dir: &str, nodes: &[BuildNode]) {
    let mut lockfile = Lockfile::new();

    for node in nodes {
        let source = if node.from_registry {
            Some("registry+https://github.com/rust-lang/crates.io-index".to_string())
        } else {
            None
        };

        let dep_names: Vec<String> = node
            .deps
            .iter()
            .filter_map(|&idx| {
                if idx < nodes.len() {
                    Some(nodes[idx].name.clone())
                } else {
                    None
                }
            })
            .collect();

        lockfile.upsert(LockedPackage {
            name: node.name.clone(),
            version: node.manifest.version.clone(),
            source,
            checksum: None,
            dependencies: dep_names,
        });
    }

    lockfile::write(root_dir, &lockfile);
}

/// Topological sort using Kahn's algorithm.
/// Returns indices in build order (dependencies first).
pub fn topological_sort(nodes: &[BuildNode]) -> Vec<usize> {
    let n = nodes.len();

    // Build adjacency list: dep -> [dependents]
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut in_deg = vec![0u32; n];

    for (i, node) in nodes.iter().enumerate() {
        for &dep in &node.deps {
            if dep < n {
                adj[dep].push(i);
                in_deg[i] += 1;
            }
        }
    }

    // Start with nodes that have no dependencies
    let mut queue: Vec<usize> = Vec::new();
    for i in 0..n {
        if in_deg[i] == 0 {
            queue.push(i);
        }
    }

    let mut order = Vec::new();
    while let Some(node) = queue.pop() {
        order.push(node);
        for &next in &adj[node] {
            in_deg[next] -= 1;
            if in_deg[next] == 0 {
                queue.push(next);
            }
        }
    }

    if order.len() != n {
        println!(
            "ccargo: error: circular dependency detected ({} of {} resolved)",
            order.len(),
            n
        );
    }

    order
}
