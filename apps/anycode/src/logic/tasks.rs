use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::logic::config::Config;
use crate::logic::project::{BuildConfiguration, Project, ProjectType, TargetKind};
use crate::logic::rust_backend::RustBuildBackend;

// ════════════════════════════════════════════════════════════════
//  Task definitions — run configurations for the IDE
// ════════════════════════════════════════════════════════════════

/// A task category determines how it appears in the UI.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum TaskCategory {
    Build,
    Run,
    Test,
    Check,
    Clean,
    Custom,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ToolchainKind {
    Cargo,
    RustCompiler,
    CCompiler,
    CxxCompiler,
    Make,
    CMake,
    Ninja,
    Python,
    Node,
    Shell,
    Executable,
    Unknown,
}

impl ToolchainKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Cargo => "ccargo",
            Self::RustCompiler => "crust",
            Self::CCompiler => "C",
            Self::CxxCompiler => "C++",
            Self::Make => "make",
            Self::CMake => "cmake",
            Self::Ninja => "ninja",
            Self::Python => "python",
            Self::Node => "npm",
            Self::Shell => "shell",
            Self::Executable => "executable",
            Self::Unknown => "tool",
        }
    }

    pub fn from_command(command: &str) -> Self {
        let base = command_basename(command);
        match base {
            "ccargo" | "cargo" | "acargo" => Self::Cargo,
            "crust" | "rustc" | "anyrc" => Self::RustCompiler,
            "cc" | "gcc" | "clang" | "tcc" => Self::CCompiler,
            "c++" | "g++" | "clang++" => Self::CxxCompiler,
            "make" => Self::Make,
            "cmake" => Self::CMake,
            "ninja" | "cninja" => Self::Ninja,
            "python" | "python3" => Self::Python,
            "npm" | "node" => Self::Node,
            "sh" | "bash" => Self::Shell,
            _ if command.starts_with("./") || command.starts_with('/') => Self::Executable,
            _ => Self::Unknown,
        }
    }
}

impl TaskCategory {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Build => "Build",
            Self::Run => "Run",
            Self::Test => "Test",
            Self::Check => "Check",
            Self::Clean => "Clean",
            Self::Custom => "Task",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Self::Build => "hammer",
            Self::Run => "player-play",
            Self::Test => "flask",
            Self::Check => "circle-check",
            Self::Clean => "trash",
            Self::Custom => "terminal",
        }
    }
}

/// A task that can be executed — auto-detected or user-defined.
#[derive(Clone, Debug)]
pub struct Task {
    pub name: String,
    pub category: TaskCategory,
    pub command: String,
    pub args: String,
    pub working_dir: String,
    /// For Cargo tasks: optional --bin or --example target name.
    pub target_name: Option<String>,
    /// Whether this task was auto-detected (vs user-defined).
    pub auto_detected: bool,
    /// Structured toolchain classification for UI and safety checks.
    pub toolchain: ToolchainKind,
    /// Display label for the UI (e.g. "cargo run --bin myapp").
    pub display_label: String,
}

impl Task {
    /// Create a new task with all fields.
    pub fn new(
        name: &str,
        category: TaskCategory,
        command: &str,
        args: &str,
        working_dir: &str,
    ) -> Self {
        let display_label = if args.is_empty() {
            format!("{} {}", category.label(), name)
        } else {
            format!(
                "{}: {} {}",
                category.label(),
                command_basename(command),
                args
            )
        };
        Self {
            name: String::from(name),
            category,
            command: String::from(command),
            args: String::from(args),
            working_dir: String::from(working_dir),
            target_name: None,
            auto_detected: true,
            toolchain: ToolchainKind::from_command(command),
            display_label,
        }
    }

    pub fn toolchain_label(&self) -> &'static str {
        self.toolchain.label()
    }
}

// ════════════════════════════════════════════════════════════════
//  Task manager — holds detected + custom tasks
// ════════════════════════════════════════════════════════════════

pub struct TaskManager {
    pub tasks: Vec<Task>,
    pub selected_run_task: usize,
    pub selected_build_task: usize,
}

impl TaskManager {
    pub fn new() -> Self {
        Self {
            tasks: Vec::new(),
            selected_run_task: 0,
            selected_build_task: 0,
        }
    }

