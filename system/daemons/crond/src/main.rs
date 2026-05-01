#![no_std]
#![no_main]

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use anyos_std::ipc;
use anyos_std::users;
use libconf::{ConfClient, ConfTarget, ConfValue, RegistryScope};
use libconf_schema::{default_int, manifest, ServiceSchema};
use libsvc::ServiceLifecycle;

anyos_std::entry!(main);

const PIPE_NAME: &str = "crond";
const POLL_INTERVAL_MS: u32 = 1_000;
const RELOAD_INTERVAL_MIN: u32 = 5;
const CROND_DIRS: &[&str] = &["config", "jobs"];
const CROND_DEFAULTS: &[libconf_schema::DefaultEntry<'static>] = &[
    default_int("config/poll_interval_ms", POLL_INTERVAL_MS as i64),
    default_int("config/reload_interval_min", RELOAD_INTERVAL_MIN as i64),
];
const CROND_MANIFEST: libconf_schema::RegistryManifest<'static> = manifest(
    "services/crond",
    RegistryScope::System,
    1,
    CROND_DIRS,
    CROND_DEFAULTS,
    &[],
);
const CROND_SCHEMA: ServiceSchema<'static> = ServiceSchema::new("crond", &CROND_MANIFEST);

const SYSTEM_JOBS_ROOT: &str = "services/crond/jobs";
const USER_JOBS_ROOT: &str = "jobs/crond/jobs";

struct RuntimeConfig {
    poll_interval_ms: u32,
    reload_interval_min: u32,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            poll_interval_ms: POLL_INTERVAL_MS,
            reload_interval_min: RELOAD_INTERVAL_MIN,
        }
    }
}

struct CronEntry {
    minute: CronField,
    hour: CronField,
    day: CronField,
    month: CronField,
    weekday: CronField,
    command: String,
}

enum CronField {
    Any,
    Value(u8),
    List(Vec<u8>),
    Step(u8),
    Range(u8, u8),
}

impl CronField {
    fn matches(&self, value: u8) -> bool {
        match self {
            CronField::Any => true,
            CronField::Value(v) => *v == value,
            CronField::List(vals) => vals.iter().any(|v| *v == value),
            CronField::Step(step) => *step != 0 && value % *step == 0,
            CronField::Range(lo, hi) => value >= *lo && value <= *hi,
        }
    }
}

fn parse_u8(s: &str) -> Option<u8> {
    let mut val = 0u16;
    let mut found = false;
    for b in s.bytes() {
        if b.is_ascii_digit() {
            val = val.saturating_mul(10).saturating_add((b - b'0') as u16);
            found = true;
        } else {
            break;
        }
    }
    if found && val <= 255 {
        Some(val as u8)
    } else {
        None
    }
}

fn parse_u32(s: &str) -> Option<u32> {
    let mut val = 0u64;
    let mut found = false;
    for b in s.bytes() {
        if b.is_ascii_digit() {
            val = val.saturating_mul(10).saturating_add((b - b'0') as u64);
            found = true;
        } else {
            break;
        }
    }
    if found && val <= u32::MAX as u64 {
        Some(val as u32)
    } else {
        None
    }
}

fn parse_u16(s: &str) -> Option<u16> {
    parse_u32(s).and_then(|v| {
        if v <= u16::MAX as u32 {
            Some(v as u16)
        } else {
            None
        }
    })
}

fn parse_field(s: &str) -> CronField {
    if s == "*" {
        return CronField::Any;
    }
    if let Some(rest) = s.strip_prefix("*/") {
        return parse_u8(rest)
            .map(CronField::Step)
            .unwrap_or(CronField::Any);
    }
    if let Some(dash_pos) = s.find('-') {
        let lo = &s[..dash_pos];
        let hi = &s[dash_pos + 1..];
        if let (Some(a), Some(b)) = (parse_u8(lo), parse_u8(hi)) {
            return CronField::Range(a, b);
        }
    }
    if s.contains(',') {
        let mut vals = Vec::new();
        for part in s.split(',') {
            if let Some(v) = parse_u8(part) {
                vals.push(v);
            }
        }
        if !vals.is_empty() {
            return CronField::List(vals);
        }
    }
    parse_u8(s).map(CronField::Value).unwrap_or(CronField::Any)
}

