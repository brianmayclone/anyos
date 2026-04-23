use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::config::ConfigStore;
use crate::errors::AsldError;
use crate::model::{
    ExecInvocation, MountSpec, PortForwardSpec, ShellSession, VmExitEvent, VmStatusSummary,
};
use crate::runtime::RuntimeService;

pub struct IpcState {
    pending_request: String,
}

impl IpcState {
    pub fn new() -> Self {
        Self {
            pending_request: String::new(),
        }
    }
}

pub fn handle_requests<S: ConfigStore>(
    runtime: &mut RuntimeService,
    store: &mut S,
    state: &mut IpcState,
    pipe_id: u32,
    buf: &mut [u8],
) -> bool {
    let n = anyos_std::ipc::pipe_read(pipe_id, buf);
    if n == 0 || n == u32::MAX {
        return false;
    }

    let data = match core::str::from_utf8(&buf[..n as usize]) {
        Ok(text) => text,
        Err(_) => return true,
    };
    state.pending_request.push_str(data);

    while let Some(pos) = state.pending_request.find('\n') {
        let mut line = state.pending_request[..pos].to_string();
        state.pending_request.drain(..=pos);
        if line.ends_with('\r') {
            line.pop();
        }
        if !line.is_empty() {
            handle_single_request(runtime, store, &line);
        }
    }

    true
}

fn handle_single_request<S: ConfigStore>(runtime: &mut RuntimeService, store: &mut S, line: &str) {
    let Some(tab_pos) = line.find('\t') else {
        return;
    };
    let Some(tid) = parse_u32(&line[..tab_pos]) else {
        return;
    };
    let cmd = line[tab_pos + 1..].trim();
    if cmd.is_empty() {
        return;
    }
    let response = dispatch(runtime, store, cmd);
    let reply_pipe_name = format!("asld-{}", tid);
    let reply_pipe = anyos_std::ipc::pipe_open(&reply_pipe_name);
    if reply_pipe != 0 {
        anyos_std::ipc::pipe_write(reply_pipe, response.as_bytes());
    }
}

