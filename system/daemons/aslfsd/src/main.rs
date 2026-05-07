#![cfg_attr(not(target_os = "linux"), no_std)]
#![cfg_attr(not(target_os = "linux"), no_main)]

#[cfg(target_os = "linux")]
extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use anyos_std::{fs, ipc, println, process, sys};
use libconf_schema::{
    default_bool, default_int, default_string, manifest, RegistryScope, ServiceSchema,
};
use libsvc::ServiceLifecycle;

#[cfg(not(target_os = "linux"))]
anyos_std::entry!(main);

const PIPE_NAME: &str = "aslfsd";
const STATUS_PATH: &str = "/System/var/asl/aslfsd.status";

const ASLFSD_DIRS: &[&str] = &["config"];
const ASLFSD_DEFAULTS: &[libconf_schema::DefaultEntry<'static>] = &[
    default_int("config/max_exports", 256),
    default_bool("config/audit_enabled", true),
    default_string("config/default_metadata_mode", "relaxed"),
    default_string("config/default_case_mode", "host-native"),
];
const ASLFSD_MANIFEST: libconf_schema::RegistryManifest<'static> = manifest(
    "services/aslfsd",
    RegistryScope::System,
    1,
    ASLFSD_DIRS,
    ASLFSD_DEFAULTS,
    &[],
);
const ASLFSD_SCHEMA: ServiceSchema<'static> = ServiceSchema::new("aslfsd", &ASLFSD_MANIFEST);

#[derive(Clone)]
struct Export {
    distro: String,
    id: String,
    host_path: String,
    guest_path: String,
    mode: String,
    metadata_mode: String,
    case_mode: String,
    exec_policy: String,
    watch_policy: String,
}

struct BrokerState {
    exports: Vec<Export>,
    apply_count: u32,
    rejected_count: u32,
    last_apply_ms: u32,
}

impl BrokerState {
    fn new() -> Self {
        Self {
            exports: Vec::new(),
            apply_count: 0,
            rejected_count: 0,
            last_apply_ms: 0,
        }
    }
}

fn main() {
    println!("aslfsd: starting");
    let _ = ASLFSD_SCHEMA.register();

    let mut lifecycle = ServiceLifecycle::connect("aslfsd").ok();
    if let Some(lifecycle) = lifecycle.as_mut() {
        let _ = lifecycle.notify_starting();
        let _ = lifecycle.set_health("starting");
    }

    let old_pipe = ipc::pipe_open(PIPE_NAME);
    if old_pipe != 0 {
        ipc::pipe_close(old_pipe);
    }
    let pipe_id = ipc::pipe_create(PIPE_NAME);
    if pipe_id == 0 || pipe_id == u32::MAX {
        println!("aslfsd: failed to create '{}' pipe", PIPE_NAME);
        if let Some(lifecycle) = lifecycle.as_mut() {
            let _ = lifecycle.notify_failed("pipe_create_failed");
        }
        return;
    }

    let mut state = BrokerState::new();
    write_status(&state, "ready");
    if let Some(lifecycle) = lifecycle.as_mut() {
        let _ = lifecycle.notify_ready();
        let _ = lifecycle.set_health("ready");
    }
    println!("aslfsd: ready (pipe='{}')", PIPE_NAME);

    let mut pending = String::new();
    let mut buf = [0u8; 1024];
    loop {
        if handle_requests(pipe_id, &mut pending, &mut state, &mut buf) {
            process::sleep(20);
        } else {
            process::sleep(100);
        }
    }
}

fn handle_requests(
    pipe_id: u32,
    pending: &mut String,
    state: &mut BrokerState,
    buf: &mut [u8],
) -> bool {
    let n = ipc::pipe_read(pipe_id, buf);
    if n == 0 || n == u32::MAX {
        return false;
    }
    let Ok(text) = core::str::from_utf8(&buf[..n as usize]) else {
        return true;
    };
    pending.push_str(text);
    while let Some(pos) = pending.find('\n') {
        let mut line = pending[..pos].to_string();
        pending.drain(..=pos);
        if line.ends_with('\r') {
            line.pop();
        }
        if !line.is_empty() {
            handle_line(state, &line);
        }
    }
    true
}

