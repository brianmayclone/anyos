//! Login and autostart launch sections.

use anyos_std::println;
use anyos_std::process;

use super::file::read_conf;

pub fn launch_login_services() {
    let text = match read_conf() {
        Some(t) => t,
        None => return,
    };

    let mut in_login = false;
    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_login = line == "[login]";
            continue;
        }
        if !in_login {
            continue;
        }
        let tid = process::spawn(line, "");
        if tid != 0 {
            println!("compositor: [login] launched '{}' (TID={})", line, tid);
        } else {
            println!("compositor: [login] FAILED to launch '{}'", line);
        }
    }
}

pub fn launch_autostart() -> alloc::vec::Vec<u32> {
    let text = match read_conf() {
        Some(t) => t,
        None => {
            println!("compositor: no compositor.conf found");
            return alloc::vec::Vec::new();
        }
    };

    let mut in_autostart = false;
    let mut tids = alloc::vec::Vec::new();

    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_autostart = line == "[autostart]";
            continue;
        }
        if !in_autostart {
            continue;
        }
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