    /// Detect tasks from the project and populate the list.
    pub fn detect_from_project(&mut self, project: &Project, config: &Config) {
        self.tasks.clear();
        self.selected_run_task = 0;
        self.selected_build_task = 0;

        match project.project_type {
            ProjectType::Cargo => self.detect_cargo_tasks(project, config),
            ProjectType::RustFolder => self.detect_rust_folder_tasks(project, config),
            ProjectType::CMake | ProjectType::Make | ProjectType::NodeJS => {
                // Studio v1 is intentionally Rust-first. C/TCC/Make and JS tasks
                // are kept out of the active auto-detection path for this phase.
            }
            ProjectType::Python => {}
            ProjectType::Generic => self.detect_generic_tasks(project, config),
        }

        self.normalize_selection();
    }

    fn detect_rust_folder_tasks(&mut self, project: &Project, config: &Config) {
        let rust_backend = RustBuildBackend::from_config(config);
        let ccargo = rust_backend.ccargo_path.clone();
        if ccargo.is_empty() {
            return;
        }
        let cargo_name = command_basename(&ccargo);

        for cargo_project in &project.cargo_projects {
            let build_args = match project.active_configuration {
                BuildConfiguration::Debug => "build",
                BuildConfiguration::Release => "build --release",
            };
            let mut build = Task::new(
                &format!("Build: {}", cargo_project.name),
                TaskCategory::Build,
                &ccargo,
                build_args,
                &cargo_project.root,
            );
            build.display_label =
                format!("{} {} ({})", cargo_name, build_args, cargo_project.rel_path);
            self.tasks.push(build);

            let mut check = Task::new(
                &format!("Check: {}", cargo_project.name),
                TaskCategory::Check,
                &ccargo,
                "check",
                &cargo_project.root,
            );
            check.display_label = format!("{} check ({})", cargo_name, cargo_project.rel_path);
            self.tasks.push(check);

            let mut test = Task::new(
                &format!("Test: {}", cargo_project.name),
                TaskCategory::Test,
                &ccargo,
                "test",
                &cargo_project.root,
            );
            test.display_label = format!("{} test ({})", cargo_name, cargo_project.rel_path);
            self.tasks.push(test);

            for target in &cargo_project.targets {
                match target.kind {
                    TargetKind::Binary => {
                        let args = format!("run --bin {}", target.name);
                        let label = format!("Run: {} / {}", cargo_project.name, target.name);
                        let mut task = Task::new(
                            &label,
                            TaskCategory::Run,
                            &ccargo,
                            &args,
                            &cargo_project.root,
                        );
                        task.target_name = Some(target.name.clone());
                        task.display_label = format!(
                            "{} run --bin {} ({})",
                            cargo_name, target.name, cargo_project.rel_path
                        );
                        self.tasks.push(task);
                    }
                    TargetKind::Example => {
                        let args = format!("run --example {}", target.name);
                        let label = format!("Example: {} / {}", cargo_project.name, target.name);
                        let mut task = Task::new(
                            &label,
                            TaskCategory::Run,
                            &ccargo,
                            &args,
                            &cargo_project.root,
                        );
                        task.target_name = Some(target.name.clone());
                        task.display_label = format!(
                            "{} run --example {} ({})",
                            cargo_name, target.name, cargo_project.rel_path
                        );
                        self.tasks.push(task);
                    }
                    _ => {}
                }
            }

            for run_config in &cargo_project.run_configs {
                let mut args = String::from("run");
                if run_config.profile == BuildConfiguration::Release {
                    args.push_str(" --release");
                }
                match run_config.kind {
                    TargetKind::Example => args.push_str(" --example "),
                    TargetKind::Bench => args.push_str(" --bench "),
                    TargetKind::Test => args.push_str(" --test "),
                    _ => args.push_str(" --bin "),
                }
                args.push_str(&run_config.target);
                if !run_config.args.trim().is_empty() {
                    args.push_str(" -- ");
                    args.push_str(run_config.args.trim());
                }
                let mut task = Task::new(
                    &format!("{} / {}", cargo_project.name, run_config.name),
                    TaskCategory::Run,
                    &ccargo,
                    &args,
                    &cargo_project.root,
                );
                task.auto_detected = false;
                task.target_name = Some(run_config.target.clone());
                task.display_label =
                    format!("{} {} ({})", cargo_name, args, cargo_project.rel_path);
                if !run_config.working_dir.is_empty() && run_config.working_dir != "." {
                    task.working_dir = if run_config.working_dir.starts_with('/') {
                        run_config.working_dir.clone()
                    } else {
                        format!("{}/{}", cargo_project.root, run_config.working_dir)
                    };
                }
                self.tasks.push(task);
            }
        }
    }

