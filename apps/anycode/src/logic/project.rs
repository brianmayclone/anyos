use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;
use crate::util::path;

// ════════════════════════════════════════════════════════════════
//  Project type detection and metadata
// ════════════════════════════════════════════════════════════════

/// Detected project type — determines available tasks, build commands, and UI behavior.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ProjectType {
    Cargo,       // Rust project (Cargo.toml)
    CMake,       // C/C++ project (CMakeLists.txt)
    Make,        // Makefile-based project
    Python,      // Python project (setup.py, pyproject.toml, requirements.txt)
    NodeJS,      // Node.js project (package.json)
    Generic,     // Unknown / single-file project
}

impl ProjectType {
    /// Human-readable name for status bar display.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Cargo => "Cargo (Rust)",
            Self::CMake => "CMake",
            Self::Make => "Makefile",
            Self::Python => "Python",
            Self::NodeJS => "Node.js",
            Self::Generic => "Generic",
        }
    }

    /// Short icon-style label.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Cargo => "Rust",
            Self::CMake => "C/C++",
            Self::Make => "Make",
            Self::Python => "Python",
            Self::NodeJS => "Node.js",
            Self::Generic => "",
        }
    }
}

// ════════════════════════════════════════════════════════════════
//  Cargo target (binary, example, lib, test, bench)
// ════════════════════════════════════════════════════════════════

#[derive(Clone, PartialEq, Debug)]
pub enum TargetKind {
    Binary,
    Library,
    Example,
    Test,
    Bench,
}

impl TargetKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Binary => "bin",
            Self::Library => "lib",
            Self::Example => "example",
            Self::Test => "test",
            Self::Bench => "bench",
        }
    }
}

#[derive(Clone, Debug)]
pub struct CargoTarget {
    pub name: String,
    pub kind: TargetKind,
}

// ════════════════════════════════════════════════════════════════
//  Workspace member (for Cargo workspaces)
// ════════════════════════════════════════════════════════════════

#[derive(Clone, Debug)]
pub struct WorkspaceMember {
    pub path: String,
    pub name: String,
    pub targets: Vec<CargoTarget>,
}

// ════════════════════════════════════════════════════════════════
//  Makefile target
// ════════════════════════════════════════════════════════════════

#[derive(Clone, Debug)]
pub struct MakeTarget {
    pub name: String,
    pub is_phony: bool,
}

// ════════════════════════════════════════════════════════════════
//  Node.js script
// ════════════════════════════════════════════════════════════════

#[derive(Clone, Debug)]
pub struct NpmScript {
    pub name: String,
    pub command: String,
}

// ════════════════════════════════════════════════════════════════
//  Project — the central project model
// ════════════════════════════════════════════════════════════════

pub struct Project {
    pub root: String,
    pub project_type: ProjectType,
    pub name: String,

    // Cargo-specific
    pub cargo_targets: Vec<CargoTarget>,
    pub workspace_members: Vec<WorkspaceMember>,
    pub is_workspace: bool,

    // Makefile-specific
    pub make_targets: Vec<MakeTarget>,

    // Node.js-specific
    pub npm_scripts: Vec<NpmScript>,

    // Legacy compat
    pub build_type: BuildType,
}

/// Legacy build type for backward compatibility with existing build.rs.
#[derive(Clone, Copy, PartialEq)]
pub enum BuildType {
    Make,
    SingleFile,
}

impl Project {
    /// Open a folder as a project — auto-detects type and parses metadata.
    pub fn open(root_path: &str) -> Self {
        let project_type = detect_project_type(root_path);
        let mut proj = Self {
            root: String::from(root_path),
            project_type,
            name: String::from(path::basename(root_path)),
            cargo_targets: Vec::new(),
            workspace_members: Vec::new(),
            is_workspace: false,
            make_targets: Vec::new(),
            npm_scripts: Vec::new(),
            build_type: match project_type {
                ProjectType::Make | ProjectType::CMake => BuildType::Make,
                _ => BuildType::SingleFile,
            },
        };
        proj.scan_metadata();
        proj
    }