fn handle_line(state: &mut BrokerState, line: &str) {
    let Some(tab_pos) = line.find('\t') else {
        return;
    };
    let Some(tid) = parse_u32(&line[..tab_pos]) else {
        return;
    };
    let cmd = line[tab_pos + 1..].trim();
    let response = dispatch(state, cmd);
    let reply_name = format!("aslfsd-{}", tid);
    let reply_pipe = ipc::pipe_open(&reply_name);
    if reply_pipe != 0 {
        ipc::pipe_write(reply_pipe, response.as_bytes());
    }
}

fn dispatch(state: &mut BrokerState, cmd: &str) -> String {
    let (verb, rest) = split_first_word(cmd);
    match verb {
        "STATUS" | "status" => status_response(state),
        "CLEAR" | "clear" => {
            let distro = rest.trim();
            if distro.is_empty() {
                return err("invalid_distro");
            }
            let before = state.exports.len();
            state.exports.retain(|export| export.distro != distro);
            state.apply_count = state.apply_count.wrapping_add(1);
            state.last_apply_ms = sys::uptime_ms();
            write_status(state, "ready");
            ok_lines(alloc::vec![
                format!("distro\t{}", distro),
                format!("removed\t{}", before.saturating_sub(state.exports.len())),
            ])
        }
        "APPLY" | "apply" => apply_export(state, rest),
        "VALIDATE" | "validate" => validate_response(rest),
        _ => err("unknown_command"),
    }
}

fn apply_export(state: &mut BrokerState, rest: &str) -> String {
    let fields = split_tab_fields(rest);
    if fields.len() < 9 {
        state.rejected_count = state.rejected_count.wrapping_add(1);
        return err("invalid_apply");
    }
    if let Err(message) = validate_export_fields(&fields) {
        state.rejected_count = state.rejected_count.wrapping_add(1);
        return err(message);
    }
    let export = Export {
        distro: String::from(fields[0]),
        id: String::from(fields[1]),
        host_path: String::from(fields[2]),
        guest_path: String::from(fields[3]),
        mode: String::from(fields[4]),
        metadata_mode: String::from(fields[5]),
        case_mode: String::from(fields[6]),
        exec_policy: String::from(fields[7]),
        watch_policy: String::from(fields[8]),
    };
    state
        .exports
        .retain(|item| !(item.distro == export.distro && item.id == export.id));
    state.exports.push(export);
    state.apply_count = state.apply_count.wrapping_add(1);
    state.last_apply_ms = sys::uptime_ms();
    write_status(state, "ready");
    ok_lines(alloc::vec![
        format!("exports\t{}", state.exports.len()),
        String::from("applied\ttrue"),
    ])
}

fn validate_response(rest: &str) -> String {
    let fields = split_tab_fields(rest);
    if fields.len() < 8 {
        return err("invalid_validate");
    }
    let mut with_distro = Vec::new();
    with_distro.push("validate");
    for field in fields {
        with_distro.push(field);
    }
    match validate_export_fields(&with_distro) {
        Ok(()) => ok_lines(alloc::vec![String::from("valid\ttrue")]),
        Err(message) => ok_lines(alloc::vec![
            String::from("valid\tfalse"),
            format!("message\t{}", message),
        ]),
    }
}

fn status_response(state: &BrokerState) -> String {
    let readwrite = state
        .exports
        .iter()
        .filter(|export| export.mode == "readwrite")
        .count();
    let mut lines = alloc::vec![
        String::from("transport\tbrokered-shared-folder"),
        format!("exports\t{}", state.exports.len()),
        format!("readwrite_exports\t{}", readwrite),
        format!("apply_count\t{}", state.apply_count),
        format!("rejected_count\t{}", state.rejected_count),
        format!("last_apply_ms\t{}", state.last_apply_ms),
    ];
    if let Some(export) = state.exports.last() {
        lines.push(format!(
            "last_export\t{}\t{}\t{}->{}\t{}\t{}\t{}\t{}\t{}",
            export.distro,
            export.id,
            export.host_path,
            export.guest_path,
            export.mode,
            export.metadata_mode,
            export.case_mode,
            export.exec_policy,
            export.watch_policy
        ));
    }
    ok_lines(lines)
}

