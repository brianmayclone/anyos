#![no_std]
#![no_main]

use alloc::format;
use alloc::string::String;

use libconf::{ConfClient, ConfTarget, ConfValue};

anyos_std::entry!(main);

const USER_JOBS_ROOT: &str = "jobs/crond/jobs";

fn usage() {
    anyos_std::println!("Usage: crontab [-l] [-r] [-e] [file]");
    anyos_std::println!("  -l        List crontab entries");
    anyos_std::println!("  -r        Remove crontab");
    anyos_std::println!("  -e        Edit crontab (opens in vi)");
    anyos_std::println!("  file      Install crontab from file");
    anyos_std::println!("");
    anyos_std::println!("Crontab format:");
    anyos_std::println!("  minute hour day month weekday command");
    anyos_std::println!("  *      *    *   *     *       /System/bin/echo hello");
    anyos_std::println!("  */5    *    *   *     *       /System/bin/date");
    anyos_std::println!("  0      12   *   *     1-5     /System/bin/echo lunch");
}

fn user_target() -> ConfTarget {
    ConfTarget::User(anyos_std::process::getuid())
}

fn read_string(client: &mut ConfClient, target: ConfTarget, path: &str) -> Option<String> {
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

fn read_bool(client: &mut ConfClient, target: ConfTarget, path: &str) -> Option<bool> {
    match client.get_target(target, path).ok()?.value {
        Some(ConfValue::Bool(value)) => Some(value),
        _ => None,
    }
}

fn ensure_user_root(client: &mut ConfClient, target: ConfTarget) {
    let _ = client.mkdir_target(target, "jobs");
    let _ = client.mkdir_target(target, "jobs/crond");
    let _ = client.mkdir_target(target, USER_JOBS_ROOT);
}

fn format_user_crontab() -> Option<String> {
    let target = user_target();
    let mut client = ConfClient::connect("crontab").ok()?;
    let items = client.list_target(target, USER_JOBS_ROOT).ok()?;
    let mut out = String::new();

    for item in items {
        if item.kind != libconf::NodeKind::Directory {
            continue;
        }
        let base = item.path;
        let enabled = read_bool(&mut client, target, &format!("{}/enabled", base)).unwrap_or(true);
        if !enabled {
            continue;
        }
        let minute = read_string(&mut client, target, &format!("{}/minute", base))
            .unwrap_or_else(|| String::from("*"));
        let hour = read_string(&mut client, target, &format!("{}/hour", base))
            .unwrap_or_else(|| String::from("*"));
        let day = read_string(&mut client, target, &format!("{}/day", base))
            .unwrap_or_else(|| String::from("*"));
        let month = read_string(&mut client, target, &format!("{}/month", base))
            .unwrap_or_else(|| String::from("*"));
        let weekday = read_string(&mut client, target, &format!("{}/weekday", base))
            .unwrap_or_else(|| String::from("*"));
        let Some(command) = read_string(&mut client, target, &format!("{}/command", base)) else {
            continue;
        };
        out.push_str(&format!(
            "{} {} {} {} {} {}\n",
            minute, hour, day, month, weekday, command
        ));
    }

    Some(out)
}

fn list_crontab() {
    match format_user_crontab() {
        Some(content) if !content.is_empty() => anyos_std::print!("{}", content),
        _ => anyos_std::println!("crontab: no crontab for current user"),
    }
}

fn remove_crontab() {
    let target = user_target();
    match ConfClient::connect("crontab") {
        Ok(mut client) => {
            let _ = client.del_target(target, USER_JOBS_ROOT);
            anyos_std::println!("crontab: removed");
        }
        Err(_) => anyos_std::println!("crontab: confd is not available"),
    }
}

fn write_job(client: &mut ConfClient, target: ConfTarget, job_id: &str, line: &str) -> bool {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return false;
    }

    let mut fields = [""; 5];
    let mut rest = line;
    for slot in &mut fields {
        rest = rest.trim_start();
        if rest.is_empty() {
            return false;
        }
        let end = rest
            .find(|c: char| c == ' ' || c == '\t')
            .unwrap_or(rest.len());
        *slot = &rest[..end];
        rest = &rest[end..];
    }

    let command = rest.trim_start();
    if command.is_empty() {
        return false;
    }

    let base = format!("{}/{}", USER_JOBS_ROOT, job_id);
    let _ = client.mkdir_target(target, &base);
    let _ = client.set_target(
        target,
        &format!("{}/minute", base),
        ConfValue::String(String::from(fields[0])),
    );
    let _ = client.set_target(
        target,
        &format!("{}/hour", base),
        ConfValue::String(String::from(fields[1])),
    );
    let _ = client.set_target(
        target,
        &format!("{}/day", base),
        ConfValue::String(String::from(fields[2])),
    );
    let _ = client.set_target(
        target,
        &format!("{}/month", base),
        ConfValue::String(String::from(fields[3])),
    );
    let _ = client.set_target(
        target,
        &format!("{}/weekday", base),
        ConfValue::String(String::from(fields[4])),
    );
    let _ = client.set_target(
        target,
        &format!("{}/command", base),
        ConfValue::String(String::from(command)),
    );
    let _ = client.set_target(target, &format!("{}/enabled", base), ConfValue::Bool(true));
    true
}