    /// Re-scan project metadata (e.g. after file changes).
    pub fn refresh(&mut self) {
        self.project_type = detect_project_type(&self.root);
        self.build_type = match self.project_type {
            ProjectType::Make | ProjectType::CMake => BuildType::Make,
            _ => BuildType::SingleFile,
        };
        self.cargo_targets.clear();
        self.workspace_members.clear();
        self.make_targets.clear();
        self.npm_scripts.clear();
        self.is_workspace = false;
        self.scan_metadata();
    }

    /// Scan and populate project-type-specific metadata.
    fn scan_metadata(&mut self) {
        match self.project_type {
            ProjectType::Cargo => self.scan_cargo(),
            ProjectType::Make => self.scan_makefile(),
            ProjectType::CMake => self.scan_cmake(),
            ProjectType::NodeJS => self.scan_nodejs(),
            ProjectType::Python => self.scan_python(),
            ProjectType::Generic => {}
        }
    }

    // ── Cargo scanning ─────────────────────────────────────────

    fn scan_cargo(&mut self) {
        let toml_path = format!("{}/Cargo.toml", self.root);
        let content = match anyos_std::fs::read_to_string(&toml_path) {
            Ok(s) => s,
            Err(_) => return,
        };

        // Parse package name
        if let Some(name) = toml_value(&content, "package", "name") {
            self.name = name;
        }

        // Check for workspace
        if content.contains("[workspace]") {
            self.is_workspace = true;
            self.scan_workspace_members(&content);
        }

        // Parse explicit [[bin]] targets
        self.parse_cargo_bin_targets(&content);

        // If no explicit [[bin]], check for src/main.rs (implicit binary)
        if self.cargo_targets.iter().all(|t| t.kind != TargetKind::Binary) {
            let main_rs = format!("{}/src/main.rs", self.root);
            if path::exists(&main_rs) {
                self.cargo_targets.push(CargoTarget {
                    name: self.name.clone(),
                    kind: TargetKind::Binary,
                });
            }
        }

        // Check for src/lib.rs
        let lib_rs = format!("{}/src/lib.rs", self.root);
        if path::exists(&lib_rs) {
            self.cargo_targets.push(CargoTarget {
                name: self.name.clone(),
                kind: TargetKind::Library,
            });
        }

        // Scan examples/ directory
        let examples_dir = format!("{}/examples", self.root);
        if path::is_directory(&examples_dir) {
            if let Ok(entries) = anyos_std::fs::read_dir(&examples_dir) {
                for entry in entries {
                    if entry.name.ends_with(".rs") && entry.name != "." && entry.name != ".." {
                        let example_name = &entry.name[..entry.name.len() - 3];
                        self.cargo_targets.push(CargoTarget {
                            name: String::from(example_name),
                            kind: TargetKind::Example,
                        });
                    }
                }
            }
        }

        // Scan tests/ directory
        let tests_dir = format!("{}/tests", self.root);
        if path::is_directory(&tests_dir) {
            if let Ok(entries) = anyos_std::fs::read_dir(&tests_dir) {
                for entry in entries {
                    if entry.name.ends_with(".rs") && entry.name != "." && entry.name != ".." {
                        let test_name = &entry.name[..entry.name.len() - 3];
                        self.cargo_targets.push(CargoTarget {
                            name: String::from(test_name),
                            kind: TargetKind::Test,
                        });
                    }
                }
            }
        }

        // Scan benches/ directory
        let bench_dir = format!("{}/benches", self.root);
        if path::is_directory(&bench_dir) {
            if let Ok(entries) = anyos_std::fs::read_dir(&bench_dir) {
                for entry in entries {
                    if entry.name.ends_with(".rs") && entry.name != "." && entry.name != ".." {
                        let bench_name = &entry.name[..entry.name.len() - 3];
                        self.cargo_targets.push(CargoTarget {
                            name: String::from(bench_name),
                            kind: TargetKind::Bench,
                        });
                    }
                }
            }
        }
    }