    // ── Cargo tasks ────────────────────────────────────────────

    fn detect_cargo_tasks(&mut self, project: &Project, config: &Config) {
        let root = &project.root;
        let rust_backend = RustBuildBackend::from_config(config);
        let ccargo = rust_backend.ccargo_path.clone();
        if ccargo.is_empty() {
            self.detect_rust_fallback_tasks(project, config);
            return;
        }
        let cargo_name = command_basename(&ccargo);

        // Build configurations
        let active_build_args = match project.active_configuration {
            BuildConfiguration::Debug => "build",
            BuildConfiguration::Release => "build --release",
        };
        let mut build = Task::new(
            &format!("Build ({})", project.active_configuration.display_name()),
            TaskCategory::Build,
            &ccargo,
            active_build_args,
            root,
        );
        build.display_label = format!("{} {}", cargo_name, active_build_args);
        self.tasks.push(build);

        let alternate = match project.active_configuration {
            BuildConfiguration::Debug => (BuildConfiguration::Release, "build --release"),
            BuildConfiguration::Release => (BuildConfiguration::Debug, "build"),
        };
        let mut build_alt = Task::new(
            &format!("Build ({})", alternate.0.display_name()),
            TaskCategory::Build,
            &ccargo,
            alternate.1,
            root,
        );
        build_alt.display_label = format!("{} {}", cargo_name, alternate.1);
        self.tasks.push(build_alt);

        // Check
        let mut check = Task::new("Check", TaskCategory::Check, &ccargo, "check", root);
        check.display_label = format!("{} check", cargo_name);
        self.tasks.push(check);

        // Clean
        let mut clean = Task::new("Clean", TaskCategory::Clean, &ccargo, "clean", root);
        clean.display_label = format!("{} clean", cargo_name);
        self.tasks.push(clean);

        // Test
        let mut test = Task::new("Test", TaskCategory::Test, &ccargo, "test", root);
        test.display_label = format!("{} test", cargo_name);
        self.tasks.push(test);

        for case in RustBuildBackend::discover_tests(root) {
            let args = format!("test {}", case.display_name);
            let mut task = Task::new(
                &format!("Test: {}", case.display_name),
                TaskCategory::Test,
                &ccargo,
                &args,
                root,
            );
            task.display_label = format!("{} {}", cargo_name, args);
            self.tasks.push(task);
        }

        // Run targets — one per binary
        for target in &project.cargo_targets {
            match target.kind {
                TargetKind::Binary => {
                    let args = format!("run --bin {}", target.name);
                    let label = format!("Run: {}", target.name);
                    let mut task = Task::new(&label, TaskCategory::Run, &ccargo, &args, root);
                    task.target_name = Some(target.name.clone());
                    task.display_label = format!("{} run --bin {}", cargo_name, target.name);
                    self.tasks.push(task);
                }
                TargetKind::Example => {
                    let args = format!("run --example {}", target.name);
                    let label = format!("Example: {}", target.name);
                    let mut task = Task::new(&label, TaskCategory::Run, &ccargo, &args, root);
                    task.target_name = Some(target.name.clone());
                    task.display_label = format!("{} run --example {}", cargo_name, target.name);
                    self.tasks.push(task);
                }
                TargetKind::Test => {
                    let args = format!("test --test {}", target.name);
                    let label = format!("Test: {}", target.name);
                    let mut task = Task::new(&label, TaskCategory::Test, &ccargo, &args, root);
                    task.target_name = Some(target.name.clone());
                    task.display_label = format!("{} test --test {}", cargo_name, target.name);
                    self.tasks.push(task);
                }
                TargetKind::Bench => {
                    let args = format!("bench --bench {}", target.name);
                    let label = format!("Bench: {}", target.name);
                    let mut task = Task::new(&label, TaskCategory::Run, &ccargo, &args, root);
                    task.target_name = Some(target.name.clone());
                    task.display_label = format!("{} bench --bench {}", cargo_name, target.name);
                    self.tasks.push(task);
                }
                _ => {}
            }
        }

        // Workspace member builds
        for member in &project.workspace_members {
            let args = format!("build -p {}", member.name);
            let label = format!("Build: {}", member.name);
            let mut task = Task::new(&label, TaskCategory::Build, &ccargo, &args, root);
            task.display_label = format!("{} build -p {}", cargo_name, member.name);
            self.tasks.push(task);

            for target in &member.targets {
                if target.kind == TargetKind::Binary {
                    let args = format!("run -p {} --bin {}", member.name, target.name);
                    let label = format!("Run: {} ({})", target.name, member.name);
                    let mut task = Task::new(&label, TaskCategory::Run, &ccargo, &args, root);
                    task.target_name = Some(target.name.clone());
                    task.display_label = format!(
                        "{} run -p {} --bin {}",
                        cargo_name, member.name, target.name
                    );
                    self.tasks.push(task);
                }
            }
        }

        for config in &project.run_configs {
            let mut args = String::from("run");
            if config.profile == BuildConfiguration::Release {
                args.push_str(" --release");
            }
            if !config.package.is_empty() {
                args.push_str(" -p ");
                args.push_str(&config.package);
            }
            match config.kind {
                TargetKind::Example => args.push_str(" --example "),
                TargetKind::Bench => args.push_str(" --bench "),
                TargetKind::Test => args.push_str(" --test "),
                _ => args.push_str(" --bin "),
            }
            args.push_str(&config.target);
            if !config.args.trim().is_empty() {
                args.push_str(" -- ");
                args.push_str(config.args.trim());
            }
            let mut task = Task::new(&config.name, TaskCategory::Run, &ccargo, &args, root);
            task.auto_detected = false;
            task.target_name = Some(config.target.clone());
            task.display_label = format!("{} {}", cargo_name, args);
            if !config.working_dir.is_empty() && config.working_dir != "." {
                task.working_dir = if config.working_dir.starts_with('/') {
                    config.working_dir.clone()
                } else {
                    format!("{}/{}", root, config.working_dir)
                };
            }
            self.tasks.push(task);
        }

        // Set defaults — first build task and first run task
        if let Some(idx) = self
            .tasks
            .iter()
            .position(|t| t.category == TaskCategory::Build)
        {
            self.selected_build_task = idx;
        }
        if let Some(idx) = self
            .tasks
            .iter()
            .position(|t| t.category == TaskCategory::Run)
        {
            self.selected_run_task = idx;
        }
    }

