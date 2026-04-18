#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let args = anyos_std::process::args(&mut args_buf);

    if args.contains("--help") {
        anyos_std::println!("dns - Resolve hostname to IP or manage dnsd\n\nUsage: dns HOSTNAME\n       dns --flush\n       dns --reload\n       dns --stats");
        return;
    }

    if args == "--flush" {
        if !libdns::flush_cache() {
            anyos_std::println!("DNS: Failed to flush cache");
            return;
        }
        anyos_std::println!("DNS cache flushed.");
        return;
    }

    if args == "--reload" {
        if !libdns::reload() {
            anyos_std::println!("DNS: Failed to reload dnsd");
            return;
        }
        anyos_std::println!("dnsd reloaded.");
        return;
    }

    if args == "--stats" {
        if let Some(stats) = libdns::stats() {
            anyos_std::println!("{}", stats.trim_end());
        } else {
            anyos_std::println!("DNS: Failed to query dnsd stats");
        }
        return;
    }

    if args.is_empty() {
        anyos_std::println!("Usage: dns <hostname>");
        anyos_std::println!("       dns --flush");
        anyos_std::println!("       dns --reload");
        anyos_std::println!("       dns --stats");
        anyos_std::println!("  Example: dns google.com");
        return;
    }

    let hostname = args.trim();
    anyos_std::println!("Resolving '{}'...", hostname);

    if let Some(result) = libdns::resolve_ipv4(hostname) {
        anyos_std::println!(
            "{} -> {}.{}.{}.{}",
            hostname, result[0], result[1], result[2], result[3]
        );
    } else {
        anyos_std::println!("DNS: Failed to resolve '{}'", hostname);
    }
}
