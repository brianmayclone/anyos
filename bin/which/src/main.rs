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

    // Check /bin/<cmd>
    let mut path = [0u8; 128];
    let prefix = b"/bin/";
    path[..prefix.len()].copy_from_slice(prefix);
    let clen = cmd.len().min(128 - prefix.len());
    path[prefix.len()..prefix.len() + clen].copy_from_slice(&cmd.as_bytes()[..clen]);
    let full = core::str::from_utf8(&path[..prefix.len() + clen]).unwrap_or("");

    let mut stat_buf = [0u32; 2];
    if anyos_std::fs::stat(full, &mut stat_buf) == 0 && stat_buf[0] == 0 {
        anyos_std::println!("{}", full);
        return;
    }

    // Check /System/<cmd>
    let prefix2 = b"/System/";
    let mut path2 = [0u8; 128];
    path2[..prefix2.len()].copy_from_slice(prefix2);
    let clen2 = cmd.len().min(128 - prefix2.len());
    path2[prefix2.len()..prefix2.len() + clen2].copy_from_slice(&cmd.as_bytes()[..clen2]);
    let full2 = core::str::from_utf8(&path2[..prefix2.len() + clen2]).unwrap_or("");

    if anyos_std::fs::stat(full2, &mut stat_buf) == 0 && stat_buf[0] == 0 {
        anyos_std::println!("{}", full2);
        return;
    }

    anyos_std::println!("{}: not found", cmd);
}