fn parse_cron_line(line: &str) -> Option<CronEntry> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }

    let mut fields = Vec::new();
    let mut rest = line;
    for _ in 0..5 {
        rest = rest.trim_start();
        if rest.is_empty() {
            return None;
        }
        let end = rest
            .find(|c: char| c == ' ' || c == '\t')
            .unwrap_or(rest.len());
        fields.push(&rest[..end]);
        rest = &rest[end..];
    }

    let command = rest.trim_start();
    if command.is_empty() {
        return None;
    }

    Some(CronEntry {
        minute: parse_field(fields[0]),
        hour: parse_field(fields[1]),
        day: parse_field(fields[2]),
        month: parse_field(fields[3]),
        weekday: parse_field(fields[4]),
        command: String::from(command),
    })
}

fn read_conf_string(client: &mut ConfClient, target: ConfTarget, path: &str) -> Option<String> {
    match client.get_target(target, path).ok()?.value {
        Some(ConfValue::String(value)) => Some(value),
        Some(ConfValue::Int(value)) => Some(format!("{}", value)),
        Some(ConfValue::Bool(value)) => Some(if value {
            String::from("true")
        } else {
            String::from("false")
        }),
        Some(ConfValue::ExternalRef(value)) => Some(value),
        None => None,
    }
}

fn read_conf_bool(client: &mut ConfClient, target: ConfTarget, path: &str) -> Option<bool> {
    match client.get_target(target, path).ok()?.value {
        Some(ConfValue::Bool(value)) => Some(value),
        _ => None,
    }
}

fn load_runtime_config() -> RuntimeConfig {
    let _ = CROND_SCHEMA.register();
    let mut cfg = RuntimeConfig::default();
    if let Some(v) = CROND_SCHEMA.read_i64("config/poll_interval_ms") {
        if v > 0 && v <= u32::MAX as i64 {
            cfg.poll_interval_ms = v as u32;
        }
    }
    if let Some(v) = CROND_SCHEMA.read_i64("config/reload_interval_min") {
        if v > 0 && v <= u32::MAX as i64 {
            cfg.reload_interval_min = v as u32;
        }
    }
    cfg
}

fn load_jobs_for_target(client: &mut ConfClient, target: ConfTarget, root: &str) -> Vec<CronEntry> {
    let mut entries = Vec::new();
    let Ok(items) = client.list_target(target, root) else {
        return entries;
    };

    for item in items {
        if item.kind != libconf::NodeKind::Directory {
            continue;
        }
        let job_base = item.path;
        let enabled =
            read_conf_bool(client, target, &format!("{}/enabled", job_base)).unwrap_or(true);
        if !enabled {
            continue;
        }

        let minute = read_conf_string(client, target, &format!("{}/minute", job_base))
            .unwrap_or_else(|| String::from("*"));
        let hour = read_conf_string(client, target, &format!("{}/hour", job_base))
            .unwrap_or_else(|| String::from("*"));
        let day = read_conf_string(client, target, &format!("{}/day", job_base))
            .unwrap_or_else(|| String::from("*"));
        let month = read_conf_string(client, target, &format!("{}/month", job_base))
            .unwrap_or_else(|| String::from("*"));
        let weekday = read_conf_string(client, target, &format!("{}/weekday", job_base))
            .unwrap_or_else(|| String::from("*"));
        let Some(command) = read_conf_string(client, target, &format!("{}/command", job_base))
        else {
            continue;
        };

        let line = format!(
            "{} {} {} {} {} {}",
            minute, hour, day, month, weekday, command
        );
        if let Some(entry) = parse_cron_line(&line) {
            entries.push(entry);
        }
    }

    entries
}

fn parse_user_uids() -> Vec<u16> {
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    let len = users::listusers(&mut buf);
    if len == 0 || len == u32::MAX {
        return out;
    }

    let text = core::str::from_utf8(&buf[..len as usize]).unwrap_or("");
    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some(colon) = line.find(':') else {
            continue;
        };
        if let Some(uid) = parse_u16(&line[..colon]) {
            out.push(uid);
        }
    }
    out
}

fn load_all_crontabs() -> Vec<CronEntry> {
    let mut entries = Vec::new();
    let Ok(mut client) = ConfClient::connect("crond") else {
        return entries;
    };

    entries.extend(load_jobs_for_target(
        &mut client,
        ConfTarget::Scope(RegistryScope::System),
        SYSTEM_JOBS_ROOT,
    ));
    for uid in parse_user_uids() {
        entries.extend(load_jobs_for_target(
            &mut client,
            ConfTarget::User(uid),
            USER_JOBS_ROOT,
        ));
    }
    entries
}