    fn parse_cargo_bin_targets(&mut self, content: &str) {
        // Parse [[bin]] sections from TOML
        let mut in_bin_section = false;
        let mut current_name = String::new();

        for line in content.split('\n') {
            let trimmed = line.trim();

            if trimmed == "[[bin]]" {
                if in_bin_section && !current_name.is_empty() {
                    self.cargo_targets.push(CargoTarget {
                        name: current_name.clone(),
                        kind: TargetKind::Binary,
                    });
                }
                in_bin_section = true;
                current_name.clear();
                continue;
            }

            if trimmed.starts_with('[') {
                // New section — flush any pending bin
                if in_bin_section && !current_name.is_empty() {
                    self.cargo_targets.push(CargoTarget {
                        name: current_name.clone(),
                        kind: TargetKind::Binary,
                    });
                }
                in_bin_section = false;
                current_name.clear();
                continue;
            }

            if in_bin_section {
                if let Some(val) = parse_toml_kv(trimmed, "name") {
                    current_name = val;
                }
            }
        }
        // Flush final bin if pending
        if in_bin_section && !current_name.is_empty() {
            self.cargo_targets.push(CargoTarget {
                name: current_name,
                kind: TargetKind::Binary,
            });
        }
    }

    fn scan_workspace_members(&mut self, content: &str) {
        // Parse members = ["path1", "path2"] from [workspace]
        let mut in_workspace = false;
        let mut members_line = String::new();
        let mut collecting = false;

        for line in content.split('\n') {
            let trimmed = line.trim();

            if trimmed == "[workspace]" {
                in_workspace = true;
                continue;
            }
            if trimmed.starts_with('[') && in_workspace {
                break;
            }
            if in_workspace {
                if trimmed.starts_with("members") {
                    members_line = String::from(trimmed);
                    collecting = !trimmed.contains(']');
                    continue;
                }
                if collecting {
                    members_line.push_str(trimmed);
                    if trimmed.contains(']') {
                        collecting = false;
                    }
                }
            }
        }

        // Extract member paths from members = ["a", "b"]
        if let Some(start) = members_line.find('[') {
            if let Some(end) = members_line.find(']') {
                let list = &members_line[start + 1..end];
                for item in list.split(',') {
                    let member = item.trim().trim_matches('"').trim_matches('\'');
                    if member.is_empty() {
                        continue;
                    }
                    // Handle glob patterns like "crates/*"
                    if member.contains('*') {
                        let prefix = member.trim_end_matches('*').trim_end_matches('/');
                        let glob_dir = format!("{}/{}", self.root, prefix);
                        if let Ok(entries) = anyos_std::fs::read_dir(&glob_dir) {
                            for entry in entries {
                                if entry.is_dir() && entry.name != "." && entry.name != ".." {
                                    let member_path = format!("{}/{}", prefix, entry.name);
                                    self.add_workspace_member(&member_path);
                                }
                            }
                        }
                    } else {
                        self.add_workspace_member(member);
                    }
                }
            }
        }
    }

    fn add_workspace_member(&mut self, rel_path: &str) {
        let full_path = format!("{}/{}", self.root, rel_path);
        let toml_path = format!("{}/Cargo.toml", full_path);

        if !path::exists(&toml_path) {
            return;
        }

        let content = match anyos_std::fs::read_to_string(&toml_path) {
            Ok(s) => s,
            Err(_) => return,
        };

        let name = toml_value(&content, "package", "name")
            .unwrap_or_else(|| String::from(path::basename(rel_path)));

        let mut targets = Vec::new();

        // Check for src/main.rs → binary
        if path::exists(&format!("{}/src/main.rs", full_path)) {
            targets.push(CargoTarget {
                name: name.clone(),
                kind: TargetKind::Binary,
            });
        }

        // Check for src/lib.rs → library
        if path::exists(&format!("{}/src/lib.rs", full_path)) {
            targets.push(CargoTarget {
                name: name.clone(),
                kind: TargetKind::Library,
            });
        }

        self.workspace_members.push(WorkspaceMember {
            path: String::from(rel_path),
            name,
            targets,
        });
    }

    // ── Makefile scanning ──────────────────────────────────────

