use alloc::string::String;
use alloc::format;
use crate::util::path;

/// Detected build system type.
#[derive(Clone, Copy, PartialEq)]
pub enum BuildType {
    Make,
    SingleFile,
}

/// Project state — tracks the opened folder and build system.
pub struct Project {
    pub root: String,
    pub build_type: BuildType,
}

impl Project {
    /// Open a folder as a project.
    pub fn open(root_path: &str) -> Self {
        let bt = detect_build_system(root_path);
        Self {
            root: String::from(root_path),
            build_type: bt,
        }
    }

    /// Re-detect the build system (e.g. after creating a Makefile).
    pub fn refresh_build_type(&mut self) {
        self.build_type = detect_build_system(&self.root);
    }
}

/// Check what build system is available in the given directory.
pub fn detect_build_system(root: &str) -> BuildType {
    if path::exists(&format!("{}/Makefile", root))
        || path::exists(&format!("{}/makefile", root))
    {
        BuildType::Make
    } else {
        BuildType::SingleFile
    }
}