/// Validate the 9-field shared-mount tuple as defined by ADR-0004.
/// Index layout (matching APPLY/VALIDATE wire commands):
///   0 distro  1 id  2 host_path  3 guest_path  4 mode
///   5 metadata_mode  6 case_mode  7 exec_policy  8 watch_policy
///
/// Returns `Err("short_fields")` if the tuple is short — defensive
/// length check so this stays safe to call from tests with arbitrary
/// inputs.
fn validate_export_fields(fields: &[&str]) -> Result<(), &'static str> {
    if fields.len() < 9 {
        return Err("short_fields");
    }
    if fields[1].is_empty() || !fields[1].bytes().all(valid_id_byte) {
        return Err("invalid_export_id");
    }
    if !valid_host_path(fields[2]) || !valid_guest_path(fields[3]) {
        return Err("invalid_path");
    }
    if fields[4] != "readonly" && fields[4] != "readwrite" {
        return Err("invalid_mode");
    }
    if fields[5] != "strict" && fields[5] != "relaxed" {
        return Err("invalid_metadata_mode");
    }
    if fields[6] != "host-native" && fields[6] != "case-sensitive" && fields[6] != "case-folded" {
        return Err("invalid_case_mode");
    }
    if fields[7] != "inherit" && fields[7] != "noexec" && fields[7] != "host-metadata" {
        return Err("invalid_exec_policy");
    }
    if fields[8] != "best-effort" && fields[8] != "disabled" {
        return Err("invalid_watch_policy");
    }
    Ok(())
}

fn valid_host_path(path: &str) -> bool {
    path.starts_with('/') && path != "/" && !has_parent_component(path)
}

fn valid_guest_path(path: &str) -> bool {
    path.starts_with('/') && !has_parent_component(path)
}

fn has_parent_component(path: &str) -> bool {
    path.split('/').any(|component| component == "..")
}

fn valid_id_byte(b: u8) -> bool {
    matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_')
}

fn write_status(state: &BrokerState, health: &str) {
    let _ = fs::mkdir("/System/var");
    let _ = fs::mkdir("/System/var/asl");
    let mut text = format!(
        "health={}\ntransport=brokered-shared-folder\nexports={}\napply_count={}\nrejected_count={}\nlast_apply_ms={}\n",
        health,
        state.exports.len(),
        state.apply_count,
        state.rejected_count,
        state.last_apply_ms
    );
    if let Some(export) = state.exports.last() {
        text.push_str(&format!(
            "last_export={}:{}:{}->{}:{}:{}:{}:{}:{}\n",
            export.distro,
            export.id,
            export.host_path,
            export.guest_path,
            export.mode,
            export.metadata_mode,
            export.case_mode,
            export.exec_policy,
            export.watch_policy
        ));
    }
    let _ = fs::write_bytes(STATUS_PATH, text.as_bytes());
}

fn ok_lines(lines: Vec<String>) -> String {
    let mut out = format!("OK\t{}\n", lines.len());
    for line in lines {
        out.push_str(&line);
        out.push('\n');
    }
    out.push('\n');
    out
}

fn err(code: &str) -> String {
    format!("ERR\t{}\t{}\n\n", code, code)
}

fn split_first_word(s: &str) -> (&str, &str) {
    let trimmed = s.trim();
    if let Some(pos) = trimmed.find(char::is_whitespace) {
        (&trimmed[..pos], trimmed[pos + 1..].trim())
    } else {
        (trimmed, "")
    }
}