fn dispatch<S: ConfigStore>(runtime: &mut RuntimeService, store: &mut S, cmd: &str) -> String {
    let (verb, rest) = split_first_word(cmd);
    match verb {
        "LIST" | "list" => match runtime.list(store) {
            Ok(items) => {
                let mut out = format!("OK\t{}\n", items.len());
                for item in items {
                    out.push_str(&format!("{}\t{}\t{}\n", item.name, item.state.as_str(), item.health.as_str()));
                }
                out.push('\n');
                out
            }
            Err(err) => err_line(&err),
        },
        "STATUS" | "status" => match runtime.status(store, rest) {
            Ok(status) => format!(
                "OK\t1\nname\t{}\nstate\t{}\nhealth\t{}\nagent\t{}\n\n",
                status.name,
                status.state.as_str(),
                status.health.as_str(),
                status.agent_state.as_str()
            ),
            Err(err) => err_line(&err),
        },
        "CREATE" | "create" => {
            let mut parts = rest.split_whitespace();
            let Some(name) = parts.next() else { return err_line(&AsldError::InvalidArgument("name")); };
            let Some(image_ref) = parts.next() else { return err_line(&AsldError::InvalidArgument("image_ref")); };
            let Some(owner) = parts.next() else { return err_line(&AsldError::InvalidArgument("owner")); };
            match runtime.create(store, name, image_ref, owner) {
                Ok(status) => format!("OK\t1\ncreated\t{}\nstate\t{}\n\n", status.name, status.state.as_str()),
                Err(err) => err_line(&err),
            }
        }
        "START" | "start" => match runtime.start(store, rest) {
            Ok(status) => format!("OK\t1\nstate\t{}\nhealth\t{}\n\n", status.state.as_str(), status.health.as_str()),
            Err(err) => err_line(&err),
        },
        "STOP" | "stop" => match runtime.stop(store, rest) {
            Ok(status) => format!("OK\t1\nstate\t{}\n\n", status.state.as_str()),
            Err(err) => err_line(&err),
        },
        "AGENT_STATUS" | "agent_status" => match runtime.status(store, rest) {
            Ok(status) => format!("OK\t1\nagent\t{}\n\n", status.agent_state.as_str()),
            Err(err) => err_line(&err),
        },
        "VM_STATUS" | "vm_status" => {
            let Some(name) = first_tab_field(rest) else { return err_line(&AsldError::InvalidArgument("name")); };
            match runtime.vm_status(name) {
                Ok(status) => ok_lines(format_vm_status_lines(&status)),
                Err(err) => err_line(&err),
            }
        }
        "VM_EVENTS" | "vm_events" => {
            let Some(name) = first_tab_field(rest) else { return err_line(&AsldError::InvalidArgument("name")); };
            match runtime.vm_events(name) {
                Ok(events) => ok_lines(format_vm_event_lines(&events)),
                Err(err) => err_line(&err),
            }
        }
        "SHELL_OPEN" | "shell_open" => {
            let fields = split_tab_fields(rest);
            if fields.is_empty() {
                return err_line(&AsldError::InvalidArgument("shell_open"));
            }
            let fallback_console = fields.get(2).copied().unwrap_or("0") == "1";
            match runtime.open_shell_session(
                store,
                fields[0],
                fields.get(1).copied().filter(|name| !name.is_empty()),
                fallback_console,
            ) {
                Ok(session) => ok_lines(format_shell_lines(&session)),
                Err(err) => err_line(&err),
            }
        }
        "EXEC" | "exec" => {
            let fields = split_tab_fields(rest);
            if fields.len() < 2 {
                return err_line(&AsldError::InvalidArgument("exec"));
            }
            let fallback_console = fields.get(1).copied().unwrap_or("0") == "1";
            let cwd = fields.get(2).copied().filter(|value| !value.is_empty());
            let env_pairs = parse_env_fields(fields.get(3).copied().unwrap_or(""));
            let argv = parse_list_field(fields.get(4).copied().unwrap_or(""));
            match runtime.exec_command(store, fields[0], &argv, cwd, &env_pairs, fallback_console) {
                Ok(exec) => ok_lines(format_exec_lines(&exec)),
                Err(err) => err_line(&err),
            }
        }
        "MOUNT_LIST" | "mount_list" => {
            let Some(name) = first_tab_field(rest) else { return err_line(&AsldError::InvalidArgument("name")); };
            match runtime.list_mounts(store, name) {
                Ok(mounts) => ok_lines(format_mount_lines(&mounts)),
                Err(err) => err_line(&err),
            }
        }
        "MOUNT_SHOW" | "mount_show" => {
            let fields = split_tab_fields(rest);
            if fields.len() < 2 {
                return err_line(&AsldError::InvalidArgument("mount_show"));
            }
            match runtime.show_mount(store, fields[0], fields[1]) {
                Ok(mount) => ok_lines(format_mount_lines(&alloc::vec![mount])),
                Err(err) => err_line(&err),
            }
        }
        "MOUNT_ADD" | "mount_add" => {
            let fields = split_tab_fields(rest);
            match parse_mount_add(&fields) {
                Ok((name, mount)) => match runtime.add_mount(store, name, &mount) {
                    Ok(mounts) => ok_lines(format_mount_lines(&mounts)),
                    Err(err) => err_line(&err),
                },
                Err(err) => err_line(&err),
            }
        }
        "MOUNT_REMOVE" | "mount_remove" => {
            let fields = split_tab_fields(rest);
            if fields.len() < 2 {
                return err_line(&AsldError::InvalidArgument("mount_remove"));
            }
            match runtime.remove_mount(store, fields[0], fields[1]) {
                Ok(mounts) => ok_lines(format_mount_lines(&mounts)),
                Err(err) => err_line(&err),
            }
        }
        "PORT_LIST" | "port_list" => {
            let Some(name) = first_tab_field(rest) else { return err_line(&AsldError::InvalidArgument("name")); };
            match runtime.list_port_forwards(store, name) {
                Ok(rules) => ok_lines(format_port_lines(&rules)),
                Err(err) => err_line(&err),
            }
        }
        "PORT_ADD" | "port_add" => {
            let fields = split_tab_fields(rest);
            match parse_port_add(&fields) {
                Ok((name, rule)) => match runtime.add_port_forward(store, name, &rule) {
                    Ok(rules) => ok_lines(format_port_lines(&rules)),
                    Err(err) => err_line(&err),
                },
                Err(err) => err_line(&err),
            }
        }
        "PORT_REMOVE" | "port_remove" => {
            let fields = split_tab_fields(rest);
            if fields.len() < 2 {
                return err_line(&AsldError::InvalidArgument("port_remove"));
            }
            match runtime.remove_port_forward(store, fields[0], fields[1]) {
                Ok(rules) => ok_lines(format_port_lines(&rules)),
                Err(err) => err_line(&err),
            }
        }
        _ => String::from("ERR\tunknown_command\n\n"),
    }
}

fn err_line(err: &AsldError) -> String {
    format!("ERR\t{}\t{}\n\n", err.code(), err.message())
}

fn split_first_word(s: &str) -> (&str, &str) {
    if let Some(pos) = s.find(' ') {
        (&s[..pos], s[pos + 1..].trim())
    } else {
        (s.trim(), "")
    }
}

fn first_tab_field(s: &str) -> Option<&str> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else if let Some((first, _)) = trimmed.split_once('\t') {
        Some(first)
    } else {
        Some(trimmed)
    }
}

