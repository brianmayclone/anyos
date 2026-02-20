//! Compositor configuration — reads compositor.conf and launches startup programs.

use anyos_std::println;
use anyos_std::process;

/// Read /System/compositor/compositor.conf and spawn each listed program.
pub fn launch_compositor_conf() {
    use anyos_std::fs;

    let conf_path = "/System/compositor/compositor.conf";
    let fd = fs::open(conf_path, 0);
    if fd == u32::MAX {
        println!("compositor: no compositor.conf found");
        return;
    }

    let mut buf = [0u8; 1024];
    let n = fs::read(fd, &mut buf) as usize;
    fs::close(fd);

    if n == 0 {
        return;
    }

    let text = match core::str::from_utf8(&buf[..n]) {
        Ok(s) => s,
        Err(_) => return,
    };

    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let tid = process::spawn(line, "");
        if tid != 0 {
            println!("compositor: launched '{}' (TID={})", line, tid);
        } else {
            println!("compositor: FAILED to launch '{}'", line);
        }
    }
}
