#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let args = anyos_std::process::args(&mut args_buf);

    if args.is_empty() {
        anyos_std::println!("Usage: dns <hostname>");
        anyos_std::println!("  Example: dns google.com");
        return;
    }

    let hostname = args.trim();
    anyos_std::println!("Resolving '{}'...", hostname);

    let mut result = [0u8; 4];
    let ret = anyos_std::net::dns(hostname, &mut result);

    if ret != 0 {
        anyos_std::println!("DNS: Failed to resolve '{}' (error {})", hostname, ret);
        return;
    }

    anyos_std::println!(
        "{} -> {}.{}.{}.{}",
        hostname, result[0], result[1], result[2], result[3]
    );
}