    fn detect_rust_fallback_tasks(&mut self, project: &Project, config: &Config) {
        let root = &project.root;
        let crust = find_first_tool_path(&["crust", "rustc", "anyrc"], config);
        if crust.is_empty() {
            return;
        }

        let main_rs = format!("{}/src/main.rs", root);
        if crate::util::path::exists(&main_rs) {
            let mut build = Task::new(
                "Build (Rust single target)",
                TaskCategory::Build,
                &crust,
                "src/main.rs -o main",
                root,
            );
            build.display_label = format!("{} src/main.rs -o main", command_basename(&crust));
            self.tasks.push(build);

            let mut run = Task::new("Run main", TaskCategory::Run, "./main", "", root);
            run.display_label = String::from("./main");
            self.tasks.push(run);
        }
    }

    // ── CMake tasks ────────────────────────────────────────────

    fn detect_cmake_tasks(&mut self, project: &Project, config: &Config) {
        let root = &project.root;
        let make = find_tool_path("make", config);

        // cmake configure
        let cmake = find_tool_path("cmake", config);
        if !cmake.is_empty() {
            let mut configure = Task::new(
                "Configure",
                TaskCategory::Build,
                &cmake,
                ".. -G Ninja",
                root,
            );
            configure.display_label = String::from("cmake .. -G Ninja");
            configure.working_dir = format!("{}/build", root);
            self.tasks.push(configure);
        }

        // ninja/make build
        let ninja = find_first_tool_path(&["cninja", "ninja"], config);
        if !ninja.is_empty() {
            let mut build = Task::new("Build", TaskCategory::Build, &ninja, "", root);
            build.display_label = String::from(command_basename(&ninja));
            build.working_dir = format!("{}/build", root);
            self.tasks.push(build);

            let mut clean = Task::new("Clean", TaskCategory::Clean, &ninja, "-t clean", root);
            clean.display_label = format!("{} -t clean", command_basename(&ninja));
            clean.working_dir = format!("{}/build", root);
            self.tasks.push(clean);
        } else if !make.is_empty() {
            let mut build = Task::new("Build", TaskCategory::Build, &make, "", root);
            build.display_label = String::from("make");
            build.working_dir = format!("{}/build", root);
            self.tasks.push(build);
        }
    }

    // ── Makefile tasks ─────────────────────────────────────────

