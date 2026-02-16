#![no_std]
#![no_main]

anyos_std::entry!(main);

fn mkdir_parents(path: &str) {
    // Create each component: /a -> /a/b -> /a/b/c
    let bytes = path.as_bytes();
    let mut i = 0;
    if !bytes.is_empty() && bytes[0] == b'/' { i = 1; }
    while i <= bytes.len() {
        if i == bytes.len() || bytes[i] == b'/' {
            if i > 0 {
                let component = core::str::from_utf8(&bytes[..i]).unwrap_or("");
                // Ignore errors — parent may already exist
                anyos_std::fs::mkdir(component);
            }
        }
        i += 1;
    }
}

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"");

    let parents = args.has(b'p');

    if args.pos_count == 0 {
        anyos_std::println!("Usage: mkdir [-p] DIR...");
        return;
    }

    for i in 0..args.pos_count {
        let path = args.positional[i];
        if parents {
            mkdir_parents(path);
        } else {
            if anyos_std::fs::mkdir(path) == u32::MAX {
                anyos_std::println!("mkdir: cannot create directory '{}': File exists or error", path);
            }
        }
    }
}
