#![no_std]
#![no_main]

mod ipc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use anyos_std::{fs, net, println, process, sys};
use libconf_schema::{
    default_bool, default_int, default_string, manifest, RegistryScope, ServiceSchema,
};
use libsvc::ServiceLifecycle;

anyos_std::entry!(main);

const PIPE_NAME: &str = "dnsd";
const STATUS_PATH: &str = "/System/etc/network/dnsd.status";
const DEFAULT_TTL_MS: u32 = 60_000;
const DEFAULT_MAX_CACHE_ENTRIES: usize = 128;

const DNSD_DIRS: &[&str] = &["config"];
const DNSD_DEFAULTS: &[libconf_schema::DefaultEntry<'static>] = &[
    default_int("config/ttl_ms", DEFAULT_TTL_MS as i64),
    default_int("config/max_entries", DEFAULT_MAX_CACHE_ENTRIES as i64),
    default_bool("config/log_queries", false),
    default_string(
        "config/hosts_blob",
        "# anyOS hosts file\n# Format: <ip> <hostname> [aliases...]\n# Changes are picked up automatically.\n\n127.0.0.1   localhost\n",
    ),
];
const DNSD_MIGRATIONS: &[libconf_schema::MigrationStep<'static>] = &[];
const DNSD_MANIFEST: libconf_schema::RegistryManifest<'static> = manifest(
    "services/dnsd",
    RegistryScope::System,
    1,
    DNSD_DIRS,
    DNSD_DEFAULTS,
    DNSD_MIGRATIONS,
);
const DNSD_SCHEMA: ServiceSchema<'static> = ServiceSchema::new("dnsd", &DNSD_MANIFEST);

#[derive(Clone, Copy)]
struct DnsConfig {
    ttl_ms: u32,
    max_entries: usize,
    log_queries: bool,
}

impl Default for DnsConfig {
    fn default() -> Self {
        Self {
            ttl_ms: DEFAULT_TTL_MS,
            max_entries: DEFAULT_MAX_CACHE_ENTRIES,
            log_queries: false,
        }
    }
}

struct HostEntry {
    name: String,
    ip: [u8; 4],
}

struct CacheEntry {
    host: String,
    ip: [u8; 4],
    expires_at: u32,
}

pub struct DnsService {
    config: DnsConfig,
    hosts: Vec<HostEntry>,
    entries: Vec<CacheEntry>,
    cache_hits: u32,
    cache_misses: u32,
    host_hits: u32,
    queries: u32,
    reloads: u32,
    last_resolve_ms: u32,
}

impl DnsService {
    fn new() -> Self {
        let mut svc = Self {
            config: DnsConfig::default(),
            hosts: Vec::new(),
            entries: Vec::new(),
            cache_hits: 0,
            cache_misses: 0,
            host_hits: 0,
            queries: 0,
            reloads: 0,
            last_resolve_ms: 0,
        };
        let _ = svc.reload();
        svc
    }

    fn resolve(&mut self, host: &str) -> Option<[u8; 4]> {
        self.queries = self.queries.wrapping_add(1);
        if let Some(ip) = parse_ipv4(host) {
            return Some(ip);
        }

        let normalized = normalize_host(host);
        if let Some(ip) = self.lookup_hosts(&normalized) {
            self.host_hits = self.host_hits.wrapping_add(1);
            self.last_resolve_ms = sys::uptime_ms();
            self.write_status();
            return Some(ip);
        }

        let now = sys::uptime_ms();
        self.entries
            .retain(|entry| !is_expired(entry.expires_at, now));

        if let Some(entry) = self.entries.iter().find(|entry| entry.host == normalized) {
            self.cache_hits = self.cache_hits.wrapping_add(1);
            self.last_resolve_ms = now;
            self.write_status();
            return Some(entry.ip);
        }

        self.cache_misses = self.cache_misses.wrapping_add(1);
        let mut resolved = [0u8; 4];
        if net::dns(host, &mut resolved) != 0 {
            if self.config.log_queries {
                println!("dnsd: resolve failed for {}", host);
            }
            self.write_status();
            return None;
        }

        if self.config.log_queries {
            println!(
                "dnsd: {} -> {}.{}.{}.{}",
                host, resolved[0], resolved[1], resolved[2], resolved[3]
            );
        }

        self.insert(&normalized, resolved, now.wrapping_add(self.config.ttl_ms));
        self.last_resolve_ms = now;
        self.write_status();
        Some(resolved)
    }

    fn flush(&mut self) {
        self.entries.clear();
        let _ = net::flush_dns_cache();
        self.write_status();
    }

    fn reload(&mut self) -> bool {
        self.config = load_config();
        self.hosts = load_hosts();
        self.entries.clear();
        self.reloads = self.reloads.wrapping_add(1);
        let _ = net::flush_dns_cache();
        self.write_status();
        true
    }

