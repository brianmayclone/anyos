#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut buf = [0u8; 256];
    let args = anyos_std::process::args(&mut buf);

    // Expand $VAR references in the arguments
    let bytes = args.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' && i + 1 < bytes.len() {
            // Extract variable name (alphanumeric + underscore)
            let start = i + 1;
            let mut end = start;
            while end < bytes.len()
                && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_')
            {
                end += 1;
            }
            if end > start {
                let var_name = core::str::from_utf8(&bytes[start..end]).unwrap_or("");
                let mut val_buf = [0u8; 256];
                let val_len = anyos_std::env::get(var_name, &mut val_buf);
                if val_len != u32::MAX && val_len > 0 {
                    let vlen = (val_len as usize).min(255);
                    let val = core::str::from_utf8(&val_buf[..vlen]).unwrap_or("");
                    anyos_std::print!("{}", val);
                }
                // else: undefined variable → print nothing (like bash)
                i = end;
            } else {
                anyos_std::print!("$");
                i += 1;
            }
        } else {
            anyos_std::print!("{}", bytes[i] as char);
            i += 1;
        }
    }
    anyos_std::println!();
}