fn install_crontab_text(content: &str) -> Result<u32, ()> {
    let target = user_target();
    let mut client = ConfClient::connect("crontab").map_err(|_| ())?;
    ensure_user_root(&mut client, target);
    let _ = client.del_target(target, USER_JOBS_ROOT);
    ensure_user_root(&mut client, target);

    let mut valid_lines = 0u32;
    for (idx, line) in content.split('\n').enumerate() {
        let job_id = format!("job{:04}", idx + 1);
        if write_job(&mut client, target, &job_id, line) {
            valid_lines += 1;
        }
    }
    Ok(valid_lines)
}

fn install_crontab(file: &str) {
    match anyos_std::fs::read_to_string(file) {
        Ok(content) => match install_crontab_text(&content) {
            Ok(valid_lines) => anyos_std::println!("crontab: installed {} entries", valid_lines),
            Err(_) => anyos_std::println!("crontab: confd is not available"),
        },
        Err(_) => anyos_std::println!("crontab: cannot open '{}'", file),
    }
}

fn edit_crontab() {
    let uid = anyos_std::process::getuid();
    let tmp_dir = "/tmp";
    let tmp_path = format!("{}/crontab-{}.tmp", tmp_dir, uid);
    let _ = anyos_std::fs::mkdir(tmp_dir);

    let content = format_user_crontab().unwrap_or_default();
    let _ = anyos_std::fs::write_bytes(&tmp_path, content.as_bytes());

    let tid = anyos_std::process::spawn("/System/bin/vi", &tmp_path);
    if tid == u32::MAX {
        anyos_std::println!("crontab: failed to launch vi");
        return;
    }
    anyos_std::process::waitpid(tid);

    match anyos_std::fs::read_to_string(&tmp_path) {
        Ok(updated) => match install_crontab_text(&updated) {
            Ok(valid_lines) => anyos_std::println!("crontab: installed {} entries", valid_lines),
            Err(_) => anyos_std::println!("crontab: confd is not available"),
        },
        Err(_) => anyos_std::println!("crontab: failed to read edited crontab"),
    }
    let _ = anyos_std::fs::unlink(&tmp_path);
}

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"");

    if raw.contains("--help") {
        anyos_std::println!("crontab - Manage cron jobs\n\nUsage: crontab [-l|-r|-e|FILE]\n\nOptions:\n  -l             List current crontab\n  -r             Remove crontab\n  -e             Edit crontab");
        return;
    }

    if args.has(b'h') {
        usage();
    } else if args.has(b'l') {
        list_crontab();
    } else if args.has(b'r') {
        remove_crontab();
    } else if args.has(b'e') {
        edit_crontab();
    } else if args.pos_count > 0 {
        install_crontab(args.positional[0]);
    } else {
        usage();
    }
}
