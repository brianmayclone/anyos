use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::json::Value;

use crate::logic::project::Project;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum NodeDependencyKind {
    Runtime,
    Dev,
    Optional,
}

impl NodeDependencyKind {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Runtime => "Dependencies",
            Self::Dev => "Dev Dependencies",
            Self::Optional => "Optional Dependencies",
        }
    }

    pub fn json_key(&self) -> &'static str {
        match self {
            Self::Runtime => "dependencies",
            Self::Dev => "devDependencies",
            Self::Optional => "optionalDependencies",
        }
    }

    pub fn from_index(index: u32) -> Self {
        match index {
            1 => Self::Dev,
            2 => Self::Optional,
            _ => Self::Runtime,
        }
    }
}

#[derive(Clone, Debug)]
pub struct NodePackage {
    pub name: String,
    pub version: String,
    pub kind: NodeDependencyKind,
}

#[derive(Clone, Copy, Debug)]
pub struct SuggestedNodePackage {
    pub name: &'static str,
    pub version: &'static str,
    pub kind: NodeDependencyKind,
    pub description: &'static str,
}

pub const SUGGESTED_PACKAGES: &[SuggestedNodePackage] = &[
    SuggestedNodePackage {
        name: "express",
        version: "^4.18.3",
        kind: NodeDependencyKind::Runtime,
        description: "HTTP app/server framework",
    },
    SuggestedNodePackage {
        name: "openai",
        version: "latest",
        kind: NodeDependencyKind::Runtime,
        description: "OpenAI API client",
    },
    SuggestedNodePackage {
        name: "@anthropic-ai/sdk",
        version: "latest",
        kind: NodeDependencyKind::Runtime,
        description: "Claude API client",
    },
    SuggestedNodePackage {
        name: "eslint",
        version: "^8.57.1",
        kind: NodeDependencyKind::Dev,
        description: "JavaScript linting",
    },
    SuggestedNodePackage {
        name: "nodemon",
        version: "latest",
        kind: NodeDependencyKind::Dev,
        description: "Restart app while developing",
    },
];

pub fn packages_for_project(project: &Project) -> Vec<NodePackage> {
    let pkg_path = package_json_path(project);
    let Ok(content) = anyos_std::fs::read_to_string(&pkg_path) else {
        return Vec::new();
    };
    let Ok(value) = Value::parse(&content) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for kind in [
        NodeDependencyKind::Runtime,
        NodeDependencyKind::Dev,
        NodeDependencyKind::Optional,
    ] {
        if let Some(deps) = value[kind.json_key()].as_object() {
            for (name, version) in deps.iter() {
                if let Some(version) = version.as_str() {
                    out.push(NodePackage {
                        name: String::from(name),
                        version: String::from(version),
                        kind,
                    });
                }
            }
        }
    }
    out
}

pub fn add_or_update_package(
    project: &Project,
    name: &str,
    version: &str,
    kind: NodeDependencyKind,
) -> Result<(), &'static str> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Package name is required");
    }
    let version = if version.trim().is_empty() {
        "latest"
    } else {
        version.trim()
    };
    let pkg_path = package_json_path(project);
    let mut root = read_package_json(&pkg_path)?;
    ensure_object_section(&mut root, kind.json_key());
    if let Some(section) = root
        .as_object_mut()
        .and_then(|obj| obj.get_mut(kind.json_key()))
        .and_then(|value| value.as_object_mut())
    {
        section.insert(String::from(name), Value::String(String::from(version)));
    }
    anyos_std::fs::write_bytes(&pkg_path, root.to_json_string_pretty().as_bytes())
        .map_err(|_| "Could not update package.json")
}

pub fn remove_package(project: &Project, name: &str) -> Result<(), &'static str> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Package name is required");
    }
    let pkg_path = package_json_path(project);
    let mut root = read_package_json(&pkg_path)?;
    for key in ["dependencies", "devDependencies", "optionalDependencies"] {
        if let Some(section) = root
            .as_object_mut()
            .and_then(|obj| obj.get_mut(key))
            .and_then(|value| value.as_object_mut())
        {
            section.remove(name);
        }
    }
    anyos_std::fs::write_bytes(&pkg_path, root.to_json_string_pretty().as_bytes())
        .map_err(|_| "Could not update package.json")
}

pub fn update_check_message(count: usize) -> String {
    if count == 0 {
        String::from("No npm package dependencies declared")
    } else {
        format!(
            "{} npm package(s) declared; run npm install/update to refresh",
            count
        )
    }
}

fn package_json_path(project: &Project) -> String {
    format!("{}/package.json", project.root)
}

fn read_package_json(path: &str) -> Result<Value, &'static str> {
    let content = anyos_std::fs::read_to_string(path).map_err(|_| "Could not read package.json")?;
    Value::parse(&content).map_err(|_| "package.json is invalid JSON")
}

fn ensure_object_section(root: &mut Value, key: &str) {
    if !root[key].is_object() {
        root.set(key, Value::new_object());
    }
}
