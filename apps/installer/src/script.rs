//! Install script parser: reads install.ini via libini_client and builds
//! a list of steps the worker thread executes sequentially.

use anyos_std::{String, Vec};
use libini_client as ini;

/// A single installation step parsed from install.ini.
pub struct InstallStep {
    /// Human-readable label shown in the UI (e.g. "Installing applications").
    pub label: String,
    /// Action to perform.
    pub action: StepAction,
}

/// What the worker thread does for this step.
pub enum StepAction {
    /// Write MBR bootloader (stage1 + stage2).
    Bootloader,
    /// Create MBR partition table on the target disk.
    Partition,
    /// Format target partition as exFAT.
    Format,
    /// Set root password via chpasswd (before file copy).
    Password,
    /// Create the target user/shared directory skeleton.
    Users,
    /// Extract a .tar.gz archive to a target directory.
    Extract { source: String, target: String },
    /// Recursively copy a directory tree.
    Copy { source: String, target: String },
    /// Write clean boot.cfg to target.
    BootCfg,
    /// Unmount and finalize.
    Finish,
}

/// Default install script path on the ISO.
const INSTALL_SCRIPT: &str = "/install/install.ini";

/// Parse the install script. Returns the step list.
/// Falls back to a hardcoded default if the file can't be read.
pub fn load_steps() -> Vec<InstallStep> {
    if ini::init() {
        if let Some(doc) = ini::IniDoc::load(INSTALL_SCRIPT) {
            let steps = parse_from_doc(&doc);
            if !steps.is_empty() {
                return steps;
            }
        }
    }
    // Fallback: hardcoded steps (works even without libini.so or install.ini)
    default_steps()
}

fn parse_from_doc(doc: &ini::IniDoc) -> Vec<InstallStep> {
    let mut steps = Vec::new();
    let count = doc.section_count();
    for i in 0..count {
        let section_name = match doc.section_name(i) {
            Some(n) => n,
            None => continue,
        };
        // Only process [step:*] sections
        if !section_name.starts_with("step:") {
            continue;
        }
        let label = match doc.get(&section_name, "label") {
            Some(l) => l,
            None => continue,
        };
        let action_str = match doc.get(&section_name, "action") {
            Some(a) => a,
            None => continue,
        };
        let action = match action_str.as_str() {
            "bootloader" => StepAction::Bootloader,
            "partition" => StepAction::Partition,
            "format" => StepAction::Format,
            "password" => StepAction::Password,
            "users" => StepAction::Users,
            "extract" => {
                let source = match doc.get(&section_name, "source") {
                    Some(s) => s,
                    None => continue,
                };
                let target = match doc.get(&section_name, "target") {
                    Some(t) => t,
                    None => continue,
                };
                StepAction::Extract { source, target }
            }
            "copy" => {
                let source = match doc.get(&section_name, "source") {
                    Some(s) => s,
                    None => continue,
                };
                let target = match doc.get(&section_name, "target") {
                    Some(t) => t,
                    None => continue,
                };
                StepAction::Copy { source, target }
            }
            "bootcfg" => StepAction::BootCfg,
            "finish" => StepAction::Finish,
            _ => continue,
        };
        steps.push(InstallStep { label, action });
    }
    steps
}

fn default_steps() -> Vec<InstallStep> {
    let mut s = Vec::new();
    s.push(InstallStep {
        label: String::from("Installing bootloader"),
        action: StepAction::Bootloader,
    });
    s.push(InstallStep {
        label: String::from("Creating partition table"),
        action: StepAction::Partition,
    });
    s.push(InstallStep {
        label: String::from("Formatting filesystem"),
        action: StepAction::Format,
    });
    s.push(InstallStep {
        label: String::from("Setting root password"),
        action: StepAction::Password,
    });
    s.push(InstallStep {
        label: String::from("Copying system files"),
        action: StepAction::Copy {
            source: String::from("/System"),
            target: String::from("/mnt/target/System"),
        },
    });
    s.push(InstallStep {
        label: String::from("Copying applications"),
        action: StepAction::Copy {
            source: String::from("/Applications"),
            target: String::from("/mnt/target/Applications"),
        },
    });
    s.push(InstallStep {
        label: String::from("Setting up user accounts"),
        action: StepAction::Users,
    });
    s.push(InstallStep {
        label: String::from("Copying boot files"),
        action: StepAction::Copy {
            source: String::from("/boot"),
            target: String::from("/mnt/target/boot"),
        },
    });
    s.push(InstallStep {
        label: String::from("Copying media files"),
        action: StepAction::Copy {
            source: String::from("/media"),
            target: String::from("/mnt/target/media"),
        },
    });
    s.push(InstallStep {
        label: String::from("Writing boot configuration"),
        action: StepAction::BootCfg,
    });
    s.push(InstallStep {
        label: String::from("Finishing installation"),
        action: StepAction::Finish,
    });
    s
}