    fn scan_makefile(&mut self) {
        let makefile_path = if path::exists(&format!("{}/Makefile", self.root)) {
            format!("{}/Makefile", self.root)
        } else if path::exists(&format!("{}/makefile", self.root)) {
            format!("{}/makefile", self.root)
        } else if path::exists(&format!("{}/GNUmakefile", self.root)) {
            format!("{}/GNUmakefile", self.root)
        } else {
            return;
        };

        let content = match anyos_std::fs::read_to_string(&makefile_path) {
            Ok(s) => s,
            Err(_) => return,
        };

        let mut phony_targets: Vec<String> = Vec::new();

        for line in content.split('\n') {
            let trimmed = line.trim();

            // Collect .PHONY targets
            if trimmed.starts_with(".PHONY:") || trimmed.starts_with(".PHONY :") {
                let rest = trimmed.splitn(2, ':').nth(1).unwrap_or("");
                for t in rest.split_whitespace() {
                    phony_targets.push(String::from(t));
                }
                continue;
            }

            // Skip variable assignments, comments, recipes (tab-indented)
            if trimmed.is_empty()
                || trimmed.starts_with('#')
                || trimmed.contains('=')
                || line.starts_with('\t')
                || line.starts_with("    ")
            {
                continue;
            }

            // Target lines: "target: deps" or "target:"
            if let Some(colon_pos) = trimmed.find(':') {
                // Skip "::" and "export:" etc.
                if colon_pos == 0 {
                    continue;
                }
                let target_part = &trimmed[..colon_pos];
                // Skip if contains special chars (variables, etc.)
                if target_part.contains('$')
                    || target_part.contains('%')
                    || target_part.contains('(')
                {
                    continue;
                }
                // Can be multiple targets: "target1 target2:"
                for target in target_part.split_whitespace() {
                    if target.starts_with('.') {
                        continue; // Skip .PHONY, .SUFFIXES, etc.
                    }
                    let is_phony = phony_targets.iter().any(|p| p == target);
                    self.make_targets.push(MakeTarget {
                        name: String::from(target),
                        is_phony,
                    });
                }
            }
        }
    }

    // ── CMake scanning ─────────────────────────────────────────

    fn scan_cmake(&mut self) {
        // Basic CMake — detect build directory
        let cmake_path = format!("{}/CMakeLists.txt", self.root);
        let content = match anyos_std::fs::read_to_string(&cmake_path) {
            Ok(s) => s,
            Err(_) => return,
        };

        // Extract project name
        for line in content.split('\n') {
            let trimmed = line.trim();
            if trimmed.starts_with("project(") || trimmed.starts_with("PROJECT(") {
                if let Some(start) = trimmed.find('(') {
                    let rest = &trimmed[start + 1..];
                    let name_end = rest.find(|c: char| c == ' ' || c == ')').unwrap_or(rest.len());
                    let project_name = &rest[..name_end];
                    if !project_name.is_empty() {
                        self.name = String::from(project_name);
                    }
                }
                break;
            }
        }

        // Default make targets for CMake projects
        self.make_targets.push(MakeTarget {
            name: String::from("all"),
            is_phony: true,
        });
        self.make_targets.push(MakeTarget {
            name: String::from("clean"),
            is_phony: true,
        });
        self.make_targets.push(MakeTarget {
            name: String::from("install"),
            is_phony: true,
        });
    }

    // ── Node.js scanning ───────────────────────────────────────

    fn scan_nodejs(&mut self) {
        let pkg_path = format!("{}/package.json", self.root);
        let content = match anyos_std::fs::read_to_string(&pkg_path) {
            Ok(s) => s,
            Err(_) => return,
        };

        if let Ok(val) = anyos_std::json::Value::parse(&content) {
            // Project name
            if let Some(name) = val["name"].as_str() {
                self.name = String::from(name);
            }

            // Scripts
            if let Some(scripts) = val["scripts"].as_object() {
                for (key, value) in scripts.iter() {
                    if let Some(cmd) = value.as_str() {
                        self.npm_scripts.push(NpmScript {
                            name: String::from(key),
                            command: String::from(cmd),
                        });
                    }
                }
            }
        }
    }

    // ── Python scanning ────────────────────────────────────────

