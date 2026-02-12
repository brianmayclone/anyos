#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let args = anyos_std::process::args(&mut args_buf);
    let args = args.trim();

    if args.is_empty() {
        // No args: list all environment variables
        let mut buf = [0u8; 4096];
        let total = anyos_std::env::list(&mut buf);
        let len = (total as usize).min(buf.len());
        let mut offset = 0;
        while offset < len {
            let end = buf[offset..len].iter().position(|&b| b == 0).unwrap_or(len - offset);
            if end == 0 { break; }
            if let Ok(entry) = core::str::from_utf8(&buf[offset..offset + end]) {
                anyos_std::println!("{}", entry);
            }
            offset += end + 1;
        }
        return;
    }

    // Parse KEY=VALUE
    if let Some(eq_pos) = args.find('=') {
        let key = &args[..eq_pos];
        let value = &args[eq_pos + 1..];
        if key.is_empty() {
            anyos_std::println!("set: invalid variable name");
            return;
        }
        anyos_std::env::set(key, value);
    } else {
        // Just a key with no value — show its value
        let mut buf = [0u8; 256];
        let len = anyos_std::env::get(args, &mut buf);
        if len != u32::MAX {
            let val = core::str::from_utf8(&buf[..len as usize]).unwrap_or("");
            anyos_std::println!("{}={}", args, val);
        } else {
            anyos_std::println!("set: '{}' not set", args);
        }
    }
}
