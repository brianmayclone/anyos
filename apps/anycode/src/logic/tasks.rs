use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;

use crate::logic::project::{Project, ProjectType, TargetKind};

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
            format!("{}: {} {}", category.label(), command_basename(command), args)
        };
        Self {
            name: String::from(name),
            category,
            command: String::from(command),
            args: String::from(args),
            working_dir: String::from(working_dir),
            target_name: None,
            auto_detected: true,
            display_label,
        }
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
    pub fn detect_from_project(&mut self, project: &Project) {
        self.tasks.clear();
        self.selected_run_task = 0;
        self.selected_build_task = 0;

        match project.project_type {
            ProjectType::Cargo => self.detect_cargo_tasks(project),
            ProjectType::CMake => self.detect_cmake_tasks(project),
            ProjectType::Make => self.detect_make_tasks(project),
            ProjectType::Python => self.detect_python_tasks(project),
            ProjectType::NodeJS => self.detect_nodejs_tasks(project),
            ProjectType::Generic => self.detect_generic_tasks(project),
        }
    }

    // ── Cargo tasks ────────────────────────────────────────────

    fn detect_cargo_tasks(&mut self, project: &Project) {
        let root = &project.root;
        let acargo = find_tool_path("acargo");
        if acargo.is_empty() {
            return;
        }

        // Build
        let mut build = Task::new("Build", TaskCategory::Build, &acargo, "build", root);
        build.display_label = String::from("acargo build");
        self.tasks.push(build);

        // Build --release
        let mut build_rel = Task::new("Build (Release)", TaskCategory::Build, &acargo, "build --release", root);
        build_rel.display_label = String::from("acargo build --release");
        self.tasks.push(build_rel);

        // Check
        let mut check = Task::new("Check", TaskCategory::Check, &acargo, "check", root);
        check.display_label = String::from("acargo check");
        self.tasks.push(check);

        // Clean
        let mut clean = Task::new("Clean", TaskCategory::Clean, &acargo, "clean", root);
        clean.display_label = String::from("acargo clean");
        self.tasks.push(clean);

        // Test
        let mut test = Task::new("Test", TaskCategory::Test, &acargo, "test", root);
        test.display_label = String::from("acargo test");
        self.tasks.push(test);

        // Run targets — one per binary
        for target in &project.cargo_targets {
            match target.kind {
                TargetKind::Binary => {
                    let args = format!("run --bin {}", target.name);
                    let label = format!("Run: {}", target.name);
                    let mut task = Task::new(&label, TaskCategory::Run, &acargo, &args, root);
                    task.target_name = Some(target.name.clone());
                    task.display_label = format!("acargo run --bin {}", target.name);
                    self.tasks.push(task);
                }
                TargetKind::Example => {
                    let args = format!("run --example {}", target.name);
                    let label = format!("Example: {}", target.name);
                    let mut task = Task::new(&label, TaskCategory::Run, &acargo, &args, root);
                    task.target_name = Some(target.name.clone());
                    task.display_label = format!("acargo run --example {}", target.name);
                    self.tasks.push(task);
                }
                TargetKind::Test => {
                    let args = format!("test --test {}", target.name);
                    let label = format!("Test: {}", target.name);
                    let mut task = Task::new(&label, TaskCategory::Test, &acargo, &args, root);
                    task.target_name = Some(target.name.clone());
                    task.display_label = format!("acargo test --test {}", target.name);
                    self.tasks.push(task);
                }
                TargetKind::Bench => {
                    let args = format!("bench --bench {}", target.name);
                    let label = format!("Bench: {}", target.name);
                    let mut task = Task::new(&label, TaskCategory::Run, &acargo, &args, root);
                    task.target_name = Some(target.name.clone());
                    task.display_label = format!("acargo bench --bench {}", target.name);
                    self.tasks.push(task);
                }
                _ => {}
            }
        }

        // Workspace member builds
        for member in &project.workspace_members {
            let args = format!("build -p {}", member.name);
            let label = format!("Build: {}", member.name);
            let mut task = Task::new(&label, TaskCategory::Build, &acargo, &args, root);
            task.display_label = format!("acargo build -p {}", member.name);
            self.tasks.push(task);

            for target in &member.targets {
                if target.kind == TargetKind::Binary {
                    let args = format!("run -p {} --bin {}", member.name, target.name);
                    let label = format!("Run: {} ({})", target.name, member.name);
                    let mut task = Task::new(&label, TaskCategory::Run, &acargo, &args, root);
                    task.target_name = Some(target.name.clone());
                    task.display_label = format!("acargo run -p {} --bin {}", member.name, target.name);
                    self.tasks.push(task);
                }
            }
        }

        // Set defaults — first build task and first run task
        if let Some(idx) = self.tasks.iter().position(|t| t.category == TaskCategory::Build) {
            self.selected_build_task = idx;
        }
        if let Some(idx) = self.tasks.iter().position(|t| t.category == TaskCategory::Run) {
            self.selected_run_task = idx;
        }
    }

    // ── CMake tasks ────────────────────────────────────────────

    fn detect_cmake_tasks(&mut self, project: &Project) {
        let root = &project.root;
        let make = find_tool_path("make");

        // cmake configure
        let cmake = find_tool_path("cmake");
        if !cmake.is_empty() {
            let mut configure = Task::new("Configure", TaskCategory::Build, &cmake, ".. -G Ninja", root);
            configure.display_label = String::from("cmake .. -G Ninja");
            configure.working_dir = format!("{}/build", root);
            self.tasks.push(configure);
        }

        // ninja/make build
        let ninja = find_tool_path("ninja");
        if !ninja.is_empty() {
            let mut build = Task::new("Build", TaskCategory::Build, &ninja, "", root);
            build.display_label = String::from("ninja");
            build.working_dir = format!("{}/build", root);
            self.tasks.push(build);

            let mut clean = Task::new("Clean", TaskCategory::Clean, &ninja, "-t clean", root);
            clean.display_label = String::from("ninja -t clean");
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

    fn detect_make_tasks(&mut self, project: &Project) {
        let root = &project.root;
        let make = find_tool_path("make");
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
        if let Some(idx) = self.tasks.iter().position(|t| t.category == TaskCategory::Build) {
            self.selected_build_task = idx;
        }
        if let Some(idx) = self.tasks.iter().position(|t| t.category == TaskCategory::Run) {
            self.selected_run_task = idx;
        }
    }

    // ── Python tasks ───────────────────────────────────────────

    fn detect_python_tasks(&mut self, project: &Project) {
        let root = &project.root;
        let python = find_tool_path("python");
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

    fn detect_nodejs_tasks(&mut self, project: &Project) {
        let root = &project.root;
        let npm = find_tool_path("npm");

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

    fn detect_generic_tasks(&mut self, project: &Project) {
        let root = &project.root;

        // C single file
        let cc = find_tool_path("cc");
        if !cc.is_empty() {
            let mut build = Task::new("Build (C)", TaskCategory::Build, &cc, "main.c -o main", root);
            build.display_label = String::from("cc main.c -o main");
            self.tasks.push(build);

            let mut run = Task::new("Run", TaskCategory::Run, "./main", "", root);
            run.display_label = String::from("./main");
            self.tasks.push(run);
        }

        // Rust single file
        let anyrc = find_tool_path("anyrc");
        if !anyrc.is_empty() {
            let mut build = Task::new("Build (Rust)", TaskCategory::Build, &anyrc, "main.rs -o main", root);
            build.display_label = String::from("anyrc main.rs -o main");
            self.tasks.push(build);
        }
    }

    // ── Accessors ──────────────────────────────────────────────

    /// Get all tasks of a given category.
    pub fn tasks_by_category(&self, category: TaskCategory) -> Vec<&Task> {
        self.tasks.iter().filter(|t| t.category == category).collect()
    }

    /// Get the currently selected run task.
    pub fn selected_run(&self) -> Option<&Task> {
        self.tasks.get(self.selected_run_task)
    }

    /// Get the currently selected build task.
    pub fn selected_build(&self) -> Option<&Task> {
        self.tasks.get(self.selected_build_task)
    }

    /// Build the run configuration label string for dropdown display.
    pub fn run_config_labels(&self) -> String {
        let mut labels = String::new();
        for (i, task) in self.tasks.iter().enumerate() {
            if task.category == TaskCategory::Run {
                if !labels.is_empty() {
                    labels.push('|');
                }
                labels.push_str(&task.name);
                let _ = i; // used for selection tracking
            }
        }
        labels
    }

    /// Select a run task by name.
    pub fn select_run_by_name(&mut self, name: &str) {
        if let Some(idx) = self.tasks.iter().position(|t| t.category == TaskCategory::Run && t.name == name) {
            self.selected_run_task = idx;
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
fn find_tool_path(name: &str) -> String {
    crate::logic::config::find_tool(name)
}

fn command_basename(cmd: &str) -> &str {
    if let Some(pos) = cmd.rfind('/') {
        &cmd[pos + 1..]
    } else {
        cmd
    }
}
