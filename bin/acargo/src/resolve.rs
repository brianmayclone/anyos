//! Dependency resolution and topological sorting.
//!
//! Resolves dependencies from both local paths and the crates.io registry.
//! Path-based dependencies take priority; registry dependencies are fetched
//! and cached automatically.

use crate::prelude::*;
use anyos_std::println;
use anyos_std::collections::HashMap;
use crate::manifest::{self, Manifest, CrateKind};
use crate::toml;
use crate::registry;
use crate::lockfile::{self, Lockfile, LockedPackage};
use crate::fs;

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
}

/// Resolve the full dependency graph starting from a root directory.
/// Walks all path-based dependencies recursively, and fetches registry
/// dependencies from crates.io when no path is specified.
pub fn resolve(root_dir: &str) -> Vec<BuildNode> {
    // Load existing Cargo.lock for version pinning
    let lockfile = lockfile::read(root_dir);

    let mut nodes: Vec<BuildNode> = Vec::new();
    let mut name_to_idx: HashMap<String, usize> = HashMap::new();

    // BFS queue: (directory, from_registry)
    let mut queue: Vec<(String, bool)> = Vec::new();
    queue.push((String::from(root_dir), false));

    while let Some((dir, from_registry)) = queue.pop() {
        let manifest_path = format!("{}/Cargo.toml", dir);
        let toml_src = match fs::read_file(&manifest_path) {
            Some(s) => s,
            None => {
                println!("ccargo: warning: cannot read {}", manifest_path);
                continue;
            }
        };
        let toml_table = toml::parse(&toml_src);
        let mf = manifest::parse(&toml_table);

        // Skip if already processed
        if name_to_idx.contains_key(&mf.name) {
            continue;
        }

        let src_file = manifest::source_file(&mf, &dir);
        let idx = nodes.len();
        name_to_idx.insert(mf.name.clone(), idx);

        // Queue dependencies
        for dep in &mf.dependencies {
            if name_to_idx.contains_key(&dep.name) {
                continue;
            }

            if let Some(ref dep_path) = dep.path {
                // Path-based dependency — resolve relative to manifest dir
                let resolved_dir = fs::resolve_path(&dir, dep_path);
                queue.push((resolved_dir, false));
            } else if let Some(ref version_req) = dep.version {
                // Registry dependency — fetch from crates.io
                let actual_name = dep.name.clone();

                // Check lockfile for pinned version first
                let resolved = if let Some(ref lf) = lockfile {
                    if let Some(locked) = lf.find(&actual_name) {
                        // Use pinned version
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
                        queue.push((src_dir, true));
                    }
                    None => {
                        println!("ccargo: error: failed to resolve `{}` {}", actual_name, version_req);
                    }
                }
            }
        }

        nodes.push(BuildNode {
            name: mf.name.clone(),
            manifest_dir: dir,
            manifest: mf,
            src_file,
            deps: Vec::new(),
            from_registry,
        });
    }

    // Second pass: resolve dependency indices
    for i in 0..nodes.len() {
        let mut dep_indices = Vec::new();
        for dep in &nodes[i].manifest.dependencies {
            if let Some(&idx) = name_to_idx.get(&dep.name) {
                dep_indices.push(idx);
            }
        }
        nodes[i].deps = dep_indices;
    }

    // Write updated Cargo.lock if we resolved any registry dependencies
    let has_registry_deps = nodes.iter().any(|n| n.from_registry);
    if has_registry_deps {
        update_lockfile(root_dir, &nodes);
    }

    nodes
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

        let dep_names: Vec<String> = node.deps.iter()
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
        println!("ccargo: error: circular dependency detected ({} of {} resolved)", order.len(), n);
    }

    order
}
