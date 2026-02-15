#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let args = anyos_std::process::args(&mut args_buf);

    if args.is_empty() {
        // No args: list all environment variables
        let mut buf = [0u8; 4096];
        let total = anyos_std::env::list(&mut buf);
        if total == 0 {
            anyos_std::println!("No environment variables set.");
            return;
        }
        // Parse "KEY=VALUE\0KEY2=VALUE2\0..." format
        let data = &buf[..(total as usize).min(buf.len())];
        let mut i = 0;
        while i < data.len() {
            let end = data[i..].iter().position(|&b| b == 0).map(|p| i + p).unwrap_or(data.len());
            if end > i {
                let entry = core::str::from_utf8(&data[i..end]).unwrap_or("???");
                anyos_std::println!("{}", entry);
            }
            i = end + 1;
        }
        return;
    }

    // Check for KEY=VALUE format (set)
    if let Some(eq_pos) = args.find('=') {
        let key = &args[..eq_pos];
        let value = &args[eq_pos + 1..];
        if key.is_empty() {
            anyos_std::println!("Usage: env [KEY=VALUE | KEY | -u KEY]");
            return;
        }
        anyos_std::env::set(key, value);
    } else if args.starts_with("-u ") || args.starts_with("-u\t") {
        // Unset: env -u KEY
        let key = args[3..].trim();
        if key.is_empty() {
            anyos_std::println!("Usage: env -u KEY");
            return;
        }
        anyos_std::env::unset(key);
    } else {
        // Get: env KEY
        let key = args.trim();
        let mut val_buf = [0u8; 256];
        let len = anyos_std::env::get(key, &mut val_buf);
        if len == u32::MAX {
            anyos_std::println!("env: '{}' not set", key);
        } else {
            let vlen = (len as usize).min(255);
            let val = core::str::from_utf8(&val_buf[..vlen]).unwrap_or("");
            anyos_std::println!("{}", val);
        }
    }
}