fn split_tab_fields(rest: &str) -> Vec<&str> {
    rest.split('\t').collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a 9-field tuple matching the APPLY wire format.
    fn good_fields() -> [&'static str; 9] {
        [
            "ubuntu-dev",
            "src",
            "/home/user/projects",
            "/mnt/projects",
            "readwrite",
            "relaxed",
            "host-native",
            "inherit",
            "best-effort",
        ]
    }

    // -- Path safety -------------------------------------------------------

    #[test]
    fn host_path_must_be_absolute_and_not_root() {
        assert!(valid_host_path("/home/user"));
        assert!(!valid_host_path("/")); // Refuse exporting the host root.
        assert!(!valid_host_path("home/user")); // Relative.
        assert!(!valid_host_path("")); // Empty.
    }

    #[test]
    fn host_path_rejects_dotdot() {
        assert!(!valid_host_path("/home/../etc"));
        assert!(!valid_host_path("/.."));
        assert!(!valid_host_path("/foo/.."));
    }

    #[test]
    fn guest_path_can_be_root() {
        // Guest may export at `/` if the operator chose to — the guest
        // namespace is owned by the distro.
        assert!(valid_guest_path("/"));
        assert!(valid_guest_path("/mnt/projects"));
        assert!(!valid_guest_path("mnt/projects"));
    }

    #[test]
    fn guest_path_rejects_dotdot() {
        assert!(!valid_guest_path("/mnt/../etc"));
    }

    // -- ID validation -----------------------------------------------------

    #[test]
    fn export_id_only_allows_alnum_dash_underscore() {
        assert!(b"abc".iter().copied().all(valid_id_byte));
        assert!(b"abc-123_X".iter().copied().all(valid_id_byte));
        assert!(!valid_id_byte(b' '));
        assert!(!valid_id_byte(b'/'));
        assert!(!valid_id_byte(b'.'));
        assert!(!valid_id_byte(b'\0'));
    }

    // -- validate_export_fields --------------------------------------------

    #[test]
    fn validate_accepts_canonical_tuple() {
        let f = good_fields();
        assert!(validate_export_fields(&f).is_ok());
    }

    #[test]
    fn validate_rejects_short_tuple_without_panicking() {
        // Defensive length guard — this is the difference between a
        // robust validator and a footgun for future callers.
        let short = ["a"];
        assert_eq!(validate_export_fields(&short), Err("short_fields"));
        let almost = ["a", "b", "c", "d", "e", "f", "g", "h"]; // 8 fields
        assert_eq!(validate_export_fields(&almost), Err("short_fields"));
    }

    #[test]
    fn validate_rejects_empty_id() {
        let mut f = good_fields();
        f[1] = "";
        assert_eq!(validate_export_fields(&f), Err("invalid_export_id"));
    }

    #[test]
    fn validate_rejects_id_with_space() {
        let mut f = good_fields();
        f[1] = "src dir";
        assert_eq!(validate_export_fields(&f), Err("invalid_export_id"));
    }

    #[test]
    fn validate_rejects_relative_host_path() {
        let mut f = good_fields();
        f[2] = "home/user";
        assert_eq!(validate_export_fields(&f), Err("invalid_path"));
    }

    #[test]
    fn validate_rejects_traversal_in_host_path() {
        let mut f = good_fields();
        f[2] = "/home/../etc";
        assert_eq!(validate_export_fields(&f), Err("invalid_path"));
    }

    #[test]
    fn validate_rejects_unknown_mode() {
        let mut f = good_fields();
        f[4] = "writeonly";
        assert_eq!(validate_export_fields(&f), Err("invalid_mode"));
    }

    #[test]
    fn validate_accepts_both_modes() {
        let mut f = good_fields();
        f[4] = "readonly";
        assert!(validate_export_fields(&f).is_ok());
        f[4] = "readwrite";
        assert!(validate_export_fields(&f).is_ok());
    }

    #[test]
    fn validate_rejects_unknown_metadata_mode() {
        let mut f = good_fields();
        f[5] = "loose";
        assert_eq!(validate_export_fields(&f), Err("invalid_metadata_mode"));
    }

    #[test]
    fn validate_rejects_unknown_case_mode() {
        let mut f = good_fields();
        f[6] = "windows-style";
        assert_eq!(validate_export_fields(&f), Err("invalid_case_mode"));
    }

    #[test]
    fn validate_accepts_all_case_modes() {
        for case in ["host-native", "case-sensitive", "case-folded"] {
            let mut f = good_fields();
            f[6] = case;
            assert!(
                validate_export_fields(&f).is_ok(),
                "case_mode={case} should validate"
            );
        }
    }

    #[test]
    fn validate_rejects_unknown_exec_policy() {
        let mut f = good_fields();
        f[7] = "exec-everything";
        assert_eq!(validate_export_fields(&f), Err("invalid_exec_policy"));
    }

    #[test]
    fn validate_accepts_all_exec_policies() {
        for policy in ["inherit", "noexec", "host-metadata"] {
            let mut f = good_fields();
            f[7] = policy;
            assert!(
                validate_export_fields(&f).is_ok(),
                "exec_policy={policy} should validate"
            );
        }
    }

    #[test]
    fn validate_rejects_unknown_watch_policy() {
        let mut f = good_fields();
        f[8] = "watch-everything";
        assert_eq!(validate_export_fields(&f), Err("invalid_watch_policy"));
    }

    // -- dispatch / IPC behaviour ------------------------------------------

    fn apply_line(distro: &str, id: &str) -> String {
        format!(
            "APPLY {}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            distro,
            id,
            "/home/user/projects",
            "/mnt/projects",
            "readwrite",
            "relaxed",
            "host-native",
            "inherit",
            "best-effort"
        )
    }

    #[test]
    fn apply_then_status_lists_export() {
        let mut state = BrokerState::new();
        let resp = dispatch(&mut state, &apply_line("ubuntu-dev", "src"));
        assert!(resp.starts_with("OK\t"), "{resp:?}");
        assert_eq!(state.exports.len(), 1);

        let status = dispatch(&mut state, "STATUS");
        assert!(status.contains("exports\t1"));
        assert!(status.contains("readwrite_exports\t1"));
        assert!(status.contains("transport\tbrokered-shared-folder"));
    }

    #[test]
    fn apply_replaces_existing_export_with_same_id() {
        // ADR-0004 invariant: re-applying the same id updates rather
        // than duplicates. Lets distros redefine a mount in-place.
        let mut state = BrokerState::new();
        let _ = dispatch(&mut state, &apply_line("ubuntu-dev", "src"));
        let _ = dispatch(&mut state, &apply_line("ubuntu-dev", "src"));
        assert_eq!(state.exports.len(), 1);
        assert_eq!(state.apply_count, 2);
    }

    #[test]
    fn apply_with_invalid_mode_increments_rejected_counter() {
        let mut state = BrokerState::new();
        let bad = format!(
            "APPLY {}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            "ubuntu-dev",
            "src",
            "/home/user",
            "/mnt/x",
            "writeonly", // invalid
            "relaxed",
            "host-native",
            "inherit",
            "best-effort"
        );
        let resp = dispatch(&mut state, &bad);
        assert!(resp.starts_with("ERR"), "{resp:?}");
        assert_eq!(state.exports.len(), 0);
        assert_eq!(state.rejected_count, 1);
    }

    #[test]
    fn clear_removes_only_distro_specific_exports() {
        let mut state = BrokerState::new();
        let _ = dispatch(&mut state, &apply_line("ubuntu-dev", "src"));
        let _ = dispatch(&mut state, &apply_line("ubuntu-dev", "data"));
        let _ = dispatch(&mut state, &apply_line("debian-test", "src"));
        assert_eq!(state.exports.len(), 3);

        let resp = dispatch(&mut state, "CLEAR ubuntu-dev");
        assert!(resp.contains("removed\t2"));
        assert_eq!(state.exports.len(), 1);
        assert_eq!(state.exports[0].distro, "debian-test");
    }

    #[test]
    fn validate_command_returns_valid_true_for_good_tuple() {
        let mut state = BrokerState::new();
        let line = format!(
            "VALIDATE {}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            "src",
            "/home/user",
            "/mnt/x",
            "readwrite",
            "relaxed",
            "host-native",
            "inherit",
            "best-effort"
        );
        let resp = dispatch(&mut state, &line);
        assert!(resp.contains("valid\ttrue"));
    }

    #[test]
    fn validate_command_returns_message_for_bad_tuple() {
        let mut state = BrokerState::new();
        let line = format!(
            "VALIDATE {}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            "src",
            "/home/user",
            "/mnt/x",
            "readwrite",
            "relaxed",
            "windows-style", // invalid case mode
            "inherit",
            "best-effort"
        );
        let resp = dispatch(&mut state, &line);
        assert!(resp.contains("valid\tfalse"));
        assert!(resp.contains("invalid_case_mode"));
    }

    #[test]
    fn unknown_command_yields_err() {
        let mut state = BrokerState::new();
        let resp = dispatch(&mut state, "FROBNICATE foo");
        assert!(resp.starts_with("ERR"));
        assert!(resp.contains("unknown_command"));
    }

    #[test]
    fn clear_with_empty_distro_is_rejected() {
        let mut state = BrokerState::new();
        let resp = dispatch(&mut state, "CLEAR  "); // whitespace only
        assert!(resp.starts_with("ERR"));
        assert!(resp.contains("invalid_distro"));
    }

    #[test]
    fn status_distinguishes_readonly_from_readwrite() {
        let mut state = BrokerState::new();
        // One readwrite, one readonly.
        let _ = dispatch(&mut state, &apply_line("ubuntu-dev", "rw-mount"));
        let ro_line = format!(
            "APPLY {}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            "ubuntu-dev",
            "ro-mount",
            "/home/user/docs",
            "/mnt/docs",
            "readonly",
            "relaxed",
            "host-native",
            "inherit",
            "best-effort"
        );
        let _ = dispatch(&mut state, &ro_line);
        assert_eq!(state.exports.len(), 2);

        let status = dispatch(&mut state, "STATUS");
        assert!(status.contains("exports\t2"));
        assert!(status.contains("readwrite_exports\t1"));
    }
}
