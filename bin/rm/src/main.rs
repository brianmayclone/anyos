#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let args = anyos_std::process::args(&mut args_buf);
    let path = args.trim();

    if path.is_empty() {
        anyos_std::println!("Usage: rm <path>");
        return;
    }

    if anyos_std::fs::unlink(path) == u32::MAX {
        anyos_std::println!("rm: cannot remove '{}': No such file", path);
    }
}