fn split_tab_fields(s: &str) -> Vec<&str> {
    s.split('\t').filter(|field| !field.is_empty()).collect()
}

fn parse_mount_add<'a>(fields: &[&'a str]) -> Result<(&'a str, MountSpec), AsldError> {
    if fields.len() < 9 {
        return Err(AsldError::InvalidArgument("mount_add"));
    }
    Ok((
        fields[0],
        MountSpec {
            id: String::from(fields[1]),
            host_path: String::from(fields[2]),
            guest_path: String::from(fields[3]),
            mode: String::from(fields[4]),
            metadata_mode: String::from(fields[5]),
            case_mode: String::from(fields[6]),
            exec_policy: String::from(fields[7]),
            watch_policy: String::from(fields[8]),
            description: String::from(fields.get(9).copied().unwrap_or("")),
        },
    ))
}

fn parse_port_add<'a>(fields: &[&'a str]) -> Result<(&'a str, PortForwardSpec), AsldError> {
    if fields.len() < 6 {
        return Err(AsldError::InvalidArgument("port_add"));
    }
    let listen_port = parse_u16(fields[3]).ok_or(AsldError::InvalidArgument("listen_port"))?;
    let guest_port = parse_u16(fields[4]).ok_or(AsldError::InvalidArgument("guest_port"))?;
    Ok((
        fields[0],
        PortForwardSpec {
            id: String::from(fields[1]),
            listen_address: String::from(fields[2]),
            listen_port,
            guest_port,
            protocol: String::from(fields[5]),
            description: String::from(fields.get(6).copied().unwrap_or("")),
        },
    ))
}

fn parse_u16(s: &str) -> Option<u16> {
    let value = parse_u32(s)?;
    if value == 0 || value > u16::MAX as u32 {
        return None;
    }
    Some(value as u16)
}

fn parse_list_field(s: &str) -> Vec<String> {
    if s.is_empty() {
        return Vec::new();
    }
    s.split('\x1f').map(String::from).collect()
}

fn parse_env_fields(s: &str) -> Vec<(&str, &str)> {
    if s.is_empty() {
        return Vec::new();
    }
    s.split('\x1f')
        .filter_map(|field| field.split_once('='))
        .collect()
}

fn format_shell_lines(session: &ShellSession) -> Vec<String> {
    alloc::vec![
        format!(
            "{}\t{}\t{}\t{}\t{}\t{}",
            session.session_id,
            session.session_name,
            session.mode.as_str(),
            session.console_pipe_name,
            session.stdin_pipe_name,
            session.attached_pid
        ),
        format!("reused\t{}", if session.reused { "true" } else { "false" }),
    ]
}

fn format_exec_lines(exec: &ExecInvocation) -> Vec<String> {
    alloc::vec![
        format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
            exec.exec_id,
            exec.mode.as_str(),
            exec.cwd,
            exec.env_count,
            exec.command_line,
            exec.stdout_pipe_name,
            exec.stdin_pipe_name
        ),
        format!("pid\t{}", exec.attached_pid),
    ]
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

fn format_mount_lines(mounts: &[MountSpec]) -> Vec<String> {
    let mut out = Vec::new();
    for mount in mounts {
        out.push(format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            mount.id,
            mount.host_path,
            mount.guest_path,
            mount.mode,
            mount.metadata_mode,
            mount.case_mode,
            mount.exec_policy,
            mount.watch_policy,
            mount.description
        ));
    }
    out
}

fn format_port_lines(rules: &[PortForwardSpec]) -> Vec<String> {
    let mut out = Vec::new();
    for rule in rules {
        out.push(format!(
            "{}\t{}\t{}\t{}\t{}\t{}",
            rule.id,
            rule.listen_address,
            rule.listen_port,
            rule.guest_port,
            rule.protocol,
            rule.description
        ));
    }
    out
}

fn format_vm_status_lines(status: &VmStatusSummary) -> Vec<String> {
    alloc::vec![
        format!("backend\t{}", status.backend),
        format!("run_state\t{}", status.run_state.as_str()),
        format!("guest_memory_mb\t{}", status.guest_memory_mb),
        format!("boot_summary\t{}", status.boot_summary),
        format!("last_exit_summary\t{}", status.last_exit_summary),
        format!("total_exits\t{}", status.total_exits),
        format!("recent_exit_count\t{}", status.recent_exit_count),
    ]
}

fn format_vm_event_lines(events: &[VmExitEvent]) -> Vec<String> {
    let mut out = Vec::new();
    for event in events {
        out.push(format!(
            "{}\t{}\t{}\t{}\t{:#x}\t{:#x}\t{:#x}",
            event.seq,
            event.reason,
            if event.fatal { "fatal" } else { "info" },
            event.summary,
            event.qualification,
            event.guest_phys_addr,
            event.guest_virt_addr
        ));
    }
    out
}

