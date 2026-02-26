#![no_std]
#![no_main]

anyos_std::entry!(main);

use anyos_std::{net, sys, process, println};

// ── TCP state names ─────────────────────────────────────────────────

fn state_name(s: u8) -> &'static str {
    match s {
        0 => "CLOSED",
        1 => "SYN_SENT",
        2 => "ESTABLISHED",
        3 => "FIN_WAIT1",
        4 => "FIN_WAIT2",
        5 => "TIME_WAIT",
        6 => "CLOSE_WAIT",
        7 => "LAST_ACK",
        8 => "LISTEN",
        9 => "SYN_RECV",
        _ => "UNKNOWN",
    }
}

// ── Flag parsing ────────────────────────────────────────────────────

struct Flags {
    tcp: bool,        // -t
    udp: bool,        // -u
    listen: bool,     // -l
    progs: bool,      // -p
    extend: bool,     // -e
    numeric: bool,    // -n
    all: bool,        // -a
    route: bool,      // -r
    iface: bool,      // -i
    stats: bool,      // -s
    continuous: bool,  // -c
    wide: bool,       // -W
    help: bool,
}

fn parse_flags(raw: &str) -> Flags {
    let mut f = Flags {
        tcp: false, udp: false, listen: false,
        progs: false, extend: false, numeric: false, all: false,
        route: false, iface: false, stats: false, continuous: false,
        wide: false, help: false,
    };

    for arg in raw.split_ascii_whitespace() {
        if arg == "--help" || arg == "-h" {
            f.help = true;
        } else if arg.starts_with("--") {
            match arg {
                "--tcp" => f.tcp = true,
                "--udp" => f.udp = true,
                "--listening" => f.listen = true,
                "--program" | "--programs" => f.progs = true,
                "--extend" => f.extend = true,
                "--numeric" => f.numeric = true,
                "--all" => f.all = true,
                "--route" => f.route = true,
                "--interfaces" => f.iface = true,
                "--statistics" => f.stats = true,
                "--continuous" => f.continuous = true,
                "--wide" => f.wide = true,
                _ => {}
            }
        } else if arg.starts_with('-') {
            for ch in arg[1..].chars() {
                match ch {
                    't' => f.tcp = true,
                    'u' => f.udp = true,
                    'l' => f.listen = true,
                    'p' => f.progs = true,
                    'e' => f.extend = true,
                    'n' => f.numeric = true,
                    'a' => f.all = true,
                    'r' => f.route = true,
                    'i' => f.iface = true,
                    's' => f.stats = true,
                    'c' => f.continuous = true,
                    'W' => f.wide = true,
                    'h' => f.help = true,
                    _ => {}
                }
            }
        }
    }

    // Default: if neither -t nor -u specified (and not -r/-i/-s), show both
    if !f.tcp && !f.udp && !f.route && !f.iface && !f.stats {
        f.tcp = true;
        f.udp = true;
    }

    f
}

fn print_usage() {
    println!("Usage: netstat [options]");
    println!("");
    println!("Options:");
    println!("  -t, --tcp          Show TCP sockets");
    println!("  -u, --udp          Show UDP sockets");
    println!("  -l, --listening    Show only listening sockets");
    println!("  -a, --all          Show all sockets (listening and non-listening)");
    println!("  -p, --program      Show PID/program name");
    println!("  -e, --extend       Show extended info (user)");
    println!("  -n, --numeric      Show numerical addresses (no DNS)");
    println!("  -r, --route        Show routing table");
    println!("  -i, --interfaces   Show interface table");
    println!("  -s, --statistics   Show protocol statistics");
    println!("  -c, --continuous   Continuous mode (repeat every 2s)");
    println!("  -W, --wide         Don't truncate addresses");
    println!("  -h, --help         Show this help");
    println!("");
    println!("Examples:");
    println!("  netstat -tulpen    Show all TCP/UDP listeners with program and user info");
    println!("  netstat -tan       Show all TCP connections (numeric)");
    println!("  netstat -r         Show routing table");
    println!("  netstat -i         Show interface statistics");
    println!("  netstat -s         Show protocol statistics");
    println!("  netstat -c -tl     Continuously show TCP listeners");
}

// ── Thread info cache for -p flag ───────────────────────────────────

struct ThreadInfo {
    tid: u32,
    name: [u8; 24],
    name_len: u8,
    uid: u16,
}

struct ThreadCache {
    entries: [ThreadInfo; 64],
    count: usize,
}

impl ThreadCache {
    fn new() -> Self {
        ThreadCache {
            entries: core::array::from_fn(|_| ThreadInfo {
                tid: 0, name: [0; 24], name_len: 0, uid: 0,
            }),
            count: 0,
        }
    }

