#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let args = anyos_std::process::args(&mut args_buf);
    let path = args.trim();

    if path.is_empty() {
        anyos_std::println!("Usage: stat FILE");
        return;
    }

    let mut stat_buf = [0u32; 7];
    if anyos_std::fs::stat(path, &mut stat_buf) != 0 {
        anyos_std::println!("stat: cannot stat '{}'", path);
        return;
    }

    let file_type = stat_buf[0];
    let size = stat_buf[1];
    let flags = stat_buf[2];
    let is_symlink = flags & 1 != 0;

    anyos_std::println!("  File: {}", path);

    // Check if it's a symlink via lstat
    let mut lstat_buf = [0u32; 7];
    let is_link = if anyos_std::fs::lstat(path, &mut lstat_buf) == 0 {
        lstat_buf[2] & 1 != 0
    } else {
        false
    };

    if is_link {
        let mut target_buf = [0u8; 256];
        let n = anyos_std::fs::readlink(path, &mut target_buf);
        if n != u32::MAX && n > 0 {
            let target = core::str::from_utf8(&target_buf[..n as usize]).unwrap_or("?");
            anyos_std::println!("  Link: {} -> {}", path, target);
        }
    }

    let uid = stat_buf[3];
    let gid = stat_buf[4];
    let mode = stat_buf[5];

    if file_type == 1 {
        anyos_std::println!("  Type: directory{}", if is_symlink { " (symlink)" } else { "" });
        anyos_std::println!("  Entries: {}", size);
    } else {
        anyos_std::println!("  Type: regular file{}", if is_symlink { " (symlink)" } else { "" });
        anyos_std::println!("  Size: {} bytes", size);
        if size >= 1024 {
            anyos_std::println!("        ({} KiB)", size / 1024);
        }
    }

    // Format mode as owner/group/others permission string
    let owner = (mode >> 8) & 0xF;
    let group = (mode >> 4) & 0xF;
    let others = mode & 0xF;
    let perm_str = |p: u32| -> &str {
        match p {
            0xF => "rmdc",
            0x7 => "rmd-",
            0x3 => "rm--",
            0x1 => "r---",
            0x0 => "----",
            _ => "????"
        }
    };
    anyos_std::println!("  Mode: 0x{:03X} ({}|{}|{})", mode, perm_str(owner), perm_str(group), perm_str(others));
    anyos_std::println!("   Uid: {}    Gid: {}", uid, gid);
}
