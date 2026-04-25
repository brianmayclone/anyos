use crate::util::path;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

// ════════════════════════════════════════════════════════════════
//  Project type detection and metadata
// ════════════════════════════════════════════════════════════════

/// Detected project type — determines available tasks, build commands, and UI behavior.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ProjectType {
    Cargo,      // Rust project (Cargo.toml)
    RustFolder, // Folder containing multiple Rust/Cargo projects
    CMake,      // C/C++ project (CMakeLists.txt)
    Make,       // Makefile-based project
    Python,     // Python project (setup.py, pyproject.toml, requirements.txt)
    NodeJS,     // Node.js project (package.json)
    Generic,    // Unknown / single-file project
}

impl ProjectType {
    /// Human-readable name for status bar display.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Cargo => "Cargo (Rust)",
            Self::RustFolder => "Rust Folder",
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
            Self::RustFolder => "Rust",
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

#[derive(Clone, Debug)]
pub struct CargoRunConfig {
    pub id: String,
    pub name: String,
    pub target: String,
    pub kind: TargetKind,
    pub profile: BuildConfiguration,
    pub args: String,
    pub working_dir: String,
    pub package: String,
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

#[derive(Clone, Debug)]
pub struct CargoProject {
    pub root: String,
    pub rel_path: String,
    pub name: String,
    pub targets: Vec<CargoTarget>,
    pub run_configs: Vec<CargoRunConfig>,
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
    pub configurations: Vec<BuildConfiguration>,
    pub active_configuration: BuildConfiguration,

