#![no_std]
#![no_main]

use alloc::format;
use alloc::string::String;

use anyos_std::{fs, ipc, net, println, process, sys};
use libconf_schema::{
    default_bool, default_int, default_string, manifest, RegistryScope, ServiceSchema,
};
use libsvc::ServiceLifecycle;

anyos_std::entry!(main);

const PIPE_NAME: &str = "networkd";
const STATUS_PATH: &str = "/System/etc/network/networkd.status";
const HOSTS_PATH: &str = "/System/etc/network/hosts";
const ENTRY_SIZE: usize = 128;

const NETWORKD_DIRS: &[&str] = &["config"];
const NETWORKD_DEFAULTS: &[libconf_schema::DefaultEntry<'static>] = &[
    default_int("config/poll_interval_ms", 2000),
    default_int("config/dhcp_retry_ms", 5000),
    default_bool("config/log_events", true),
    default_bool("config/reload_dnsd", true),
];
const NETWORKD_MIGRATIONS: &[libconf_schema::MigrationStep<'static>] = &[];
const NETWORKD_MANIFEST: libconf_schema::RegistryManifest<'static> = manifest(
    "services/networkd",
    RegistryScope::System,
    1,
    NETWORKD_DIRS,
    NETWORKD_DEFAULTS,
    NETWORKD_MIGRATIONS,
);
const NETWORKD_SCHEMA: ServiceSchema<'static> = ServiceSchema::new("networkd", &NETWORKD_MANIFEST);
const DNSD_HOSTS_DEFAULT: &str =
    "# anyOS hosts file\n# Format: <ip> <hostname> [aliases...]\n# Changes are picked up automatically.\n\n127.0.0.1   localhost\n";
const DNSD_HOSTS_DEFAULTS: &[libconf_schema::DefaultEntry<'static>] =
    &[default_string("config/hosts_blob", DNSD_HOSTS_DEFAULT)];
const DNSD_HOSTS_MIGRATIONS: &[libconf_schema::MigrationStep<'static>] = &[];
const DNSD_HOSTS_MANIFEST: libconf_schema::RegistryManifest<'static> = manifest(
    "services/dnsd",
    RegistryScope::System,
    1,
    &["config"],
    DNSD_HOSTS_DEFAULTS,
    DNSD_HOSTS_MIGRATIONS,
);
const DNSD_HOSTS_SCHEMA: ServiceSchema<'static> =
    ServiceSchema::new("networkd", &DNSD_HOSTS_MANIFEST);

struct NetworkdConfig {
    poll_interval_ms: u32,
    dhcp_retry_ms: u32,
    log_events: bool,
    reload_dnsd: bool,
}

impl Default for NetworkdConfig {
    fn default() -> Self {
        Self {
            poll_interval_ms: 2000,
            dhcp_retry_ms: 5000,
            log_events: true,
            reload_dnsd: true,
        }
    }
}

struct NetworkState {
    last_apply_ms: u32,
    last_link_up: bool,
    method: u8,
    ip: [u8; 4],
    mask: [u8; 4],
    gateway: [u8; 4],
    dns: [u8; 4],
    name: [u8; 16],
    name_len: u8,
    dhcp_failures: u32,
}

impl NetworkState {
    const fn new() -> Self {
        Self {
            last_apply_ms: 0,
            last_link_up: false,
            method: 0,
            ip: [0; 4],
            mask: [0; 4],
            gateway: [0; 4],
            dns: [0; 4],
            name: [0; 16],
            name_len: 0,
            dhcp_failures: 0,
        }
    }

    fn iface_name(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len as usize]).unwrap_or("?")
    }
}

