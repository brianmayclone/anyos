#![no_std]
#![no_main]

anyos_std::entry!(main);

use anyos_std::{net, process, println};

struct Options {
    show_tcp: bool,
    show_udp: bool,
    show_dns: bool,
    show_arp: bool,
    show_rx: bool,
    show_tx: bool,
    clear: bool,
    disable: bool,
    count: u32,
}

fn main() {
    let mut args_buf = [0u8; 256];
    let args = process::args(&mut args_buf);
    let opts = parse_args(args);

    if args.contains("--help") || args.contains("-h") {
        print_usage();
        return;
    }

    if opts.clear {
        net::net_trace_clear();
    }
    if opts.disable {
        net::net_trace_disable();
        println!("dcpdump: trace disabled");
        return;
    }

    net::net_trace_enable();
    println!("dcpdump: tracing enabled");

    let mut printed = 0u32;
    let mut raw = [0u8; net::NET_TRACE_ENTRY_SIZE * 64];
    loop {
        let count = net::net_trace_read(&mut raw);
        for i in 0..count as usize {
            let off = i * net::NET_TRACE_ENTRY_SIZE;
            if let Some(entry) = net::NetTraceEntry::from_bytes(&raw[off..off + net::NET_TRACE_ENTRY_SIZE]) {
                if !matches_filters(&entry, &opts) {
                    continue;
                }
                print_entry(&entry);
                printed = printed.wrapping_add(1);
                if opts.count != 0 && printed >= opts.count {
                    return;
                }
            }
        }
        process::sleep(200);
    }
}

fn parse_args(args: &str) -> Options {
    let mut opts = Options {
        show_tcp: false,
        show_udp: false,
        show_dns: false,
        show_arp: false,
        show_rx: true,
        show_tx: true,
        clear: false,
        disable: false,
        count: 0,
    };
    let mut want_count = false;

    for arg in args.split_ascii_whitespace() {
        if want_count {
            opts.count = parse_u32(arg).unwrap_or(0);
            want_count = false;
            continue;
        }
        match arg {
            "--tcp" => opts.show_tcp = true,
            "--udp" => opts.show_udp = true,
            "--dns" => opts.show_dns = true,
            "--arp" => opts.show_arp = true,
            "--rx" => opts.show_tx = false,
            "--tx" => opts.show_rx = false,
            "--clear" => opts.clear = true,
            "--disable" => opts.disable = true,
            "-c" | "--count" => want_count = true,
            _ => {}
        }
    }

    opts
}

fn matches_filters(entry: &net::NetTraceEntry, opts: &Options) -> bool {
    if entry.direction == net::NET_TRACE_DIR_RX && !opts.show_rx {
        return false;
    }
    if entry.direction == net::NET_TRACE_DIR_TX && !opts.show_tx {
        return false;
    }

    let protocol_filtered = opts.show_tcp || opts.show_udp || opts.show_dns || opts.show_arp;
    if !protocol_filtered {
        return true;
    }

    if opts.show_arp && entry.ethertype == 0x0806 {
        return true;
    }
    if opts.show_dns && entry.is_dns() {
        return true;
    }
    if opts.show_tcp && entry.protocol == 6 {
        return true;
    }
    if opts.show_udp && entry.protocol == 17 && !entry.is_dns() {
        return true;
    }

    false
}

fn print_entry(entry: &net::NetTraceEntry) {
    let dir = if entry.direction == net::NET_TRACE_DIR_TX { "TX" } else { "RX" };
    if entry.ethertype == 0x0806 {
        println!("[{}] {} ARP len={}", entry.timestamp_ms, dir, entry.length);
        return;
    }

    let proto = match entry.protocol {
        6 => "TCP",
        17 => {
            if entry.is_dns() { "DNS" } else { "UDP" }
        }
        1 => "ICMP",
        _ => "IP",
    };

    println!(
        "[{}] {} {} {}.{}.{}.{}:{} -> {}.{}.{}.{}:{} len={}",
        entry.timestamp_ms,
        dir,
        proto,
        entry.src_ip[0], entry.src_ip[1], entry.src_ip[2], entry.src_ip[3], entry.src_port,
        entry.dst_ip[0], entry.dst_ip[1], entry.dst_ip[2], entry.dst_ip[3], entry.dst_port,
        entry.length
    );
}

fn parse_u32(s: &str) -> Option<u32> {
    let mut value = 0u32;
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(value)
}

fn print_usage() {
    println!("dcpdump - lightweight packet dump");
    println!("");
    println!("Usage: dcpdump [--tcp] [--udp] [--dns] [--arp] [--rx|--tx] [-c N]");
    println!("       dcpdump --clear");
    println!("       dcpdump --disable");
}
