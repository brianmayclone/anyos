//! Login and autostart launch sections.

use anyos_std::println;
use anyos_std::process;
use alloc::vec::Vec;

use super::file::{read_string, register_manifest};

fn collect_program_lines<'a>(text: &'a str, section_name: &str) -> Vec<&'a str> {
    let has_sections = text.lines().any(|line| line.trim_start().starts_with('['));
    let mut in_target_section = !has_sections;
    let mut lines = Vec::new();

    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_target_section = line == section_name;
            continue;
        }
        if in_target_section {
            lines.push(line);
        }
    }

    lines
}

pub fn launch_login_services() {
    register_manifest();
    let text = match read_string("login/programs_blob") {
        Some(t) => t,
        None => return,
    };

    for line in collect_program_lines(&text, "[login]") {
        let tid = process::spawn(line, "");
        if tid != 0 {
            println!("compositor: [login] launched '{}' (TID={})", line, tid);
        } else {
            println!("compositor: [login] FAILED to launch '{}'", line);
        }
    }
}

pub fn launch_autostart() -> alloc::vec::Vec<u32> {
    register_manifest();
    let text = match read_string("autostart/programs_blob") {
        Some(t) => t,
        None => {
            println!("compositor: no autostart configuration found");
            return alloc::vec::Vec::new();
        }
    };

    let mut tids = alloc::vec::Vec::new();

    for line in collect_program_lines(&text, "[autostart]") {
        let tid = process::spawn(line, "");
        if tid != 0 && tid != u32::MAX {
            println!("compositor: launched '{}' (TID={})", line, tid);
            tids.push(tid);
        } else {
            println!("compositor: FAILED to launch '{}'", line);
        }
    }
    tids
}