    fn scan_python(&mut self) {
        // Try pyproject.toml first
        let pyproject = format!("{}/pyproject.toml", self.root);
        if path::exists(&pyproject) {
            if let Ok(content) = anyos_std::fs::read_to_string(&pyproject) {
                if let Some(name) = toml_value(&content, "project", "name") {
                    self.name = name;
                }
            }
        }
        // Fallback: setup.py
        let setup_py = format!("{}/setup.py", self.root);
        if self.name == path::basename(&self.root) && path::exists(&setup_py) {
            if let Ok(content) = anyos_std::fs::read_to_string(&setup_py) {
                // Basic: look for name="..." in setup()
                for line in content.split('\n') {
                    let trimmed = line.trim();
                    if trimmed.starts_with("name=") || trimmed.starts_with("name =") {
                        if let Some(start) = trimmed.find('"') {
                            if let Some(end) = trimmed[start + 1..].find('"') {
                                self.name = String::from(&trimmed[start + 1..start + 1 + end]);
                            }
                        } else if let Some(start) = trimmed.find('\'') {
                            if let Some(end) = trimmed[start + 1..].find('\'') {
                                self.name = String::from(&trimmed[start + 1..start + 1 + end]);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Get all runnable binary targets (for the run dropdown).
    pub fn runnable_targets(&self) -> Vec<&CargoTarget> {
        self.cargo_targets
            .iter()
            .filter(|t| t.kind == TargetKind::Binary || t.kind == TargetKind::Example)
            .collect()
    }
}

// ════════════════════════════════════════════════════════════════
//  Project type detection
// ════════════════════════════════════════════════════════════════

/// Detect the project type from the root directory.
pub fn detect_project_type(root: &str) -> ProjectType {
    // Priority order: Cargo > CMake > Make > Python > Node > Generic
    if path::exists(&format!("{}/Cargo.toml", root)) {
        ProjectType::Cargo
    } else if path::exists(&format!("{}/CMakeLists.txt", root)) {
        ProjectType::CMake
    } else if path::exists(&format!("{}/Makefile", root))
        || path::exists(&format!("{}/makefile", root))
        || path::exists(&format!("{}/GNUmakefile", root))
    {
        ProjectType::Make
    } else if path::exists(&format!("{}/setup.py", root))
        || path::exists(&format!("{}/pyproject.toml", root))
        || path::exists(&format!("{}/requirements.txt", root))
    {
        ProjectType::Python
    } else if path::exists(&format!("{}/package.json", root)) {
        ProjectType::NodeJS
    } else {
        ProjectType::Generic
    }
}

/// Legacy compat: detect_build_system
pub fn detect_build_system(root: &str) -> BuildType {
    if path::exists(&format!("{}/Makefile", root))
        || path::exists(&format!("{}/makefile", root))
    {
        BuildType::Make
    } else {
        BuildType::SingleFile
    }
}

// ════════════════════════════════════════════════════════════════
//  Simple TOML helpers (no external crate needed)
// ════════════════════════════════════════════════════════════════

/// Extract a value from a TOML section: [section] key = "value"
fn toml_value(content: &str, section: &str, key: &str) -> Option<String> {
    let section_header = format!("[{}]", section);
    let mut in_section = false;

    for line in content.split('\n') {
        let trimmed = line.trim();

        if trimmed == section_header {
            in_section = true;
            continue;
        }
        if trimmed.starts_with('[') {
            if in_section {
                break;
            }
            continue;
        }

        if in_section {
            if let Some(val) = parse_toml_kv(trimmed, key) {
                return Some(val);
            }
        }
    }
    None
}

/// Parse a TOML key = "value" line, returning the unquoted value if the key matches.
fn parse_toml_kv(line: &str, expected_key: &str) -> Option<String> {
    let eq_pos = line.find('=')?;
    let key = line[..eq_pos].trim();
    if key != expected_key {
        return None;
    }
    let val = line[eq_pos + 1..].trim();
    // Strip quotes
    if (val.starts_with('"') && val.ends_with('"'))
        || (val.starts_with('\'') && val.ends_with('\''))
    {
        Some(String::from(&val[1..val.len() - 1]))
    } else {
        Some(String::from(val))
    }
}
