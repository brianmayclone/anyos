#![no_std]
#![no_main]

anyos_std::entry!(main);

use anyos_std::{net, process, sys};
use anyos_std::format;

fn main() -> u32 {
    let mut args_buf = [0u8; 256];
    let raw = process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"");

    if !net::wifi_available() {
        sys::con_write("wifi: no WiFi hardware detected\n");
        return 1;
    }

    if args.pos_count == 0 || raw.contains("--help") {
        print_usage();
        return if raw.contains("--help") { 0 } else { 1 };
    }

    match args.positional[0] {
        "scan" => cmd_scan(),
        "status" => cmd_status(),
        "connect" => cmd_connect(&args),
        "disconnect" => cmd_disconnect(),
        _ => {
            sys::con_write("wifi: unknown command\n");
            print_usage();
            1
        }
    }
}

fn print_usage() {
    sys::con_write("usage: wifi <command> [args]\n");
    sys::con_write("\n");
    sys::con_write("commands:\n");
    sys::con_write("  scan              Scan for available networks\n");
    sys::con_write("  status            Show current connection status\n");
    sys::con_write("  connect <ssid> [password]  Connect to a network\n");
    sys::con_write("  disconnect        Disconnect from current network\n");
}

fn cmd_scan() -> u32 {
    sys::con_write("Scanning for networks...\n");
    net::wifi_scan();

    // Wait ~2 seconds for scan to complete (driver dwells per channel)
    process::sleep(2000);

    let results = net::wifi_scan_results(64);
    if results.is_empty() {
        sys::con_write("No networks found.\n");
        return 0;
    }

    sys::con_write("BSSID              SSID                             Ch  RSSI  Security\n");
    sys::con_write("─────────────────────────────────────────────────────────────────────\n");
    for bss in &results {
        let ssid_str = core::str::from_utf8(&bss.ssid[..bss.ssid_len]).unwrap_or("(invalid)");
        let security = match bss.security {
            net::WifiSecurity::Open => "Open",
            net::WifiSecurity::Wpa2Personal => "WPA2",
        };
        let line = format!(
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}  {:<32} {:>3}  {:>4}  {}\n",
            bss.bssid[0], bss.bssid[1], bss.bssid[2],
            bss.bssid[3], bss.bssid[4], bss.bssid[5],
            ssid_str,
            bss.channel,
            bss.rssi,
            security,
        );
        sys::con_write(&line);
    }
    0
}

fn cmd_status() -> u32 {
    let status = net::wifi_status();
    let state_str = match status.state {
        net::WifiState::Disconnected   => "Disconnected",
        net::WifiState::Scanning       => "Scanning",
        net::WifiState::Associating    => "Associating",
        net::WifiState::Authenticating => "Authenticating (WPA2 handshake)",
        net::WifiState::Connected      => "Connected",
    };
    let line = format!("State: {}\n", state_str);
    sys::con_write(&line);

    if status.connected {
        let ssid_str = core::str::from_utf8(&status.ssid[..status.ssid_len]).unwrap_or("(invalid)");
        let ssid_line = format!("SSID:  {}\n", ssid_str);
        let bssid_line = format!(
            "BSSID: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}\n",
            status.bssid[0], status.bssid[1], status.bssid[2],
            status.bssid[3], status.bssid[4], status.bssid[5],
        );
        let ch_line = format!("Ch:    {}\n", status.channel);
        sys::con_write(&ssid_line);
        sys::con_write(&bssid_line);
        sys::con_write(&ch_line);
    }
    0
}

fn cmd_connect(args: &anyos_std::args::ParsedArgs) -> u32 {
    if args.pos_count < 2 {
        sys::con_write("wifi connect: missing SSID\n");
        sys::con_write("usage: wifi connect <ssid> [password]\n");
        return 1;
    }
    let ssid = args.positional[1];
    let password = if args.pos_count >= 3 { args.positional[2] } else { "" };

    let ssid_line = format!("Connecting to \"{}\"...\n", ssid);
    sys::con_write(&ssid_line);

    let rc = net::wifi_connect(ssid, password);
    if rc == u32::MAX {
        sys::con_write("wifi connect: invalid parameters (SSID >32 or password >64 bytes)\n");
        return 1;
    }

    // Poll state until connected or failed (max 30 seconds)
    let mut ticks = 0u32;
    loop {
        process::sleep(500);
        ticks += 500;

        let state = net::wifi_state();
        match state {
            net::WifiState::Connected => {
                sys::con_write("Connected.\n");
                return 0;
            }
            net::WifiState::Disconnected => {
                sys::con_write("wifi connect: connection failed.\n");
                return 1;
            }
            net::WifiState::Scanning => {
                if ticks % 2000 == 0 {
                    sys::con_write("  Scanning...\n");
                }
            }
            net::WifiState::Associating => {
                if ticks % 2000 == 0 {
                    sys::con_write("  Associating...\n");
                }
            }
            net::WifiState::Authenticating => {
                if ticks % 2000 == 0 {
                    sys::con_write("  WPA2 handshake...\n");
                }
            }
        }

        if ticks >= 30_000 {
            sys::con_write("wifi connect: timed out.\n");
            return 1;
        }
    }
}

fn cmd_disconnect() -> u32 {
    net::wifi_disconnect();
    sys::con_write("Disconnected.\n");
    0
}