    fn load(&mut self) {
        let mut buf = [0u8; 60 * 64];
        let count = sys::sysinfo(1, &mut buf);
        if count == u32::MAX { return; }
        self.count = (count as usize).min(64);
        for i in 0..self.count {
            let off = i * 60;
            if off + 60 > buf.len() { break; }
            self.entries[i].tid = u32::from_le_bytes([
                buf[off], buf[off+1], buf[off+2], buf[off+3]
            ]);
            self.entries[i].name[..24].copy_from_slice(&buf[off+8..off+32]);
            let nlen = self.entries[i].name.iter()
                .position(|&b| b == 0).unwrap_or(24);
            self.entries[i].name_len = nlen as u8;
            self.entries[i].uid = u16::from_le_bytes([buf[off+56], buf[off+57]]);
        }
    }

    fn find(&self, tid: u32) -> Option<&ThreadInfo> {
        self.entries[..self.count].iter().find(|e| e.tid == tid)
    }

    fn name_of(&self, tid: u32) -> &str {
        match self.find(tid) {
            Some(info) => {
                core::str::from_utf8(&info.name[..info.name_len as usize])
                    .unwrap_or("-")
            }
            None => "-",
        }
    }

    fn uid_of(&self, tid: u32) -> u16 {
        match self.find(tid) {
            Some(info) => info.uid,
            None => 0,
        }
    }
}

// ── Username cache for -e flag ──────────────────────────────────────

struct UserCache {
    entries: [(u16, [u8; 16], u8); 16],
    count: usize,
}

impl UserCache {
    fn new() -> Self {
        UserCache {
            entries: [(0, [0u8; 16], 0); 16],
            count: 0,
        }
    }

    fn resolve(&mut self, uid: u16) -> &str {
        for i in 0..self.count {
            if self.entries[i].0 == uid {
                let len = self.entries[i].2 as usize;
                return core::str::from_utf8(&self.entries[i].1[..len]).unwrap_or("?");
            }
        }
        if self.count < 16 {
            let mut name_buf = [0u8; 16];
            let nlen = process::getusername(uid, &mut name_buf);
            let len = if nlen != u32::MAX && nlen > 0 { (nlen as u8).min(15) } else { 0 };
            self.entries[self.count] = (uid, name_buf, len);
            self.count += 1;
            let len = len as usize;
            return core::str::from_utf8(&self.entries[self.count - 1].1[..len]).unwrap_or("?");
        }
        "?"
    }
}

// ── Format helpers ──────────────────────────────────────────────────

fn fmt_ip(ip: &[u8]) -> alloc::string::String {
    alloc::format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3])
}

fn fmt_ip_port(ip: &[u8; 4], port: u16) -> alloc::string::String {
    alloc::format!("{}.{}.{}.{}:{}", ip[0], ip[1], ip[2], ip[3], port)
}