    fn detect_make_tasks(&mut self, project: &Project, config: &Config) {
        let root = &project.root;
        let make = find_tool_path("make", config);
        if make.is_empty() {
            return;
        }

        // Default 'make' (build all)
        let mut build = Task::new("Build (all)", TaskCategory::Build, &make, "", root);
        build.display_label = String::from("make");
        self.tasks.push(build);

        // Add detected targets
        for target in &project.make_targets {
            let category = match target.name.as_str() {
                "all" | "build" => TaskCategory::Build,
                "clean" | "distclean" => TaskCategory::Clean,
                "test" | "check" => TaskCategory::Test,
                "run" => TaskCategory::Run,
                "install" => TaskCategory::Custom,
                _ => {
                    if target.is_phony {
                        TaskCategory::Custom
                    } else {
                        TaskCategory::Build
                    }
                }
            };
            // Skip 'all' since we already have the default build
            if target.name == "all" {
                continue;
            }
            let mut task = Task::new(&target.name, category, &make, &target.name, root);
            task.display_label = format!("make {}", target.name);
            self.tasks.push(task);
        }

        // Set defaults
        if let Some(idx) = self
            .tasks
            .iter()
            .position(|t| t.category == TaskCategory::Build)
        {
            self.selected_build_task = idx;
        }
        if let Some(idx) = self
            .tasks
            .iter()
            .position(|t| t.category == TaskCategory::Run)
        {
            self.selected_run_task = idx;
        }
    }

    // ── Python tasks ───────────────────────────────────────────

    fn detect_python_tasks(&mut self, project: &Project, config: &Config) {
        let root = &project.root;
        let python = find_tool_path("python", config);
        if python.is_empty() {
            return;
        }

        // Run main module
        let main_py = format!("{}/main.py", root);
        if crate::util::path::exists(&main_py) {
            let mut run = Task::new("Run main.py", TaskCategory::Run, &python, "main.py", root);
            run.display_label = String::from("python main.py");
            self.tasks.push(run);
        }

        // Run tests
        let mut test = Task::new("Test", TaskCategory::Test, &python, "-m pytest", root);
        test.display_label = String::from("python -m pytest");
        self.tasks.push(test);
    }

    // ── Node.js tasks ──────────────────────────────────────────

    fn detect_nodejs_tasks(&mut self, project: &Project, config: &Config) {
        let root = &project.root;
        let npm = find_tool_path("npm", config);

        for script in &project.npm_scripts {
            let category = match script.name.as_str() {
                "build" => TaskCategory::Build,
                "start" | "dev" | "serve" => TaskCategory::Run,
                "test" => TaskCategory::Test,
                "lint" | "check" => TaskCategory::Check,
                "clean" => TaskCategory::Clean,
                _ => TaskCategory::Custom,
            };
            let args = format!("run {}", script.name);
            let mut task = Task::new(&script.name, category, &npm, &args, root);
            task.display_label = format!("npm run {}", script.name);
            self.tasks.push(task);
        }
    }

    // ── Generic (single-file) tasks ────────────────────────────

    fn detect_generic_tasks(&mut self, project: &Project, config: &Config) {
        let root = &project.root;

        // Rust single file
        let main_rs = format!("{}/main.rs", root);
        let src_main_rs = format!("{}/src/main.rs", root);
        let rust_entry = if crate::util::path::exists(&main_rs) {
            "main.rs"
        } else if crate::util::path::exists(&src_main_rs) {
            "src/main.rs"
        } else {
            ""
        };
        let crust = find_first_tool_path(&["crust", "rustc", "anyrc"], config);
        if !rust_entry.is_empty() && !crust.is_empty() {
            let args = format!("{} -o main", rust_entry);
            let mut build = Task::new("Build (Rust)", TaskCategory::Build, &crust, &args, root);
            build.display_label = format!("{} {}", command_basename(&crust), args);
            self.tasks.push(build);

            let mut run = Task::new("Run", TaskCategory::Run, "./main", "", root);
            run.display_label = String::from("./main");
            self.tasks.push(run);
            return;
        }

        // C/TCC/Make and JavaScript are intentionally outside the Rust-first v1 path.
    }

    // ── Accessors ──────────────────────────────────────────────

    /// Get all tasks of a given category.
    pub fn tasks_by_category(&self, category: TaskCategory) -> Vec<&Task> {
        self.tasks
            .iter()
            .filter(|t| t.category == category)
            .collect()
    }