fn main() {
    println!("networkd: starting");
    register_manifest();
    let mut lifecycle = ServiceLifecycle::connect("networkd").ok();
    if let Some(lifecycle) = lifecycle.as_mut() {
        let _ = lifecycle.notify_starting();
        let _ = lifecycle.set_health("starting");
    }

    let pipe_id = ipc::pipe_create(PIPE_NAME);
    if pipe_id == 0 || pipe_id == u32::MAX {
        println!("networkd: failed to create '{}' pipe", PIPE_NAME);
        if let Some(lifecycle) = lifecycle.as_mut() {
            let _ = lifecycle.notify_failed("pipe_create_failed");
        }
        return;
    }

    let mut cfg = load_config();
    let mut state = NetworkState::new();
    sync_hosts_projection();
    let _ = net::reload_hosts();
    // Announce readiness as soon as the IPC pipe is open — a DHCP failure must
    // not block `svc start-all` and therefore the entire boot chain.  A still-
    // configuring interface keeps trying in the background via periodic_check.
    let applied_ok = apply_interfaces(&cfg, &mut state, true);
    let mut ready_announced = true;
    if let Some(lifecycle) = lifecycle.as_mut() {
        let _ = lifecycle.notify_ready();
        let _ = lifecycle.set_health(if applied_ok { "configured" } else { "degraded" });
    }

    let mut pipe_buf = [0u8; 512];
    let mut last_poll = 0u32;
    println!(
        "networkd: ready (pipe='{}', applied_ok={})",
        PIPE_NAME, applied_ok
    );

    loop {
        if handle_requests(
            pipe_id,
            &mut cfg,
            &mut state,
            lifecycle.as_mut(),
            &mut ready_announced,
        ) {
            process::sleep(20);
            continue;
        }

        let now = sys::uptime_ms();
        if now.wrapping_sub(last_poll) >= cfg.poll_interval_ms {
            last_poll = now;
            periodic_check(&cfg, &mut state, lifecycle.as_mut(), &mut ready_announced);
        }
        let _ = &mut pipe_buf;
        process::sleep(100);
    }
}

fn load_config() -> NetworkdConfig {
    let mut cfg = NetworkdConfig::default();
    if let Some(v) = NETWORKD_SCHEMA.read_i64("config/poll_interval_ms") {
        if v > 0 {
            cfg.poll_interval_ms = v as u32;
        }
    }
    if let Some(v) = NETWORKD_SCHEMA.read_i64("config/dhcp_retry_ms") {
        if v > 0 {
            cfg.dhcp_retry_ms = v as u32;
        }
    }
    if let Some(v) = NETWORKD_SCHEMA.read_bool("config/log_events") {
        cfg.log_events = v;
    }
    if let Some(v) = NETWORKD_SCHEMA.read_bool("config/reload_dnsd") {
        cfg.reload_dnsd = v;
    }
    cfg
}

fn apply_interfaces(cfg: &NetworkdConfig, state: &mut NetworkState, force: bool) -> bool {
    let mut iface_buf = [0u8; 1024];
    let count = net::get_interfaces(&mut iface_buf) as usize;
    if count == 0 || count == usize::MAX {
        return false;
    }

    for i in 0..count {
        let off = i * ENTRY_SIZE;
        let method = iface_buf[off];
        if method == 2 {
            continue;
        }

        let name_len = iface_buf[off + 1].min(16);
        state.name[..name_len as usize]
            .copy_from_slice(&iface_buf[off + 2..off + 2 + name_len as usize]);
        state.name_len = name_len;
        state.method = method;

        let ok = if method == 0 {
            let now = sys::uptime_ms();
            let link_up = read_link_up();
            if !force && !link_up {
                // Kein Link -> kein Sinn einen DISCOVER zu broadcasten.
                state.last_link_up = false;
                write_status(state, false);
                return false;
            }
            if !force {
                let backoff = dhcp_backoff_ms(cfg.dhcp_retry_ms, state.dhcp_failures);
                if now.wrapping_sub(state.last_apply_ms) < backoff {
                    return false;
                }
            }
            let mut result = [0u8; 16];
            if net::dhcp(&mut result) != 0 {
                state.dhcp_failures = state.dhcp_failures.wrapping_add(1);
                state.last_apply_ms = sys::uptime_ms();
                write_status(state, false);
                return false;
            }
            state.ip.copy_from_slice(&result[0..4]);
            state.mask.copy_from_slice(&result[4..8]);
            state.gateway.copy_from_slice(&result[8..12]);
            state.dns.copy_from_slice(&result[12..16]);
            state.dhcp_failures = 0;
            true
        } else {
            let mut static_cfg = [0u8; 16];
            static_cfg[0..4].copy_from_slice(&iface_buf[off + 18..off + 22]);
            static_cfg[4..8].copy_from_slice(&iface_buf[off + 22..off + 26]);
            static_cfg[8..12].copy_from_slice(&iface_buf[off + 26..off + 30]);
            static_cfg[12..16].copy_from_slice(&iface_buf[off + 30..off + 34]);
            let rc = net::set_config(&static_cfg);
            state.ip.copy_from_slice(&static_cfg[0..4]);
            state.mask.copy_from_slice(&static_cfg[4..8]);
            state.gateway.copy_from_slice(&static_cfg[8..12]);
            state.dns.copy_from_slice(&static_cfg[12..16]);
            rc == 0
        };

        state.last_apply_ms = sys::uptime_ms();
        state.last_link_up = read_link_up();
        if cfg.reload_dnsd {
            let _ = libdns::reload();
        }
        if cfg.log_events {
            println!(
                "networkd: applied {} on {} -> {}.{}.{}.{}",
                if method == 0 { "dhcp" } else { "static" },
                state.iface_name(),
                state.ip[0],
                state.ip[1],
                state.ip[2],
                state.ip[3]
            );
        }
        write_status(state, ok);
        return ok;
    }

    false
}

