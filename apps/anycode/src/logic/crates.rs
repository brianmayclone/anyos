use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::logic::project::{Project, ProjectType};
use crate::util::path;

#[derive(Clone, Debug, PartialEq)]
pub enum DependencyKind {
    Normal,
    Dev,
    Build,
}

impl DependencyKind {
    pub fn section(&self) -> &'static str {
        match self {
            Self::Normal => "dependencies",
            Self::Dev => "dev-dependencies",
            Self::Build => "build-dependencies",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Normal => "Dependencies",
            Self::Dev => "Dev Dependencies",
            Self::Build => "Build Dependencies",
        }
    }

    pub fn from_index(index: u32) -> Self {
        match index {
            1 => Self::Dev,
            2 => Self::Build,
            _ => Self::Normal,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CrateDependency {
    pub manifest_path: String,
    pub package_name: String,
    pub name: String,
    pub version: String,
    pub kind: DependencyKind,
}

pub fn dependencies_for_project(project: &Project) -> Vec<CrateDependency> {
    let mut out = Vec::new();
    for manifest_path in manifest_paths(project) {
        let content = anyos_std::fs::read_to_string(&manifest_path).unwrap_or_default();
        let package_name = package_name_from_manifest(&content)
            .unwrap_or_else(|| String::from(path::basename(path::parent(&manifest_path))));
        parse_dependencies(&manifest_path, &package_name, &content, &mut out);
    }
    out
}

pub fn add_dependency(
    project: &Project,
    name: &str,
    version: &str,
    kind: DependencyKind,
) -> Result<(), &'static str> {
    let manifest_path = primary_manifest_path(project).ok_or("No Cargo.toml found")?;
    let mut content =
        anyos_std::fs::read_to_string(&manifest_path).map_err(|_| "Could not read Cargo.toml")?;
    let trimmed_name = name.trim();
    if trimmed_name.is_empty() {
        return Err("Enter a crate name");
    }
    let trimmed_version = version.trim();
    if trimmed_version.is_empty() {
        return Err("Enter a crate version");
    }
    if dependency_exists(&content, kind.section(), trimmed_name) {
        return Err("Dependency already exists in this manifest");
    }
    insert_dependency(&mut content, kind.section(), trimmed_name, trimmed_version);
    anyos_std::fs::write_bytes(&manifest_path, content.as_bytes())
        .map_err(|_| "Could not update Cargo.toml")
}

pub fn update_dependency(
    project: &Project,
    name: &str,
    version: &str,
    kind: DependencyKind,
) -> Result<(), &'static str> {
    let manifest_path = primary_manifest_path(project).ok_or("No Cargo.toml found")?;
    let content =
        anyos_std::fs::read_to_string(&manifest_path).map_err(|_| "Could not read Cargo.toml")?;
    let updated = replace_dependency_version(&content, kind.section(), name.trim(), version.trim())
        .ok_or("Dependency not found in selected section")?;
    anyos_std::fs::write_bytes(&manifest_path, updated.as_bytes())
        .map_err(|_| "Could not update Cargo.toml")
}

pub fn search_message(query: &str) -> String {
    let q = query.trim();
    if q.is_empty() {
        String::from("Enter a crate name or search term")
    } else {
        format!(
            "Search for '{}' is ready for crates.io backend integration; local Cargo.toml management is active.",
            q
        )
    }
}

pub fn update_check_message(count: usize) -> String {
    format!(
        "Checked {} installed crate dependencies locally; crates.io version lookup is pending registry backend wiring.",
        count
    )
}

fn manifest_paths(project: &Project) -> Vec<String> {
    let mut out = Vec::new();
    let root_manifest = format!("{}/Cargo.toml", project.root);
    if path::exists(&root_manifest) {
        out.push(root_manifest);
    }
    if project.project_type == ProjectType::RustFolder {
        for cargo_project in &project.cargo_projects {
            let manifest = format!("{}/Cargo.toml", cargo_project.root);
            if path::exists(&manifest) && !out.iter().any(|p| p == &manifest) {
                out.push(manifest);
            }
        }
    }
    out
}

