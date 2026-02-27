//! Dependency resolution via topological sort.
//!
//! Resolves the install order for a set of packages, ensuring dependencies
//! are installed before the packages that require them.

use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::println;
use crate::index::{Index, PackageInfo};
use crate::db::Database;
use crate::version::{self, Version};

/// A package scheduled for installation.
#[derive(Debug, Clone)]
pub struct InstallItem {
    /// Package name.
    pub name: String,
    /// Whether this was pulled in as a dependency (auto=true).
    pub auto: bool,
}

/// Resolve dependencies for a list of target packages.
///
/// Returns an ordered list (dependencies first) of packages to install.
/// Packages already installed with a sufficient version are skipped.
pub fn resolve(
    targets: &[&str],
    index: &Index,
    db: &Database,
) -> Result<Vec<InstallItem>, String> {
    let mut resolved: Vec<InstallItem> = Vec::new();
    let mut seen: Vec<String> = Vec::new();

    for target in targets {
        resolve_one(target, false, index, db, &mut resolved, &mut seen)?;
    }

    Ok(resolved)
}

/// Resolve a single package and its dependencies recursively.
fn resolve_one(
    name: &str,
    auto: bool,
    index: &Index,
    db: &Database,
    resolved: &mut Vec<InstallItem>,
    seen: &mut Vec<String>,
) -> Result<(), String> {
    // Already in the resolved list?
    if resolved.iter().any(|r| r.name == name) {
        return Ok(());
    }

    // Cycle detection
    if seen.iter().any(|s| s == name) {
        return Err(alloc::format!("circular dependency detected: {}", name));
    }
    seen.push(String::from(name));

    // Find in index
    let pkg = index.find_provider(name).ok_or_else(|| {
        alloc::format!("package '{}' not found in index", name)
    })?;

    // Check if already installed with sufficient version
    if let Some(installed) = db.get(&pkg.name) {
        let installed_ver = Version::parse(&installed.version)
            .unwrap_or(Version { major: 0, minor: 0, patch: 0 });
        if installed_ver >= pkg.version {
            // Already up to date
            seen.pop();
            return Ok(());
        }
    }

    // Resolve dependencies first
    for dep_str in &pkg.depends {
        let (dep_name, constraint) = version::parse_dependency(dep_str);

        // Check if already installed and satisfies constraint
        if let Some(installed) = db.get(dep_name) {
            if let Some(ref c) = constraint {
                let installed_ver = Version::parse(&installed.version)
                    .unwrap_or(Version { major: 0, minor: 0, patch: 0 });
                if c.satisfied_by(&installed_ver) {
                    continue; // Dependency satisfied
                }
            } else {
                continue; // Any version is fine, already installed
            }
        }

        // Need to install this dependency
        resolve_one(dep_name, true, index, db, resolved, seen)?;
    }

    resolved.push(InstallItem {
        name: String::from(&pkg.name),
        auto,
    });

    seen.pop();
    Ok(())
}