fn sync_hosts_projection() {
    let _ = DNSD_HOSTS_SCHEMA.register();
    let hosts = DNSD_HOSTS_SCHEMA
        .read_string("config/hosts_blob")
        .unwrap_or_else(|| String::from(DNSD_HOSTS_DEFAULT));
    let _ = fs::mkdir("/System/etc");
    let _ = fs::mkdir("/System/etc/network");
    let _ = fs::write_bytes(HOSTS_PATH, hosts.as_bytes());
}

fn periodic_check(
    cfg: &NetworkdConfig,
    state: &mut NetworkState,
    lifecycle: Option<&mut ServiceLifecycle>,
    ready_announced: &mut bool,
) {
    let link_up = read_link_up();
    let ip_zero = state.ip == [0, 0, 0, 0];
    if link_up && !state.last_link_up {
        // Link-Up-Flanke: Backoff zuruecksetzen und sofort versuchen.
        state.dhcp_failures = 0;
        state.last_apply_ms = 0;
        let _ = apply_and_publish(cfg, state, true, lifecycle, ready_announced);
    } else if state.method == 0 && ip_zero && link_up {
        let _ = apply_and_publish(cfg, state, false, lifecycle, ready_announced);
    }
    state.last_link_up = link_up;
    write_status(state, true);
}

fn handle_requests(
    pipe_id: u32,
    cfg: &mut NetworkdConfig,
    state: &mut NetworkState,
    mut lifecycle: Option<&mut ServiceLifecycle>,
    ready_announced: &mut bool,
) -> bool {
    let mut buf = [0u8; 512];
    let n = ipc::pipe_read(pipe_id, &mut buf);
    if n == 0 || n == u32::MAX {
        return false;
    }

    let data = &buf[..n as usize];
    let mut line_start = 0usize;
    for i in 0..data.len() {
        if data[i] == b'\n' {
            if i > line_start {
                handle_request_line(
                    &data[line_start..i],
                    cfg,
                    state,
                    lifecycle.as_deref_mut(),
                    ready_announced,
                );
            }
            line_start = i + 1;
        }
    }
    if line_start < data.len() {
        handle_request_line(
            &data[line_start..],
            cfg,
            state,
            lifecycle.as_deref_mut(),
            ready_announced,
        );
    }
    true
}