    // Cargo-specific
    pub cargo_targets: Vec<CargoTarget>,
    pub workspace_members: Vec<WorkspaceMember>,
    pub run_configs: Vec<CargoRunConfig>,
    pub cargo_projects: Vec<CargoProject>,
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

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum BuildConfiguration {
    Debug,
    Release,
}

impl BuildConfiguration {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Debug => "Debug",
            Self::Release => "Release",
        }
    }
}

impl Project {
    /// Open a folder as a project — auto-detects type and parses metadata.
    pub fn open(root_path: &str) -> Self {
        let root = discover_project_root(root_path);
        let project_type = detect_project_type(&root);
        let mut proj = Self {
            root: root.clone(),
            project_type,
            name: String::from(path::basename(&root)),
            configurations: vec![BuildConfiguration::Debug, BuildConfiguration::Release],
            active_configuration: BuildConfiguration::Debug,
            cargo_targets: Vec::new(),
            workspace_members: Vec::new(),
            run_configs: Vec::new(),
            cargo_projects: Vec::new(),
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

    pub fn display_context(&self) -> String {
        if self.project_type == ProjectType::RustFolder {
            return format!(
                "{} | {} projects | {}",
                self.name,
                self.cargo_projects.len(),
                self.active_configuration.display_name()
            );
        }
        format!(
            "{} | {} | {}",
            self.name,
            self.project_type.display_name(),
            self.active_configuration.display_name()
        )
    }

    pub fn target_count(&self) -> usize {
        self.cargo_targets.len()
            + self
                .cargo_projects
                .iter()
                .map(|project| project.targets.len() + project.run_configs.len())
                .sum::<usize>()
            + self.make_targets.len()
            + self.npm_scripts.len()
            + self
                .workspace_members
                .iter()
                .map(|member| member.targets.len())
                .sum::<usize>()
    }

    pub fn set_active_configuration(&mut self, config: BuildConfiguration) {
        self.active_configuration = config;
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
        self.run_configs.clear();
        self.cargo_projects.clear();
        self.make_targets.clear();
        self.npm_scripts.clear();
        self.is_workspace = false;
        self.scan_metadata();
    }

    /// Scan and populate project-type-specific metadata.
    fn scan_metadata(&mut self) {
        match self.project_type {
            ProjectType::Cargo => self.scan_cargo(),
            ProjectType::RustFolder => self.scan_rust_folder(),
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

        self.parse_cargo_run_configs(&content);

        // Check for workspace
        if content.contains("[workspace]") {
            self.is_workspace = true;
            self.scan_workspace_members(&content);
        }

        // Parse explicit [[bin]] targets
        self.parse_cargo_bin_targets(&content);

        // If no explicit [[bin]], check for src/main.rs (implicit binary)
        if self
            .cargo_targets
            .iter()
            .all(|t| t.kind != TargetKind::Binary)
        {
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

    fn parse_cargo_run_configs(&mut self, content: &str) {
        self.run_configs.extend(parse_cargo_run_configs(content));
    }

    fn scan_rust_folder(&mut self) {
        let roots = find_cargo_project_roots(&self.root, 3, 32);
        if roots.is_empty() {
            return;
        }
        self.name = String::from(path::basename(&self.root));
        self.is_workspace = roots.len() > 1;
        for project_root in roots {
            if let Some(project) = scan_cargo_project_at(&self.root, &project_root) {
                self.cargo_projects.push(project);
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
                    let name_end = rest
                        .find(|c: char| c == ' ' || c == ')')
                        .unwrap_or(rest.len());
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
    } else if has_nested_cargo_project(root, 3) {
        ProjectType::RustFolder
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

/// Find the closest project root at or above the selected folder.
///
/// This lets users open `src/` inside a Rust crate and still get Cargo/ccargo
/// tasks instead of falling back to single-file C tasks.
pub fn discover_project_root(root: &str) -> String {
    let mut current = String::from(root.trim_end_matches('/'));
    if current.is_empty() {
        current = String::from("/");
    }

    loop {
        if has_project_marker(&current) {
            return current;
        }
        let parent = path::parent(&current);
        if parent == current || parent == "." || parent.is_empty() {
            break;
        }
        current = String::from(parent);
    }

    String::from(root)
}

fn has_project_marker(root: &str) -> bool {
    path::exists(&format!("{}/Cargo.toml", root))
        || path::exists(&format!("{}/CMakeLists.txt", root))
        || path::exists(&format!("{}/Makefile", root))
        || path::exists(&format!("{}/makefile", root))
        || path::exists(&format!("{}/GNUmakefile", root))
        || path::exists(&format!("{}/setup.py", root))
        || path::exists(&format!("{}/pyproject.toml", root))
        || path::exists(&format!("{}/requirements.txt", root))
        || path::exists(&format!("{}/package.json", root))
}

fn has_nested_cargo_project(root: &str, max_depth: u32) -> bool {
    !find_cargo_project_roots(root, max_depth, 1).is_empty()
}

/// Legacy compat: detect_build_system
pub fn detect_build_system(root: &str) -> BuildType {
    if path::exists(&format!("{}/Makefile", root)) || path::exists(&format!("{}/makefile", root)) {
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

fn parse_cargo_run_configs(content: &str) -> Vec<CargoRunConfig> {
    let mut configs = Vec::new();
    let mut current: Option<CargoRunConfig> = None;

    for line in content.split('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if let Some(cfg) = current.take() {
                push_valid_run_config(&mut configs, cfg);
            }
            current = run_config_id_from_header(trimmed).map(|id| CargoRunConfig {
                name: id.clone(),
                id,
                target: String::new(),
                kind: TargetKind::Binary,
                profile: BuildConfiguration::Debug,
                args: String::new(),
                working_dir: String::from("."),
                package: String::new(),
            });
            continue;
        }

        let Some(cfg) = current.as_mut() else {
            continue;
        };
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(val) = parse_toml_kv(trimmed, "name") {
            cfg.name = val;
        } else if let Some(val) = parse_toml_kv(trimmed, "target") {
            cfg.target = val;
        } else if let Some(val) = parse_toml_kv(trimmed, "kind") {
            cfg.kind = match val.as_str() {
                "example" => TargetKind::Example,
                "bench" => TargetKind::Bench,
                "test" => TargetKind::Test,
                _ => TargetKind::Binary,
            };
        } else if let Some(val) = parse_toml_kv(trimmed, "profile") {
            cfg.profile = if val == "release" {
                BuildConfiguration::Release
            } else {
                BuildConfiguration::Debug
            };
        } else if let Some(val) = parse_toml_kv(trimmed, "args") {
            cfg.args = val;
        } else if let Some(val) = parse_toml_kv(trimmed, "working_dir") {
            cfg.working_dir = val;
        } else if let Some(val) = parse_toml_kv(trimmed, "package") {
            cfg.package = val;
        }
    }

    if let Some(cfg) = current.take() {
        push_valid_run_config(&mut configs, cfg);
    }
    configs
}

fn push_valid_run_config(out: &mut Vec<CargoRunConfig>, mut cfg: CargoRunConfig) {
    if cfg.id.is_empty() {
        cfg.id = run_config_id(&cfg.name);
    }
    if cfg.name.is_empty() {
        cfg.name = cfg.id.clone();
    }
    if cfg.working_dir.is_empty() {
        cfg.working_dir = String::from(".");
    }
    if !cfg.target.is_empty() {
        out.push(cfg);
    }
}

fn scan_cargo_project_at(workspace_root: &str, project_root: &str) -> Option<CargoProject> {
    let manifest_path = format!("{}/Cargo.toml", project_root);
    let content = anyos_std::fs::read_to_string(&manifest_path).ok()?;
    let rel_path = relative_path(workspace_root, project_root);
    let name = toml_value(&content, "package", "name")
        .unwrap_or_else(|| String::from(path::basename(project_root)));
    let targets = discover_cargo_targets(project_root, &content, &name);
    let run_configs = parse_cargo_run_configs(&content);
    Some(CargoProject {
        root: String::from(project_root),
        rel_path,
        name,
        targets,
        run_configs,
    })
}

fn discover_cargo_targets(root: &str, content: &str, package_name: &str) -> Vec<CargoTarget> {
    let mut targets = Vec::new();
    append_explicit_bin_targets(&mut targets, content);
    if targets.iter().all(|t| t.kind != TargetKind::Binary)
        && path::exists(&format!("{}/src/main.rs", root))
    {
        targets.push(CargoTarget {
            name: String::from(package_name),
            kind: TargetKind::Binary,
        });
    }
    if path::exists(&format!("{}/src/lib.rs", root)) {
        targets.push(CargoTarget {
            name: String::from(package_name),
            kind: TargetKind::Library,
        });
    }
    append_rs_dir_targets(&mut targets, root, "examples", TargetKind::Example);
    append_rs_dir_targets(&mut targets, root, "tests", TargetKind::Test);
    append_rs_dir_targets(&mut targets, root, "benches", TargetKind::Bench);
    targets
}

fn append_explicit_bin_targets(targets: &mut Vec<CargoTarget>, content: &str) {
    let mut in_bin_section = false;
    let mut current_name = String::new();
    for line in content.split('\n') {
        let trimmed = line.trim();
        if trimmed == "[[bin]]" {
            if in_bin_section && !current_name.is_empty() {
                targets.push(CargoTarget {
                    name: current_name.clone(),
                    kind: TargetKind::Binary,
                });
            }
            in_bin_section = true;
            current_name.clear();
            continue;
        }
        if trimmed.starts_with('[') {
            if in_bin_section && !current_name.is_empty() {
                targets.push(CargoTarget {
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
    if in_bin_section && !current_name.is_empty() {
        targets.push(CargoTarget {
            name: current_name,
            kind: TargetKind::Binary,
        });
    }
}

fn append_rs_dir_targets(targets: &mut Vec<CargoTarget>, root: &str, dir: &str, kind: TargetKind) {
    let full_dir = format!("{}/{}", root, dir);
    if !path::is_directory(&full_dir) {
        return;
    }
    if let Ok(entries) = anyos_std::fs::read_dir(&full_dir) {
        for entry in entries {
            if entry.name.ends_with(".rs") && entry.name != "." && entry.name != ".." {
                let name = &entry.name[..entry.name.len() - 3];
                targets.push(CargoTarget {
                    name: String::from(name),
                    kind: kind.clone(),
                });
            }
        }
    }
}

fn find_cargo_project_roots(root: &str, max_depth: u32, max_count: usize) -> Vec<String> {
    let mut roots = Vec::new();
    collect_cargo_project_roots(root, root, 0, max_depth, max_count, &mut roots);
    roots
}

fn collect_cargo_project_roots(
    workspace_root: &str,
    dir: &str,
    depth: u32,
    max_depth: u32,
    max_count: usize,
    roots: &mut Vec<String>,
) {
    if roots.len() >= max_count || depth > max_depth {
        return;
    }
    if dir != workspace_root && path::exists(&format!("{}/Cargo.toml", dir)) {
        roots.push(String::from(dir));
        return;
    }
    if depth == max_depth {
        return;
    }
    let Ok(entries) = anyos_std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        if roots.len() >= max_count {
            break;
        }
        if !entry.is_dir() || should_skip_project_dir(&entry.name) {
            continue;
        }
        collect_cargo_project_roots(
            workspace_root,
            &format!("{}/{}", dir, entry.name),
            depth + 1,
            max_depth,
            max_count,
            roots,
        );
    }
}

fn should_skip_project_dir(name: &str) -> bool {
    matches!(
        name,
        "." | ".." | ".git" | "target" | "build" | "node_modules" | ".cache"
    )
}

fn relative_path(root: &str, path: &str) -> String {
    let prefix = if root.ends_with('/') {
        String::from(root)
    } else {
        format!("{}/", root)
    };
    if let Some(rel) = path.strip_prefix(&prefix) {
        String::from(rel)
    } else {
        String::from(path)
    }
}

fn run_config_id_from_header(header: &str) -> Option<String> {
    let inner = header.trim_matches(|c| c == '[' || c == ']');
    let prefix_workspace = "workspace.metadata.anycode.run.";
    let prefix_package = "package.metadata.anycode.run.";
    let id = inner
        .strip_prefix(prefix_workspace)
        .or_else(|| inner.strip_prefix(prefix_package))?;
    let clean = id.trim().trim_matches('"').trim_matches('\'');
    if clean.is_empty() {
        None
    } else {
        Some(String::from(clean))
    }
}

pub fn run_config_id(name: &str) -> String {
    let mut id = String::new();
    for b in name.bytes() {
        let c = match b {
            b'a'..=b'z' | b'0'..=b'9' => b as char,
            b'A'..=b'Z' => (b + 32) as char,
            b'-' | b'_' => b as char,
            _ => '-',
        };
        if c == '-' && id.ends_with('-') {
            continue;
        }
        id.push(c);
    }
    while id.ends_with('-') {
        id.pop();
    }
    if id.is_empty() {
        String::from("run")
    } else {
        id
    }
}

pub fn save_cargo_run_config(root: &str, config: &CargoRunConfig) -> Result<(), &'static str> {
    let manifest_path = format!("{}/Cargo.toml", root);
    let content =
        anyos_std::fs::read_to_string(&manifest_path).map_err(|_| "Cargo.toml not readable")?;
    let id = run_config_id(&config.name);
    let header = if content.contains("[workspace]") {
        format!("[workspace.metadata.anycode.run.{}]", id)
    } else {
        format!("[package.metadata.anycode.run.{}]", id)
    };
    let content = remove_run_config_section(&content, &id);
    let mut out = String::from(content.trim_end());
    out.push_str("\n\n");
    out.push_str(&header);
    out.push('\n');
    out.push_str(&format!("name = \"{}\"\n", toml_escape(&config.name)));
    out.push_str(&format!("target = \"{}\"\n", toml_escape(&config.target)));
    out.push_str(&format!("kind = \"{}\"\n", config.kind.label()));
    out.push_str(&format!(
        "profile = \"{}\"\n",
        match config.profile {
            BuildConfiguration::Debug => "debug",
            BuildConfiguration::Release => "release",
        }
    ));
    out.push_str(&format!("args = \"{}\"\n", toml_escape(&config.args)));
    out.push_str(&format!(
        "working_dir = \"{}\"\n",
        toml_escape(&config.working_dir)
    ));
    if !config.package.is_empty() {
        out.push_str(&format!("package = \"{}\"\n", toml_escape(&config.package)));
    }
    anyos_std::fs::write_bytes(&manifest_path, out.as_bytes())
        .map_err(|_| "Cargo.toml not writable")
}

fn remove_run_config_section(content: &str, id: &str) -> String {
    let mut out = String::new();
    let mut skipping = false;
    for line in content.split('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let matches = run_config_id_from_header(trimmed)
                .map(|section_id| section_id == id)
                .unwrap_or(false);
            skipping = matches;
        }
        if !skipping {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn toml_escape(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            _ => out.push(ch),
        }
    }
    out
}