fn primary_manifest_path(project: &Project) -> Option<String> {
    manifest_paths(project).into_iter().next()
}

fn parse_dependencies(
    manifest_path: &str,
    package_name: &str,
    content: &str,
    out: &mut Vec<CrateDependency>,
) {
    let mut current_kind: Option<DependencyKind> = None;
    for line in content.split('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            current_kind = match trimmed {
                "[dependencies]" => Some(DependencyKind::Normal),
                "[dev-dependencies]" => Some(DependencyKind::Dev),
                "[build-dependencies]" => Some(DependencyKind::Build),
                _ => None,
            };
            continue;
        }
        let Some(kind) = current_kind.clone() else {
            continue;
        };
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some(eq) = trimmed.find('=') else {
            continue;
        };
        let name = trimmed[..eq].trim();
        if name.is_empty() {
            continue;
        }
        let value = trimmed[eq + 1..].trim();
        out.push(CrateDependency {
            manifest_path: String::from(manifest_path),
            package_name: String::from(package_name),
            name: String::from(name),
            version: parse_version(value),
            kind,
        });
    }
}

fn parse_version(value: &str) -> String {
    if let Some(stripped) = quoted_value(value) {
        return stripped;
    }
    if let Some(pos) = value.find("version") {
        if let Some(eq) = value[pos..].find('=') {
            return quoted_value(value[pos + eq + 1..].trim()).unwrap_or_default();
        }
    }
    String::from(value)
}

fn quoted_value(value: &str) -> Option<String> {
    let v = value.trim();
    let rest = v.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(String::from(&rest[..end]))
}

fn package_name_from_manifest(content: &str) -> Option<String> {
    let mut in_package = false;
    for line in content.split('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_package = trimmed == "[package]";
            continue;
        }
        if in_package && trimmed.starts_with("name") {
            if let Some(eq) = trimmed.find('=') {
                return quoted_value(trimmed[eq + 1..].trim());
            }
        }
    }
    None
}

fn dependency_exists(content: &str, section: &str, name: &str) -> bool {
    let mut in_section = false;
    for line in content.split('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_section = trimmed == format!("[{}]", section);
            continue;
        }
        if in_section && trimmed.starts_with(name) {
            let rest = trimmed[name.len()..].trim_start();
            if rest.starts_with('=') {
                return true;
            }
        }
    }
    false
}

fn insert_dependency(content: &mut String, section: &str, name: &str, version: &str) {
    if !content.ends_with('\n') {
        content.push('\n');
    }
    let header = format!("[{}]", section);
    if !content.contains(&header) {
        content.push('\n');
        content.push_str(&header);
        content.push('\n');
        content.push_str(&format!("{} = \"{}\"\n", name, version));
        return;
    }

    let mut out = String::new();
    let mut inserted = false;
    let mut in_section = false;
    for line in content.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            if in_section && !inserted {
                out.push_str(&format!("{} = \"{}\"\n", name, version));
                inserted = true;
            }
            in_section = trimmed == header;
        }
        out.push_str(line);
    }
    if in_section && !inserted {
        out.push_str(&format!("{} = \"{}\"\n", name, version));
    }
    *content = out;
}

fn replace_dependency_version(
    content: &str,
    section: &str,
    name: &str,
    version: &str,
) -> Option<String> {
    if name.is_empty() || version.is_empty() {
        return None;
    }
    let header = format!("[{}]", section);
    let mut out = String::new();
    let mut in_section = false;
    let mut changed = false;
    for line in content.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_section = trimmed == header;
        }
        if in_section && trimmed.starts_with(name) {
            let rest = trimmed[name.len()..].trim_start();
            if rest.starts_with('=') {
                out.push_str(&format!("{} = \"{}\"\n", name, version));
                changed = true;
                continue;
            }
        }
        out.push_str(line);
    }
    if changed {
        Some(out)
    } else {
        None
    }
}