fn parse_u32(s: &str) -> Option<u32> {
    let mut out = 0u32;
    if s.is_empty() {
        return None;
    }
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        out = out.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use crate::config::FakeStore;
    use crate::runtime::RuntimeService;

    use super::dispatch;

    #[test]
    fn list_empty_returns_zero_rows() {
        let mut runtime = RuntimeService::new();
        let mut store = FakeStore::default();
        let response = dispatch(&mut runtime, &mut store, "LIST");
        assert!(response.starts_with("OK\t0"));
    }

    #[test]
    fn create_then_status_roundtrip() {
        let mut runtime = RuntimeService::new();
        let mut store = FakeStore::default();
        let create = dispatch(&mut runtime, &mut store, "CREATE ubuntu-dev ubuntu-24.04-x86_64-v1 strati");
        assert!(create.starts_with("OK"));
        let status = dispatch(&mut runtime, &mut store, "STATUS ubuntu-dev");
        assert!(status.contains("name\tubuntu-dev"));
    }

    #[test]
    fn mount_commands_roundtrip() {
        let mut runtime = RuntimeService::new();
        let mut store = FakeStore::default();
        let _ = dispatch(&mut runtime, &mut store, "CREATE ubuntu-dev ubuntu-24.04-x86_64-v1 strati");
        let add = dispatch(
            &mut runtime,
            &mut store,
            "MOUNT_ADD ubuntu-dev\tworkspace\t/Users/strati/work\t/mnt/work\treadwrite\trelaxed\thost-native\tinherit\tbest-effort\tWorkspace",
        );
        assert!(add.contains("workspace\t/Users/strati/work"));
        let show = dispatch(&mut runtime, &mut store, "MOUNT_SHOW ubuntu-dev\tworkspace");
        assert!(show.contains("/mnt/work"));
        let remove = dispatch(&mut runtime, &mut store, "MOUNT_REMOVE ubuntu-dev\tworkspace");
        assert!(remove.starts_with("OK\t0"));
    }

    #[test]
    fn port_commands_roundtrip() {
        let mut runtime = RuntimeService::new();
        let mut store = FakeStore::default();
        let _ = dispatch(&mut runtime, &mut store, "CREATE ubuntu-dev ubuntu-24.04-x86_64-v1 strati");
        let add = dispatch(
            &mut runtime,
            &mut store,
            "PORT_ADD ubuntu-dev\tweb\t127.0.0.1\t3000\t3000\ttcp\tWeb",
        );
        assert!(add.contains("web\t127.0.0.1\t3000\t3000\ttcp"));
        let list = dispatch(&mut runtime, &mut store, "PORT_LIST ubuntu-dev");
        assert!(list.contains("web\t127.0.0.1"));
        let remove = dispatch(&mut runtime, &mut store, "PORT_REMOVE ubuntu-dev\tweb");
        assert!(remove.starts_with("OK\t0"));
    }

    #[test]
    fn shell_and_exec_roundtrip() {
        let mut runtime = RuntimeService::new();
        let mut store = FakeStore::default();
        let _ = dispatch(&mut runtime, &mut store, "CREATE ubuntu-dev ubuntu-24.04-x86_64-v1 strati");
        let _ = dispatch(&mut runtime, &mut store, "START ubuntu-dev");
        let shell = dispatch(&mut runtime, &mut store, "SHELL_OPEN ubuntu-dev\tdev\t0");
        assert!(shell.contains("sh-"));
        assert!(shell.contains("agent"));
        assert!(shell.contains("asl-shell-stdin-"));
        let exec = dispatch(
            &mut runtime,
            &mut store,
            "EXEC ubuntu-dev\t0\t/workspace\tRUST_BACKTRACE=1\x1fTERM=xterm\tcargo\x1ftest",
        );
        assert!(exec.contains("exec-"));
        assert!(exec.contains("cargo test"));
        assert!(exec.contains("asl-agent-exec-stdout-"));
    }

    #[test]
    fn vm_status_and_events_roundtrip() {
        let mut runtime = RuntimeService::new();
        let mut store = FakeStore::default();
        let _ = dispatch(&mut runtime, &mut store, "CREATE ubuntu-dev ubuntu-24.04-x86_64-v1 strati");
        let _ = dispatch(&mut runtime, &mut store, "START ubuntu-dev");
        let status = dispatch(&mut runtime, &mut store, "VM_STATUS ubuntu-dev");
        assert!(status.contains("backend\t"));
        assert!(status.contains("boot_summary\t"));
        let events = dispatch(&mut runtime, &mut store, "VM_EVENTS ubuntu-dev");
        assert!(events.starts_with("OK\t0"));
    }
}
