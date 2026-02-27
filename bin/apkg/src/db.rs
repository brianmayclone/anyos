//! Installed package database (read/write `/System/etc/apkg/installed.json`).

use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::json::{Value, Number};
use anyos_std::fs;
use crate::config;

/// An installed package record.
#[derive(Debug, Clone)]
pub struct InstalledPackage {
    pub name: String,
    pub version: String,
    pub files: Vec<String>,
    pub depends: Vec<String>,
    pub pkg_type: String,
    /// True if installed automatically as a dependency.
    pub auto: bool,
}

/// The installed packages database.
pub struct Database {
    pub packages: Vec<InstalledPackage>,
}

impl Database {
    /// Load the database from disk. Returns empty database on error.
    pub fn load() -> Database {
        let content = match fs::read_to_string(config::INSTALLED_PATH) {
            Ok(s) => s,
            Err(_) => return Database { packages: Vec::new() },
        };
        match Value::parse(&content) {
            Ok(val) => Self::from_json(&val),
            Err(_) => Database { packages: Vec::new() },
        }
    }

    /// Parse database from JSON value.
    fn from_json(val: &Value) -> Database {
        let mut packages = Vec::new();
        if let Some(pkgs_obj) = val["packages"].as_object() {
            for (name, pkg_val) in pkgs_obj.iter() {
                let version = pkg_val["version"].as_str().unwrap_or("0.0.0").into();
                let files = parse_string_array(&pkg_val["files"]);
                let depends = parse_string_array(&pkg_val["depends"]);
                let pkg_type = pkg_val["type"].as_str().unwrap_or("bin").into();
                let auto = pkg_val["auto"].as_bool().unwrap_or(false);
                packages.push(InstalledPackage {
                    name: String::from(name),
                    version,
                    files,
                    depends,
                    pkg_type,
                    auto,
                });
            }
        }
        Database { packages }
    }

    /// Save the database to disk.
    pub fn save(&self) -> bool {
        let json = self.to_json();
        let content = json.to_json_string_pretty();
        match fs::File::create(config::INSTALLED_PATH) {
            Ok(mut f) => {
                use anyos_std::fs::Write;
                f.write_all(content.as_bytes()).is_ok()
            }
            Err(_) => false,
        }
    }

    /// Convert database to JSON.
    fn to_json(&self) -> Value {
        let mut root = Value::new_object();
        let mut pkgs = Value::new_object();
        for pkg in &self.packages {
            let mut obj = Value::new_object();
            obj.set("version", Value::from(pkg.version.as_str()));
            obj.set("type", Value::from(pkg.pkg_type.as_str()));
            obj.set("auto", Value::Bool(pkg.auto));

            let files: Vec<Value> = pkg.files.iter().map(|f| Value::from(f.as_str())).collect();
            obj.set("files", Value::Array(files));

            let deps: Vec<Value> = pkg.depends.iter().map(|d| Value::from(d.as_str())).collect();
            obj.set("depends", Value::Array(deps));

            pkgs.set(&pkg.name, obj);
        }
        root.set("packages", pkgs);
        root
    }

    /// Check if a package is installed.
    pub fn is_installed(&self, name: &str) -> bool {
        self.packages.iter().any(|p| p.name == name)
    }

    /// Get an installed package by name.
    pub fn get(&self, name: &str) -> Option<&InstalledPackage> {
        self.packages.iter().find(|p| p.name == name)
    }

    /// Add or update an installed package record.
    pub fn add(&mut self, pkg: InstalledPackage) {
        if let Some(existing) = self.packages.iter_mut().find(|p| p.name == pkg.name) {
            *existing = pkg;
        } else {
            self.packages.push(pkg);
        }
    }

    /// Remove a package record by name. Returns the removed record.
    pub fn remove(&mut self, name: &str) -> Option<InstalledPackage> {
        if let Some(pos) = self.packages.iter().position(|p| p.name == name) {
            Some(self.packages.remove(pos))
        } else {
            None
        }
    }

    /// Get all packages that depend on the given package name.
    pub fn reverse_deps(&self, name: &str) -> Vec<&InstalledPackage> {
        self.packages.iter().filter(|p| {
            p.depends.iter().any(|d| {
                let dep_name = crate::version::parse_dependency(d).0;
                dep_name == name
            })
        }).collect()
    }

    /// Get auto-installed packages that no other installed package depends on.
    pub fn orphan_auto_packages(&self) -> Vec<&InstalledPackage> {
        self.packages.iter().filter(|p| {
            if !p.auto {
                return false;
            }
            // Check if any other installed package depends on this one
            !self.packages.iter().any(|other| {
                other.name != p.name && other.depends.iter().any(|d| {
                    let dep_name = crate::version::parse_dependency(d).0;
                    dep_name == p.name
                })
            })
        }).collect()
    }
}

/// Parse a JSON array of strings.
fn parse_string_array(val: &Value) -> Vec<String> {
    match val.as_array() {
        Some(arr) => arr.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
        None => Vec::new(),
    }
}
