#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"sf");

    let symbolic = args.has(b's');
    let force = args.has(b'f');

    if args.pos_count < 2 {
        anyos_std::println!("Usage: ln [-sf] TARGET LINK_NAME");
        anyos_std::println!("  -s  create symbolic link");
        anyos_std::println!("  -f  remove existing destination files");
        return;
    }

    let target = args.positional[0];
    let link_name = args.positional[1];

    if !symbolic {
        anyos_std::println!("ln: hard links are not supported, use -s for symbolic links");
        return;
    }

    // If force flag, remove existing file
    if force {
        let mut stat_buf = [0u32; 6];
        if anyos_std::fs::lstat(link_name, &mut stat_buf) == 0 {
            if anyos_std::fs::unlink(link_name) != 0 {
                anyos_std::println!("ln: cannot remove '{}': Permission denied", link_name);
                return;
            }
        }
    }

    let ret = anyos_std::fs::symlink(target, link_name);
    if ret != 0 {
        anyos_std::println!("ln: failed to create symbolic link '{}' -> '{}'", link_name, target);
    }
}