    /// Get the currently selected run task.
    pub fn selected_run(&self) -> Option<&Task> {
        self.tasks
            .get(self.selected_run_task)
            .filter(|task| task.category == TaskCategory::Run)
            .or_else(|| {
                self.tasks
                    .iter()
                    .find(|task| task.category == TaskCategory::Run)
            })
    }

    /// Get the currently selected build task.
    pub fn selected_build(&self) -> Option<&Task> {
        self.tasks
            .get(self.selected_build_task)
            .filter(|task| task.category == TaskCategory::Build)
            .or_else(|| {
                self.tasks
                    .iter()
                    .find(|task| task.category == TaskCategory::Build)
            })
    }

    /// Build the run configuration label string for dropdown display.
    pub fn run_config_labels(&self) -> String {
        let mut labels = String::new();
        for task in &self.tasks {
            if task.category == TaskCategory::Run {
                if !labels.is_empty() {
                    labels.push('|');
                }
                labels.push_str(&task.name);
            }
        }
        if labels.is_empty() {
            labels.push_str("No run target");
        }
        labels
    }

    pub fn run_task_index_for_dropdown(&self, run_index: usize) -> Option<usize> {
        let mut seen = 0usize;
        for (idx, task) in self.tasks.iter().enumerate() {
            if task.category != TaskCategory::Run {
                continue;
            }
            if seen == run_index {
                return Some(idx);
            }
            seen += 1;
        }
        None
    }

    pub fn selected_run_dropdown_index(&self) -> u32 {
        let mut seen = 0u32;
        for (idx, task) in self.tasks.iter().enumerate() {
            if task.category != TaskCategory::Run {
                continue;
            }
            if idx == self.selected_run_task {
                return seen;
            }
            seen += 1;
        }
        0
    }

    /// Select a run task by name.
    pub fn select_run_by_name(&mut self, name: &str) {
        if let Some(idx) = self
            .tasks
            .iter()
            .position(|t| t.category == TaskCategory::Run && t.name == name)
        {
            self.selected_run_task = idx;
        }
    }

    fn normalize_selection(&mut self) {
        if self
            .tasks
            .get(self.selected_build_task)
            .map(|task| task.category != TaskCategory::Build)
            .unwrap_or(true)
        {
            self.selected_build_task = self
                .tasks
                .iter()
                .position(|task| task.category == TaskCategory::Build)
                .unwrap_or(usize::MAX);
        }
        if self
            .tasks
            .get(self.selected_run_task)
            .map(|task| task.category != TaskCategory::Run)
            .unwrap_or(true)
        {
            self.selected_run_task = self
                .tasks
                .iter()
                .position(|task| task.category == TaskCategory::Run)
                .unwrap_or(usize::MAX);
        }
    }

    /// Build tab labels for task categories in the output panel.
    pub fn task_category_labels(&self) -> String {
        let mut labels = String::new();
        let mut seen_categories: Vec<TaskCategory> = Vec::new();
        for task in &self.tasks {
            if !seen_categories.contains(&task.category) {
                if !labels.is_empty() {
                    labels.push('|');
                }
                labels.push_str(task.category.label());
                seen_categories.push(task.category);
            }
        }
        labels
    }
}

// ════════════════════════════════════════════════════════════════
//  Tool path resolution
// ════════════════════════════════════════════════════════════════

/// Find a tool by name, searching PATH and system directories.
fn find_tool_path(name: &str, config: &Config) -> String {
    match name {
        "ccargo" | "cargo" | "acargo" if !config.ccargo_path.is_empty() => {
            config.ccargo_path.clone()
        }
        "crust" | "rustc" | "anyrc" if !config.crust_path.is_empty() => config.crust_path.clone(),
        "make" if !config.make_path.is_empty() => config.make_path.clone(),
        "cc" | "gcc" | "clang" if !config.cc_path.is_empty() => config.cc_path.clone(),
        "c++" | "g++" | "clang++" if !config.cxx_path.is_empty() => config.cxx_path.clone(),
        "git" if !config.git_path.is_empty() => config.git_path.clone(),
        _ => crate::logic::config::find_tool(name),
    }
}

fn find_first_tool_path(names: &[&str], config: &Config) -> String {
    for name in names {
        let path = find_tool_path(name, config);
        if !path.is_empty() {
            return path;
        }
    }
    String::new()
}

fn command_basename(cmd: &str) -> &str {
    if let Some(pos) = cmd.rfind('/') {
        &cmd[pos + 1..]
    } else {
        cmd
    }
}
