#![no_std]
#![no_main]

use alloc::string::String;
use alloc::vec::Vec;

anyos_std::entry!(main);

// ─── Configuration ──────────────────────────────────────────────────────────

/// Directory containing crontab files (one per user/service).
const CRONTAB_DIR: &str = "/System/etc/crond";

/// How often to poll the clock (in milliseconds).
const POLL_INTERVAL_MS: u32 = 1_000; // 1 second

/// How often to reload crontab files (in minutes).
const RELOAD_INTERVAL_MIN: u8 = 5;

// ─── Cron Entry ─────────────────────────────────────────────────────────────

/// A single cron schedule entry.
struct CronEntry {
    minute: CronField,
    hour: CronField,
    day: CronField,
    month: CronField,
    weekday: CronField,
    command: String,
}

/// Represents a cron field value — wildcard, single value, or list.
enum CronField {
    /// `*` — matches any value.
    Any,
    /// Single value, e.g. `5`.
    Value(u8),
    /// List of values, e.g. `1,15,30`.
    List(Vec<u8>),
    /// Step, e.g. `*/5` — every 5th value.
    Step(u8),
    /// Range, e.g. `1-5`.
    Range(u8, u8),
}

impl CronField {
    fn matches(&self, value: u8) -> bool {
        match self {
            CronField::Any => true,
            CronField::Value(v) => *v == value,
            CronField::List(vals) => vals.iter().any(|v| *v == value),
            CronField::Step(step) => {
                if *step == 0 {
                    return false;
                }
                value % *step == 0
            }
            CronField::Range(lo, hi) => value >= *lo && value <= *hi,
        }
    }
}

// ─── Parsing ────────────────────────────────────────────────────────────────

/// Parse a decimal string into u8.
fn parse_u8(s: &str) -> Option<u8> {
    let mut val = 0u16;
    let mut found = false;
    for b in s.bytes() {
        if b >= b'0' && b <= b'9' {
            val = val.saturating_mul(10).saturating_add((b - b'0') as u16);
            found = true;
        } else {
            break;
        }
    }
    if found && val <= 255 { Some(val as u8) } else { None }
}

/// Parse a single cron field (e.g. `*`, `5`, `1,15`, `*/10`, `1-5`).
fn parse_field(s: &str) -> CronField {
    if s == "*" {
        return CronField::Any;
    }

    // Step: */N
    if let Some(rest) = s.strip_prefix("*/") {
        if let Some(step) = parse_u8(rest) {
            return CronField::Step(step);
        }
        return CronField::Any;
    }

    // Range: A-B
    if let Some(dash_pos) = s.find('-') {
        let lo = &s[..dash_pos];
        let hi = &s[dash_pos + 1..];
        if let (Some(a), Some(b)) = (parse_u8(lo), parse_u8(hi)) {
            return CronField::Range(a, b);
        }
    }

    // List: A,B,C
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

    // Single value
    if let Some(v) = parse_u8(s) {
        return CronField::Value(v);
    }

    CronField::Any
}

/// Parse a crontab line into a CronEntry.
/// Format: `minute hour day month weekday command`
fn parse_cron_line(line: &str) -> Option<CronEntry> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }

    let mut fields = Vec::new();
    let mut rest = line;

    // Extract the first 5 whitespace-separated fields
    for _ in 0..5 {
        rest = rest.trim_start();
        if rest.is_empty() {
            return None;
        }
        let end = rest.find(|c: char| c == ' ' || c == '\t').unwrap_or(rest.len());
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

/// Load all cron entries from all files in the crontab directory.
fn load_crontabs() -> Vec<CronEntry> {
    let mut entries = Vec::new();

    if let Ok(dir) = anyos_std::fs::read_dir(CRONTAB_DIR) {
        for entry in dir {
            if !entry.is_file() {
                continue;
            }
            let path = alloc::format!("{}/{}", CRONTAB_DIR, entry.name);
            if let Ok(content) = anyos_std::fs::read_to_string(&path) {
                for line in content.split('\n') {
                    if let Some(cron_entry) = parse_cron_line(line) {
                        entries.push(cron_entry);
                    }
                }
            }
        }
    }

    entries
}

// ─── Time ───────────────────────────────────────────────────────────────────

/// Get the current day of week (0=Sunday, 6=Saturday) from year/month/day
/// using Tomohiko Sakamoto's algorithm.
fn day_of_week(year: u16, month: u8, day: u8) -> u8 {
    let t = [0u16, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let mut y = year as u16;
    if month < 3 {
        y = y.wrapping_sub(1);
    }
    let m = month as usize;
    let d = day as u16;
    ((y + y / 4 - y / 100 + y / 400 + t[m - 1] + d) % 7) as u8
}

struct CurrentTime {
    minute: u8,
    hour: u8,
    day: u8,
    month: u8,
    weekday: u8,
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

// ─── Job Execution ──────────────────────────────────────────────────────────

/// Execute a command by spawning a process.
fn execute_command(cmd: &str) {
    // Split command into program and arguments
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

// ─── Main Loop ──────────────────────────────────────────────────────────────

fn main() {
    anyos_std::println!("crond: starting (crontab_dir={})", CRONTAB_DIR);

    // Ensure crontab directory exists
    anyos_std::fs::mkdir(CRONTAB_DIR);

    // Load crontabs once at startup
    let mut entries = load_crontabs();
    anyos_std::println!("crond: loaded {} entries", entries.len());

    // Track last executed minute to avoid running jobs multiple times
    let mut last_minute: i8 = -1;
    // Track minutes since last reload
    let mut minutes_since_reload: u8 = 0;

    loop {
        let now = get_current_time();

        // Only trigger once per minute
        if now.minute as i8 != last_minute {
            last_minute = now.minute as i8;

            // Periodically reload crontab files
            minutes_since_reload += 1;
            if minutes_since_reload >= RELOAD_INTERVAL_MIN {
                minutes_since_reload = 0;
                entries = load_crontabs();
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
                        now.hour, now.minute, entry.command
                    );
                    execute_command(&entry.command);
                }
            }
        }

        // Poll every second to catch minute transitions reliably
        anyos_std::process::sleep(POLL_INTERVAL_MS);
    }
}
