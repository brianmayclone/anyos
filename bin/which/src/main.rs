#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let args = anyos_std::process::args(&mut args_buf);
    let cmd = args.trim();

    if cmd.is_empty() {
        anyos_std::println!("Usage: which COMMAND");
        return;
    }

    // Read PATH from environment (fallback to defaults)
    let mut path_buf = [0u8; 256];
    let len = anyos_std::env::get("PATH", &mut path_buf);
    let path_str = if len != u32::MAX {
        core::str::from_utf8(&path_buf[..len as usize]).unwrap_or("/System/bin:/System")
    } else {
        "/System/bin:/System"
    };

    let mut stat_buf = [0u32; 7];

    for dir in path_str.split(':') {
        let dir = dir.trim();
        if dir.is_empty() {
            continue;
        }
        // Build candidate path
        let mut buf = [0u8; 256];
        let dlen = dir.len();
        if dlen + 1 + cmd.len() > buf.len() {
            continue;
        }
        buf[..dlen].copy_from_slice(dir.as_bytes());
        buf[dlen] = b'/';
        buf[dlen + 1..dlen + 1 + cmd.len()].copy_from_slice(cmd.as_bytes());
        let full = core::str::from_utf8(&buf[..dlen + 1 + cmd.len()]).unwrap_or("");

        if anyos_std::fs::stat(full, &mut stat_buf) == 0 && stat_buf[0] == 0 {
            anyos_std::println!("{}", full);
            return;
        }
    }

    anyos_std::println!("{}: not found", cmd);
}