fn handle_request_line(
    line: &[u8],
    cfg: &mut NetworkdConfig,
    state: &mut NetworkState,
    lifecycle: Option<&mut ServiceLifecycle>,
    ready_announced: &mut bool,
) {
    let tab_pos = match line.iter().position(|&b| b == b'\t') {
        Some(pos) => pos,
        None => return,
    };
    let tid_str = match core::str::from_utf8(&line[..tab_pos]) {
        Ok(s) => s,
        Err(_) => return,
    };
    let tid = match parse_u32(tid_str) {
        Some(v) => v,
        None => return,
    };
    let cmd = match core::str::from_utf8(&line[tab_pos + 1..]) {
        Ok(s) => s.trim(),
        Err(_) => return,
    };

    let response = match cmd {
        "reload" | "RELOAD" => {
            *cfg = load_config();
            let _ = apply_and_publish(cfg, state, true, lifecycle, ready_announced);
            String::from("OK\t0\nreloaded\n\n")
        }
        "renew" | "RENEW" => {
            let _ = apply_and_publish(cfg, state, true, lifecycle, ready_announced);
            String::from("OK\t0\nrenewed\n\n")
        }
        "status" | "STATUS" => status_response(state),
        _ => format!("ERR\tUnknown command: {}\n\n", cmd),
    };

    let reply_name = format!("networkd-{}", tid);
    let reply_pipe = ipc::pipe_open(&reply_name);
    if reply_pipe != 0 {
        ipc::pipe_write(reply_pipe, response.as_bytes());
    }
}

fn status_response(state: &NetworkState) -> String {
    format!(
        "OK\t7\niface\t{}\nmethod\t{}\nlink\t{}\nip\t{}.{}.{}.{}\ngateway\t{}.{}.{}.{}\ndns\t{}.{}.{}.{}\ndhcp_failures\t{}\n\n",
        state.iface_name(),
        if state.method == 0 { "dhcp" } else { "static" },
        if state.last_link_up { "up" } else { "down" },
        state.ip[0], state.ip[1], state.ip[2], state.ip[3],
        state.gateway[0], state.gateway[1], state.gateway[2], state.gateway[3],
        state.dns[0], state.dns[1], state.dns[2], state.dns[3],
        state.dhcp_failures
    )
}

fn write_status(state: &NetworkState, ok: bool) {
    let text = format!(
        "ok={}\niface={}\nmethod={}\nlink={}\nip={}.{}.{}.{}\nmask={}.{}.{}.{}\ngateway={}.{}.{}.{}\ndns={}.{}.{}.{}\ndhcp_failures={}\nlast_apply_ms={}\n",
        if ok { "yes" } else { "no" },
        state.iface_name(),
        if state.method == 0 { "dhcp" } else { "static" },
        if state.last_link_up { "up" } else { "down" },
        state.ip[0], state.ip[1], state.ip[2], state.ip[3],
        state.mask[0], state.mask[1], state.mask[2], state.mask[3],
        state.gateway[0], state.gateway[1], state.gateway[2], state.gateway[3],
        state.dns[0], state.dns[1], state.dns[2], state.dns[3],
        state.dhcp_failures,
        state.last_apply_ms
    );
    let _ = fs::write_bytes(STATUS_PATH, text.as_bytes());
}

fn apply_and_publish(
    cfg: &NetworkdConfig,
    state: &mut NetworkState,
    force: bool,
    lifecycle: Option<&mut ServiceLifecycle>,
    ready_announced: &mut bool,
) -> bool {
    let ok = apply_interfaces(cfg, state, force);
    if ok {
        sync_hosts_projection();
        let _ = net::reload_hosts();
    }
    if let Some(lifecycle) = lifecycle {
        if ok {
            if !*ready_announced {
                let _ = lifecycle.notify_ready();
                *ready_announced = true;
            }
            let _ = lifecycle.set_health("configured");
        } else if !*ready_announced {
            let _ = lifecycle.set_health("waiting-network");
        } else {
            let _ = lifecycle.set_health("degraded");
        }
    }
    ok
}

/// Exponential backoff fuer DHCP-Retries.
/// 0 Fehler -> base, dann verdoppeln bis 5 min Cap.
fn dhcp_backoff_ms(base: u32, failures: u32) -> u32 {
    const CAP_MS: u32 = 5 * 60 * 1000;
    let shift = failures.min(8);
    base.saturating_mul(1u32 << shift).min(CAP_MS)
}

fn read_link_up() -> bool {
    let mut cfg = [0u8; 24];
    net::get_config(&mut cfg) == 0 && cfg[22] != 0
}

fn parse_bool(value: &str) -> bool {
    matches!(value, "1" | "yes" | "true" | "on")
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

fn register_manifest() {
    let _ = NETWORKD_SCHEMA.register();
}
