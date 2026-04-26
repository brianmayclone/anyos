use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::logic::project::{Project, ProjectType};

#[derive(Clone, Debug)]
pub struct SolutionMetadata {
    pub path: String,
    pub startup_project: String,
    pub startup_form: String,
    pub startup_run_config: String,
    pub build_order: Vec<String>,
    pub unloaded_projects: Vec<String>,
}

impl SolutionMetadata {
    pub fn load(project: &Project) -> Self {
        let path = format!("{}/.anycode-workspace", project.root);
        let mut metadata = Self {
            path: path.clone(),
            startup_project: default_startup_project(project),
            startup_form: String::new(),
            startup_run_config: String::new(),
            build_order: default_build_order(project),
            unloaded_projects: Vec::new(),
        };

        let Ok(text) = anyos_std::fs::read_to_string(&path) else {
            return metadata;
        };
        for line in text.split('\n') {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let Some((key, value)) = trimmed.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "startup_project" => metadata.startup_project = String::from(value),
                "startup_form" => metadata.startup_form = String::from(value),
                "startup_run_config" => metadata.startup_run_config = String::from(value),
                "build_order" => metadata.build_order = split_list(value),
                "unloaded_projects" => metadata.unloaded_projects = split_list(value),
                _ => {}
            }
        }
        metadata
    }

    pub fn save(&self) -> Result<(), &'static str> {
        let mut out = String::from("anycode-workspace-v1\n");
        out.push_str(&format!("startup_project={}\n", self.startup_project));
        out.push_str(&format!("startup_form={}\n", self.startup_form));
        out.push_str(&format!("startup_run_config={}\n", self.startup_run_config));
        out.push_str(&format!("build_order={}\n", self.build_order.join(",")));
        out.push_str(&format!(
            "unloaded_projects={}\n",
            self.unloaded_projects.join(",")
        ));
        anyos_std::fs::write_bytes(&self.path, out.as_bytes())
            .map_err(|_| "Could not write .anycode-workspace")
    }

    pub fn project_count(&self, project: &Project) -> usize {
        match project.project_type {
            ProjectType::RustFolder => project.cargo_projects.len(),
            ProjectType::Cargo if project.workspace_members.is_empty() => 1,
            ProjectType::Cargo => project.workspace_members.len() + 1,
            _ => 1,
        }
    }
}

fn default_startup_project(project: &Project) -> String {
    match project.project_type {
        ProjectType::RustFolder => project
            .cargo_projects
            .first()
            .map(|project| project.name.clone())
            .unwrap_or_else(|| project.name.clone()),
        _ => project.name.clone(),
    }
}

fn default_build_order(project: &Project) -> Vec<String> {
    match project.project_type {
        ProjectType::RustFolder => project
            .cargo_projects
            .iter()
            .map(|project| project.name.clone())
            .collect(),
        ProjectType::Cargo => {
            let mut out = Vec::new();
            out.push(project.name.clone());
            for member in &project.workspace_members {
                out.push(member.name.clone());
            }
            out
        }
        _ => {
            let mut out = Vec::new();
            out.push(project.name.clone());
            out
        }
    }
}

fn split_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|item| String::from(item.trim()))
        .filter(|item| !item.is_empty())
        .collect()
}