fn fmt_mac(mac: &[u8]) -> alloc::string::String {
    alloc::format!("{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5])
}

fn fmt_bytes(bytes: u64) -> alloc::string::String {
    if bytes >= 1_073_741_824 {
        let whole = bytes / 1_073_741_824;
        let frac = (bytes % 1_073_741_824) * 10 / 1_073_741_824;
        alloc::format!("{}.{} GB", whole, frac)
    } else if bytes >= 1_048_576 {
        let whole = bytes / 1_048_576;
        let frac = (bytes % 1_048_576) * 10 / 1_048_576;
        alloc::format!("{}.{} MB", whole, frac)
    } else if bytes >= 1024 {
        let whole = bytes / 1024;
        let frac = (bytes % 1024) * 10 / 1024;
        alloc::format!("{}.{} KB", whole, frac)
    } else {
        alloc::format!("{} B", bytes)
    }
}

// ── Routing table (-r) ──────────────────────────────────────────────

fn show_routing() {
    let mut cfg = [0u8; 24];
    if net::get_config(&mut cfg) != 0 {
        println!("Failed to get network configuration.");
        return;
    }

    let ip = &cfg[0..4];
    let mask = &cfg[4..8];
    let gw = &cfg[8..12];
    let link = cfg[22];

    if link == 0 {
        println!("Network is down.");
        return;
    }

    println!("Kernel IP routing table");
    println!("{:<16} {:<16} {:<16} {:<6} {:<6} {}",
        "Destination", "Gateway", "Genmask", "Flags", "Metric", "Iface");

    // Local subnet route
    let net_ip: [u8; 4] = [
        ip[0] & mask[0], ip[1] & mask[1],
        ip[2] & mask[2], ip[3] & mask[3],
    ];
    println!("{:<16} {:<16} {:<16} {:<6} {:<6} {}",
        fmt_ip(&net_ip), "0.0.0.0", fmt_ip(mask), "U", "0", "eth0");

    // Default gateway
    if gw[0] != 0 || gw[1] != 0 || gw[2] != 0 || gw[3] != 0 {
        println!("{:<16} {:<16} {:<16} {:<6} {:<6} {}",
            "0.0.0.0", fmt_ip(gw), "0.0.0.0", "UG", "100", "eth0");
    }

    // ARP neighbors
    let mut arp_buf = [0u8; 12 * 64];
    let arp_count = net::arp(&mut arp_buf);
    if arp_count > 0 {
        println!("");
        println!("ARP cache ({} entries):", arp_count);
        println!("{:<16} {:<20} {}", "Address", "HWaddress", "Iface");
        for i in 0..arp_count as usize {
            let off = i * 12;
            let aip = &arp_buf[off..off+4];
            let amac = &arp_buf[off+4..off+10];
            println!("{:<16} {:<20} {}",
                fmt_ip(aip), fmt_mac(amac), "eth0");
        }
    }
}

// ── Interface table (-i) ────────────────────────────────────────────

fn show_interfaces() {
    let mut cfg = [0u8; 24];
    if net::get_config(&mut cfg) != 0 {
        println!("Failed to get network configuration.");
        return;
    }

    let ip = &cfg[0..4];
    let mask = &cfg[4..8];
    let gw = &cfg[8..12];
    let dns = &cfg[12..16];
    let mac = &cfg[16..22];
    let link = cfg[22];

    println!("Kernel Interface table");
    println!("{:<8} {:<6} {:<16} {:<10} {:<10} {:<10} {:<10} {:<10} {:<10}",
        "Iface", "MTU", "RX-OK", "RX-ERR", "RX-OVR", "TX-OK", "TX-ERR", "TX-OVR", "Flg");

    let flags = if link != 0 { "BMRU" } else { "BMR" };

    // Get NIC stats
    let (rx_ok, tx_ok, _rxb, _txb, rx_err, tx_err) = if let Some(s) = net::net_stats() {
        (s.rx_packets, s.tx_packets, s.rx_bytes, s.tx_bytes, s.rx_errors, s.tx_errors)
    } else {
        (0, 0, 0, 0, 0, 0)
    };

    println!("{:<8} {:<6} {:<16} {:<10} {:<10} {:<10} {:<10} {:<10} {:<10}",
        "eth0", "1500", rx_ok, rx_err, "0", tx_ok, tx_err, "0", flags);

    // Loopback
    println!("{:<8} {:<6} {:<16} {:<10} {:<10} {:<10} {:<10} {:<10} {:<10}",
        "lo", "65536", "0", "0", "0", "0", "0", "0", "LRU");

    println!("");
    println!("eth0:");
    println!("  Link     : {}", if link != 0 { "UP" } else { "DOWN" });
    println!("  MAC      : {}", fmt_mac(mac));
    println!("  IPv4     : {}/{}", fmt_ip(ip), fmt_ip(mask));
    println!("  Gateway  : {}", fmt_ip(gw));
    println!("  DNS      : {}", fmt_ip(dns));

    if let Some(s) = net::net_stats() {
        println!("  RX packets : {}  bytes : {} ({})", s.rx_packets, s.rx_bytes, fmt_bytes(s.rx_bytes));
        println!("  TX packets : {}  bytes : {} ({})", s.tx_packets, s.tx_bytes, fmt_bytes(s.tx_bytes));
        if s.rx_errors > 0 || s.tx_errors > 0 {
            println!("  RX errors  : {}  TX errors : {}", s.rx_errors, s.tx_errors);
        }
    }
}

// ── Statistics (-s) ─────────────────────────────────────────────────

fn show_statistics(show_tcp: bool, show_udp: bool) {
    let stats = match net::net_stats() {
        Some(s) => s,
        None => {
            println!("Failed to get network statistics.");
            return;
        }
    };

    // IP statistics (from NIC counters)
    println!("Ip:");
    println!("    {} total packets received", stats.rx_packets);
    println!("    {} packets sent", stats.tx_packets);
    println!("    {} incoming bytes ({})", stats.rx_bytes, fmt_bytes(stats.rx_bytes));
    println!("    {} outgoing bytes ({})", stats.tx_bytes, fmt_bytes(stats.tx_bytes));
    if stats.rx_errors > 0 {
        println!("    {} incoming errors", stats.rx_errors);
    }
    if stats.tx_errors > 0 {
        println!("    {} outgoing errors", stats.tx_errors);
    }

    if show_tcp {
        println!("Tcp:");
        println!("    {} active connection openings", stats.tcp_active_opens);
        println!("    {} passive connection openings", stats.tcp_passive_opens);
        println!("    {} current established connections", stats.tcp_curr_established);
        println!("    {} segments sent", stats.tcp_segments_sent);
        println!("    {} segments received", stats.tcp_segments_recv);
        println!("    {} segments retransmitted", stats.tcp_retransmits);
        if stats.tcp_resets_sent > 0 {
            println!("    {} resets sent", stats.tcp_resets_sent);
        }
        if stats.tcp_conn_errors > 0 {
            println!("    {} connection errors", stats.tcp_conn_errors);
        }
    }

    if show_udp {
        // UDP stats from bound port list
        let bindings = net::udp_list();
        let total_queued: u32 = bindings.iter().map(|b| b.recv_queue_len as u32).sum();
        println!("Udp:");
        println!("    {} ports bound", bindings.len());
        println!("    {} datagrams queued", total_queued);
    }
}

// ── Socket listing ──────────────────────────────────────────────────

fn show_sockets(flags: &Flags) {
    let mut tcache = ThreadCache::new();
    let mut ucache = UserCache::new();
    if flags.progs || flags.extend {
        tcache.load();
    }

    // Column widths
    let addr_w: usize = if flags.wide { 40 } else { 24 };

    // Build header
    let mut header = alloc::format!(
        "Proto  {:<aw$} {:<aw$} {:<11}",
        "Local Address", "Foreign Address", "State",
        aw = addr_w
    );
    if flags.progs {
        header.push_str(" PID/Program         ");
    }
    if flags.extend {
        header.push_str(" User       ");
    }
    header.push_str(" RecvQ");

    let sep_len = header.len();
    let sep: alloc::string::String = (0..sep_len).map(|_| '-').collect();

    let mut printed_header = false;
    let mut total = 0u32;

    // ── TCP ─────────────────────────────────────────────────────
    if flags.tcp {
        let conns = net::tcp_list();
        for c in &conns {
            let is_listen = c.state == 8;
            if flags.listen && !is_listen { continue; }
            if !flags.listen && !flags.all && is_listen { continue; }

            if !printed_header {
                println!("{}", header);
                println!("{}", sep);
                printed_header = true;
            }

            let local = fmt_ip_port(&c.local_ip, c.local_port);
            let remote = if is_listen {
                alloc::string::String::from("0.0.0.0:*")
            } else {
                fmt_ip_port(&c.remote_ip, c.remote_port)
            };

            let mut line = alloc::format!(
                "tcp    {:<aw$} {:<aw$} {:<11}",
                local, remote, state_name(c.state),
                aw = addr_w
            );

            if flags.progs {
                let tid = c.owner_tid as u32;
                let name = tcache.name_of(tid);
                let prog = alloc::format!("{}/{}", tid, name);
                line.push_str(&alloc::format!(" {:<20}", prog));
            }

            if flags.extend {
                let uid = tcache.uid_of(c.owner_tid as u32);
                let user = ucache.resolve(uid);
                line.push_str(&alloc::format!(" {:<11}", user));
            }

            line.push_str(&alloc::format!(" {:>5}", c.recv_buf_len));
            println!("{}", line);
            total += 1;
        }
    }

    // ── UDP ─────────────────────────────────────────────────────
    if flags.udp {
        let bindings = net::udp_list();
        for b in &bindings {
            if !flags.listen && !flags.all { continue; }

            if !printed_header {
                println!("{}", header);
                println!("{}", sep);
                printed_header = true;
            }

            let local = alloc::format!("0.0.0.0:{}", b.port);
            let remote = alloc::string::String::from("0.0.0.0:*");

            let mut line = alloc::format!(
                "udp    {:<aw$} {:<aw$} {:<11}",
                local, remote, "",
                aw = addr_w
            );

            if flags.progs {
                let tid = b.owner_tid as u32;
                let name = tcache.name_of(tid);
                let prog = alloc::format!("{}/{}", tid, name);
                line.push_str(&alloc::format!(" {:<20}", prog));
            }

            if flags.extend {
                let uid = tcache.uid_of(b.owner_tid as u32);
                let user = ucache.resolve(uid);
                line.push_str(&alloc::format!(" {:<11}", user));
            }

            line.push_str(&alloc::format!(" {:>5}", b.recv_queue_len));
            println!("{}", line);
            total += 1;
        }
    }

    if total == 0 {
        if flags.listen {
            println!("No listening sockets.");
        } else {
            println!("No active connections.");
        }
    }
}

// ── Main ────────────────────────────────────────────────────────────

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = process::args(&mut args_buf);
    let flags = parse_flags(raw);

    if flags.help {
        print_usage();
        return;
    }

    loop {
        // Dispatch to the correct display mode
        if flags.route {
            show_routing();
        } else if flags.iface {
            show_interfaces();
        } else if flags.stats {
            show_statistics(flags.tcp, flags.udp);
        } else {
            show_sockets(&flags);
        }

        if !flags.continuous { break; }

        println!("");
        // Sleep ~2 seconds
        process::sleep(2000);
    }
}