    fn insert(&mut self, host: &str, ip: [u8; 4], expires_at: u32) {
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.host == host) {
            entry.ip = ip;
            entry.expires_at = expires_at;
            return;
        }
        if self.entries.len() >= self.config.max_entries {
            self.entries.remove(0);
        }
        self.entries.push(CacheEntry {
            host: String::from(host),
            ip,
            expires_at,
        });
    }

    fn lookup_hosts(&self, host: &str) -> Option<[u8; 4]> {
        self.hosts
            .iter()
            .find(|entry| entry.name == host)
            .map(|entry| entry.ip)
    }

    fn stats_response(&self) -> String {
        format!(
            "OK\t7\nentries\t{}\ncache_hits\t{}\ncache_misses\t{}\nhost_hits\t{}\nqueries\t{}\nreloads\t{}\nlast_resolve_ms\t{}\n\n",
            self.entries.len(),
            self.cache_hits,
            self.cache_misses,
            self.host_hits,
            self.queries,
            self.reloads,
            self.last_resolve_ms
        )
    }

    fn status_response(&self) -> String {
        format!(
            "OK\t8\nttl_ms\t{}\nmax_entries\t{}\nentries\t{}\nhosts\t{}\ncache_hits\t{}\ncache_misses\t{}\nqueries\t{}\nreloads\t{}\n\n",
            self.config.ttl_ms,
            self.config.max_entries,
            self.entries.len(),
            self.hosts.len(),
            self.cache_hits,
            self.cache_misses,
            self.queries,
            self.reloads
        )
    }

    fn write_status(&self) {
        let text = format!(
            "ttl_ms={}\nmax_entries={}\nentries={}\nhosts={}\ncache_hits={}\ncache_misses={}\nhost_hits={}\nqueries={}\nreloads={}\nlast_resolve_ms={}\n",
            self.config.ttl_ms,
            self.config.max_entries,
            self.entries.len(),
            self.hosts.len(),
            self.cache_hits,
            self.cache_misses,
            self.host_hits,
            self.queries,
            self.reloads,
            self.last_resolve_ms
        );
        let _ = fs::write_bytes(STATUS_PATH, text.as_bytes());
    }
}

fn main() {
    println!("dnsd: starting");
    register_manifest();
    let mut lifecycle = ServiceLifecycle::connect("dnsd").ok();
    if let Some(lifecycle) = lifecycle.as_mut() {
        let _ = lifecycle.notify_starting();
        let _ = lifecycle.set_health("starting");
    }

    let pipe_id = anyos_std::ipc::pipe_create(PIPE_NAME);
    if pipe_id == 0 || pipe_id == u32::MAX {
        println!("dnsd: failed to create '{}' pipe", PIPE_NAME);
        if let Some(lifecycle) = lifecycle.as_mut() {
            let _ = lifecycle.notify_failed("pipe_create_failed");
        }
        return;
    }

    let mut svc = DnsService::new();
    let mut pipe_buf = [0u8; 1024];
    if let Some(lifecycle) = lifecycle.as_mut() {
        let _ = lifecycle.notify_ready();
        let _ = lifecycle.set_health("ready");
    }

    println!("dnsd: ready (pipe='{}')", PIPE_NAME);

    loop {
        if ipc::handle_requests(&mut svc, pipe_id, &mut pipe_buf) {
            process::sleep(20);
        } else {
            process::sleep(200);
        }
    }
}

fn load_config() -> DnsConfig {
    let mut cfg = DnsConfig::default();
    if let Some(v) = DNSD_SCHEMA.read_i64("config/ttl_ms") {
        if v > 0 {
            cfg.ttl_ms = v as u32;
        }
    }
    if let Some(v) = DNSD_SCHEMA.read_i64("config/max_entries") {
        if v > 0 {
            cfg.max_entries = v as usize;
        }
    }
    if let Some(v) = DNSD_SCHEMA.read_bool("config/log_queries") {
        cfg.log_queries = v;
    }
    cfg
}

fn load_hosts() -> Vec<HostEntry> {
    let mut hosts = Vec::new();
    let text = DNSD_SCHEMA
        .read_string("config/hosts_blob")
        .unwrap_or_default();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let ip = match parts.next().and_then(parse_ipv4) {
            Some(ip) => ip,
            None => continue,
        };
        for name in parts {
            hosts.push(HostEntry {
                name: normalize_host(name),
                ip,
            });
        }
    }

    hosts
}

fn normalize_host(host: &str) -> String {
    let mut out = String::with_capacity(host.len());
    for b in host.bytes() {
        out.push((if b'A' <= b && b <= b'Z' { b + 32 } else { b }) as char);
    }
    out
}

fn is_expired(expires_at: u32, now: u32) -> bool {
    now.wrapping_sub(expires_at) < 0x8000_0000
}

fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut parts = s.split('.');
    for octet in &mut out {
        *octet = parse_u8(parts.next()?)?;
    }
    if parts.next().is_some() {
        return None;
    }
    Some(out)
}

fn parse_u8(s: &str) -> Option<u8> {
    let mut val = 0u32;
    if s.is_empty() {
        return None;
    }
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        val = val.checked_mul(10)?.checked_add((b - b'0') as u32)?;
        if val > 255 {
            return None;
        }
    }
    Some(val as u8)
}

fn parse_u32(s: &str) -> Option<u32> {
    let mut val = 0u32;
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        val = val.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(val)
}

fn parse_bool(value: &str) -> bool {
    matches!(value, "1" | "yes" | "true" | "on")
}

fn register_manifest() {
    let _ = DNSD_SCHEMA.register();
}
