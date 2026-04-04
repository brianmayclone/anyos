//! Dependency resolution and topological sorting.

use crate::prelude::*;
use anyos_std::println;
use anyos_std::collections::HashMap;
use crate::manifest::{self, Manifest, CrateKind};
use crate::toml;
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
}

/// Resolve the full dependency graph starting from a root directory.
/// Walks all path-based dependencies recursively.
pub fn resolve(root_dir: &str) -> Vec<BuildNode> {
    let mut nodes: Vec<BuildNode> = Vec::new();
    let mut name_to_idx: HashMap<String, usize> = HashMap::new();

    // BFS queue: directories to process
    let mut queue: Vec<String> = Vec::new();
    queue.push(String::from(root_dir));

    while let Some(dir) = queue.pop() {
        let manifest_path = format!("{}/Cargo.toml", dir);
        let toml_src = match fs::read_file(&manifest_path) {
            Some(s) => s,
            None => {
                println!("acargo: warning: cannot read {}", manifest_path);
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

        // Queue path-based dependencies
        for dep in &mf.dependencies {
            if let Some(ref dep_path) = dep.path {
                let resolved_dir = fs::resolve_path(&dir, dep_path);
                if !name_to_idx.contains_key(&dep.name) {
                    queue.push(resolved_dir);
                }
            }
        }

        nodes.push(BuildNode {
            name: mf.name.clone(),
            manifest_dir: dir,
            manifest: mf,
            src_file,
            deps: Vec::new(),
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

    nodes
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
        println!("acargo: error: circular dependency detected ({} of {} resolved)", order.len(), n);
    }

    order
}