struct CurrentTime {
    minute: u8,
    hour: u8,
    day: u8,
    month: u8,
    weekday: u8,
}

fn day_of_week(year: u16, month: u8, day: u8) -> u8 {
    let t = [0u16, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let mut y = year;
    if !(1..=12).contains(&month) {
        return 0;
    }
    if month < 3 {
        y = y.wrapping_sub(1);
    }
    let m = month as usize;
    let d = day as u16;
    ((y + y / 4 - y / 100 + y / 400 + t[m - 1] + d) % 7) as u8
}

fn get_current_time() -> CurrentTime {
    let mut t = [0u8; 8];
    anyos_std::sys::time(&mut t);
    let year = t[0] as u16 | ((t[1] as u16) << 8);
    CurrentTime {
        minute: t[5],
        hour: t[4],
        day: t[3],
        month: t[2],
        weekday: day_of_week(year, t[2], t[3]),
    }
}

fn execute_command(cmd: &str) {
    let cmd = cmd.trim();
    let (program, args) = match cmd.find(' ') {
        Some(pos) => (&cmd[..pos], cmd[pos + 1..].trim_start()),
        None => (cmd, ""),
    };
    let pid = anyos_std::process::spawn(program, args);
    if pid == u32::MAX {
        anyos_std::println!("crond: failed to execute: {}", cmd);
    }
}

fn handle_control(pipe_id: u32) -> Option<&'static str> {
    let mut buf = [0u8; 256];
    let n = ipc::pipe_read(pipe_id, &mut buf);
    if n == 0 || n == u32::MAX {
        return None;
    }
    let cmd = core::str::from_utf8(&buf[..n as usize])
        .unwrap_or("")
        .trim();
    match cmd {
        "reload" => Some("reload"),
        "stop" => Some("stop"),
        _ => None,
    }
}

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    if raw.contains("--help") {
        anyos_std::println!("crond - Cron scheduler daemon\n\nUsage: crond");
        return;
    }

    let mut lifecycle = ServiceLifecycle::connect("crond").ok();
    if let Some(lifecycle) = lifecycle.as_mut() {
        let _ = lifecycle.notify_starting();
        let _ = lifecycle.set_health("starting");
    }

    let pipe_id = ipc::pipe_create(PIPE_NAME);
    if pipe_id == 0 || pipe_id == u32::MAX {
        anyos_std::println!("crond: failed to create control pipe");
        if let Some(lifecycle) = lifecycle.as_mut() {
            let _ = lifecycle.notify_failed("pipe_create_failed");
        }
        return;
    }

    let mut runtime = load_runtime_config();
    let mut entries = load_all_crontabs();
    anyos_std::println!(
        "crond: starting (system_root={}, user_root={})",
        SYSTEM_JOBS_ROOT,
        USER_JOBS_ROOT
    );
    anyos_std::println!("crond: loaded {} entries", entries.len());

    if let Some(lifecycle) = lifecycle.as_mut() {
        let _ = lifecycle.notify_ready();
        let _ = lifecycle.set_health("ready");
    }

    let mut last_minute: i8 = -1;
    let mut minutes_since_reload: u32 = 0;

    loop {
        if let Some(cmd) = handle_control(pipe_id) {
            match cmd {
                "reload" => {
                    runtime = load_runtime_config();
                    entries = load_all_crontabs();
                    anyos_std::println!("crond: reloaded {} entries", entries.len());
                    minutes_since_reload = 0;
                }
                "stop" => break,
                _ => {}
            }
        }

        let now = get_current_time();
        if now.minute as i8 != last_minute {
            last_minute = now.minute as i8;
            minutes_since_reload = minutes_since_reload.saturating_add(1);
            if minutes_since_reload >= runtime.reload_interval_min.max(1) {
                minutes_since_reload = 0;
                runtime = load_runtime_config();
                entries = load_all_crontabs();
            }

            for entry in &entries {
                if entry.minute.matches(now.minute)
                    && entry.hour.matches(now.hour)
                    && entry.day.matches(now.day)
                    && entry.month.matches(now.month)
                    && entry.weekday.matches(now.weekday)
                {
                    anyos_std::println!(
                        "crond: executing [{:02}:{:02}] {}",
                        now.hour,
                        now.minute,
                        entry.command
                    );
                    execute_command(&entry.command);
                }
            }
        }

        anyos_std::process::sleep(runtime.poll_interval_ms.max(100));
    }
}
