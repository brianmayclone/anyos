use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::config::ConfigStore;
use crate::errors::AsldError;
use crate::model::{
    AgentState, ExecInvocation, MountSpec, MountValidation, NetworkPolicy, NetworkValidation,
    PortForwardSpec, ShellSession, StorageValidation, VmExitEvent, VmStatusSummary,
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
    let (verb, _) = split_first_word(cmd);
    crate::log::info("ipc", &format!("rx tid={} verb={}", tid, verb));
    let response = dispatch(runtime, store, cmd);
    let ok = response.starts_with("OK");
    let reply_pipe_name = format!("asld-{}", tid);
    let reply_pipe = anyos_std::ipc::pipe_open(&reply_pipe_name);
    if reply_pipe == 0 || reply_pipe == u32::MAX {
        crate::log::warn(
            "ipc",
            &format!("reply pipe '{}' not open; tid={}", reply_pipe_name, tid),
        );
        return;
    }
    let written = anyos_std::ipc::pipe_write(reply_pipe, response.as_bytes());
    if written == u32::MAX {
        crate::log::warn(
            "ipc",
            &format!("reply pipe_write failed tid={} verb={}", tid, verb),
        );
    } else if !ok {
        // First line carries the error code; log it so CLI failures are
        // visible server-side without having to rerun with more verbosity.
        let first = response.split('\n').next().unwrap_or("");
        crate::log::warn("ipc", &format!("tx tid={} verb={} {}", tid, verb, first));
    }
}

fn dispatch<S: ConfigStore>(runtime: &mut RuntimeService, store: &mut S, cmd: &str) -> String {
    let (verb, rest) = split_first_word(cmd);
    match verb {
        "LIST" | "list" => match runtime.list(store) {
            Ok(items) => {
                let mut out = format!("OK\t{}\n", items.len());
                for item in items {
                    out.push_str(&format!(
                        "{}\t{}\t{}\n",
                        item.name,
                        item.state.as_str(),
                        item.health.as_str()
                    ));
                }
                out.push('\n');
                out
            }
            Err(err) => err_line(&err),
        },
        "STATUS" | "status" => match runtime.status(store, rest) {
            Ok(status) => {
                let mut out = format!(
                    "OK\t1\nname\t{}\nstate\t{}\nhealth\t{}\nagent\t{}\n",
                    status.name,
                    status.state.as_str(),
                    status.health.as_str(),
                    status.agent_state.as_str()
                );
                if let Some(err) = &status.last_error {
                    if !err.is_empty() {
                        out.push_str(&format!("last_error\t{}\n", sanitize_line(err)));
                    }
                }
                if let Ok(vm) = runtime.vm_status(rest) {
                    if !vm.boot_summary.is_empty() {
                        out.push_str(&format!(
                            "boot_summary\t{}\n",
                            sanitize_line(&vm.boot_summary)
                        ));
                    }
                    if !vm.last_exit_summary.is_empty() && vm.last_exit_summary != "none" {
                        out.push_str(&format!(
                            "last_exit\t{}\n",
                            sanitize_line(&vm.last_exit_summary)
                        ));
                    }
                }
                out.push('\n');
                out
            }
            Err(err) => err_line(&err),
        },
        "CREATE" | "create" => {
            let fields = parse_create_fields(rest);
            let Some(name) = fields.first().copied() else {
                return err_line(&AsldError::InvalidArgument("name"));
            };
            let Some(image_ref) = fields.get(1).copied() else {
                return err_line(&AsldError::InvalidArgument("image_ref"));
            };
            let Some(owner) = fields.get(2).copied() else {
                return err_line(&AsldError::InvalidArgument("owner"));
            };
            let kernel_profile = fields.get(3).copied().filter(|value| *value != "-");
            match runtime.create_with_kernel_profile(store, name, image_ref, owner, kernel_profile)
            {
                Ok(status) => format!(
                    "OK\t1\ncreated\t{}\nstate\t{}\n\n",
                    status.name,
                    status.state.as_str()
                ),
                Err(err) => err_line(&err),
            }
        }
        "DELETE" | "delete" => {
            let fields = split_tab_fields(rest);
            if fields.is_empty() {
                return err_line(&AsldError::InvalidArgument("delete"));
            }
            let force = fields.get(1).copied().unwrap_or("0") == "1";
            match runtime.delete(store, fields[0], force) {
                Ok(cfg) => ok_lines(alloc::vec![
                    format!("deleted\t{}", cfg.name),
                    format!("base_image_ref\t{}", cfg.base_image_ref),
                ]),
                Err(err) => err_line(&err),
            }
        }
        "CLONE" | "clone" => {
            let fields = split_tab_fields(rest);
            if fields.len() < 2 {
                return err_line(&AsldError::InvalidArgument("clone"));
            }
            let include_mounts = fields.get(3).copied().unwrap_or("1") != "0";
            match runtime.clone(
                store,
                fields[0],
                fields[1],
                optional_field(fields.get(2).copied().unwrap_or("-")),
                include_mounts,
            ) {
                Ok(lines) => ok_lines(lines),
                Err(err) => err_line(&err),
            }
        }
        "EXPORT" | "export" => {
            let Some(name) = first_tab_field(rest) else {
                return err_line(&AsldError::InvalidArgument("name"));
            };
            match runtime.export_lines(store, name) {
                Ok(lines) => ok_lines(lines),
                Err(err) => err_line(&err),
            }
        }
        "CONFIG_SHOW" | "config_show" => {
            let Some(name) = first_tab_field(rest) else {
                return err_line(&AsldError::InvalidArgument("name"));
            };
            match runtime.config_lines(store, name) {
                Ok(lines) => ok_lines(lines),
                Err(err) => err_line(&err),
            }
        }
        "CONFIG_SET_RESOURCES" | "config_set_resources" => {
            let fields = split_tab_fields(rest);
            if fields.is_empty() {
                return err_line(&AsldError::InvalidArgument("config_set_resources"));
            }
            let memory_mb = match parse_optional_u32(fields.get(1).copied().unwrap_or("-")) {
                Ok(value) => value,
                Err(err) => return err_line(&err),
            };
            let vcpu_count = match parse_optional_u16(fields.get(2).copied().unwrap_or("-")) {
                Ok(value) => value,
                Err(err) => return err_line(&err),
            };
            match runtime.update_resources(store, fields[0], memory_mb, vcpu_count) {
                Ok(lines) => ok_lines(lines),
                Err(err) => err_line(&err),
            }
        }
        "STORAGE_VALIDATE" | "storage_validate" => {
            let Some(name) = first_tab_field(rest) else {
                return err_line(&AsldError::InvalidArgument("name"));
            };
            match runtime.validate_storage(store, name) {
                Ok(report) => ok_lines(format_storage_validation_lines(&report)),
                Err(err) => err_line(&err),
            }
        }
        "STORAGE_IMPORT" | "storage_import" => {
            let fields = split_tab_fields(rest);
            if fields.len() < 2 {
                return err_line(&AsldError::InvalidArgument("storage_import"));
            }
            match runtime.import_base_image(store, fields[0], fields[1]) {
                Ok(lines) => ok_lines(lines),
                Err(err) => err_line(&err),
            }
        }
        "NETWORK_SHOW" | "network_show" => {
            let Some(name) = first_tab_field(rest) else {
                return err_line(&AsldError::InvalidArgument("name"));
            };
            match runtime.network_policy(store, name) {
                Ok(policy) => ok_lines(format_network_policy_lines(&policy)),
                Err(err) => err_line(&err),
            }
        }
        "NETWORK_SET" | "network_set" => {
            let fields = split_tab_fields(rest);
            if fields.is_empty() {
                return err_line(&AsldError::InvalidArgument("network_set"));
            }
            let allow_outbound = match parse_optional_bool(fields.get(3).copied().unwrap_or("-")) {
                Ok(value) => value,
                Err(err) => return err_line(&err),
            };
            match runtime.update_network(
                store,
                fields[0],
                optional_field(fields.get(1).copied().unwrap_or("-")),
                optional_field(fields.get(2).copied().unwrap_or("-")),
                allow_outbound,
            ) {
                Ok(lines) => ok_lines(lines),
                Err(err) => err_line(&err),
            }
        }
        "NETWORK_VALIDATE" | "network_validate" => {
            let Some(name) = first_tab_field(rest) else {
                return err_line(&AsldError::InvalidArgument("name"));
            };
            match runtime.validate_network(store, name) {
                Ok(report) => ok_lines(format_network_validation_lines(&report)),
                Err(err) => err_line(&err),
            }
        }
        "START" | "start" => match runtime.start(store, rest) {
            Ok(status) => format!(
                "OK\t1\nstate\t{}\nhealth\t{}\n\n",
                status.state.as_str(),
                status.health.as_str()
            ),
            Err(err) => err_line(&err),
        },
        "RESTART" | "restart" => match runtime.restart(store, rest) {
            Ok(status) => format!(
                "OK\t1\nstate\t{}\nhealth\t{}\n\n",
                status.state.as_str(),
                status.health.as_str()
            ),
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
        "AGENT_RESTART" | "agent_restart" => {
            let Some(name) = first_tab_field(rest) else {
                return err_line(&AsldError::InvalidArgument("name"));
            };
            match runtime.restart_agent(store, name) {
                Ok(lines) => ok_lines(lines),
                Err(err) => err_line(&err),
            }
        }
        "AGENT_HELLO" | "agent_hello" => {
            agent_update(runtime, store, rest, AgentState::Connected, "agent_hello")
        }
        "AGENT_READY" | "agent_ready" => {
            agent_update(runtime, store, rest, AgentState::Ready, "agent_ready")
        }
        "AGENT_HEARTBEAT" | "agent_heartbeat" => agent_update(
            runtime,
            store,
            rest,
            AgentState::Connected,
            "agent_heartbeat",
        ),
        "AGENT_DISCONNECT" | "agent_disconnect" => agent_update(
            runtime,
            store,
            rest,
            AgentState::Disconnected,
            "agent_disconnect",
        ),
        "VM_STATUS" | "vm_status" => {
            let Some(name) = first_tab_field(rest) else {
                return err_line(&AsldError::InvalidArgument("name"));
            };
            match runtime.vm_status(name) {
                Ok(status) => ok_lines(format_vm_status_lines(&status)),
                Err(err) => err_line(&err),
            }
        }
        "VM_EVENTS" | "vm_events" => {
            let Some(name) = first_tab_field(rest) else {
                return err_line(&AsldError::InvalidArgument("name"));
            };
            match runtime.vm_events(name) {
                Ok(events) => ok_lines(format_vm_event_lines(&events)),
                Err(err) => err_line(&err),
            }
        }
        "VM_EVENTS_TAIL" | "vm_events_tail" => {
            let fields = split_tab_fields(rest);
            if fields.is_empty() {
                return err_line(&AsldError::InvalidArgument("vm_events_tail"));
            }
            let limit = fields
                .get(1)
                .and_then(|raw| raw.parse::<usize>().ok())
                .unwrap_or(10);
            match runtime.vm_events_tail(fields[0], limit) {
                Ok(events) => ok_lines(format_vm_event_lines(&events)),
                Err(err) => err_line(&err),
            }
        }
        "VM_EVENTS_CLEAR" | "vm_events_clear" => {
            let Some(name) = first_tab_field(rest) else {
                return err_line(&AsldError::InvalidArgument("name"));
            };
            match runtime.clear_vm_events(name) {
                Ok(count) => ok_lines(alloc::vec![format!("cleared\t{}", count)]),
                Err(err) => err_line(&err),
            }
        }
        "DIAGNOSE" | "diagnose" => {
            let Some(name) = first_tab_field(rest) else {
                return err_line(&AsldError::InvalidArgument("name"));
            };
            match runtime.diagnose(store, name) {
                Ok(lines) => ok_lines(lines),
                Err(err) => err_line(&err),
            }
        }
        "DOCTOR" | "doctor" => {
            // RunDoctor: pass/warn/fail per subsystem + summary line.
            // Designed for monitoring/CI consumers, not a replacement
            // for DIAGNOSE (which is the full status dump).
            let Some(name) = first_tab_field(rest) else {
                return err_line(&AsldError::InvalidArgument("name"));
            };
            match runtime.doctor_report(store, name) {
                Ok(lines) => ok_lines(lines),
                Err(err) => err_line(&err),
            }
        }
        "LOGS" | "logs" => {
            // LOGS <distro|*>\t<limit>
            //   distro="*" or empty -> no substring filter (full asld log)
            //   limit=0             -> no cap
            // Returns one OK line per log entry, oldest first within
            // the returned tail. Best-effort — silently empty if the
            // log file is missing or unreadable (matches the write
            // path which is also best-effort).
            let fields = split_tab_fields(rest);
            let raw_distro = fields.first().copied().unwrap_or("");
            let limit: usize = fields
                .get(1)
                .and_then(|raw| raw.parse::<usize>().ok())
                .unwrap_or(200);
            let needle = if raw_distro.is_empty() || raw_distro == "*" {
                None
            } else {
                Some(raw_distro)
            };
            let lines = crate::log::read_tail(limit, needle);
            ok_lines(lines)
        }
        "SELF_CHECK" | "self_check" => ok_lines(runtime.self_check()),
        "CONSOLE_CANVAS" | "console_canvas" => {
            let Some(name) = first_tab_field(rest) else {
                return err_line(&AsldError::InvalidArgument("name"));
            };
            match crate::broker::console_canvas_snapshot(name) {
                Ok(lines) => ok_lines(lines),
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
        "SHELL_LIST" | "shell_list" => {
            let Some(name) = first_tab_field(rest) else {
                return err_line(&AsldError::InvalidArgument("name"));
            };
            match runtime.list_shell_sessions(name) {
                Ok(sessions) => ok_lines(format_shell_collection_lines(&sessions)),
                Err(err) => err_line(&err),
            }
        }
        "SHELL_SHOW" | "shell_show" => {
            let fields = split_tab_fields(rest);
            if fields.len() < 2 {
                return err_line(&AsldError::InvalidArgument("shell_show"));
            }
            match runtime.show_shell_session(fields[0], fields[1]) {
                Ok(session) => ok_lines(format_shell_lines(&session)),
                Err(err) => err_line(&err),
            }
        }
        "SHELL_CLOSE" | "shell_close" => {
            let fields = split_tab_fields(rest);
            if fields.len() < 2 {
                return err_line(&AsldError::InvalidArgument("shell_close"));
            }
            match runtime.close_shell_session(fields[0], fields[1]) {
                Ok(sessions) => ok_lines(format_shell_collection_lines(&sessions)),
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
        "EXEC_LIST" | "exec_list" => {
            let Some(name) = first_tab_field(rest) else {
                return err_line(&AsldError::InvalidArgument("name"));
            };
            match runtime.list_execs(name) {
                Ok(execs) => ok_lines(format_exec_collection_lines(&execs)),
                Err(err) => err_line(&err),
            }
        }
        "EXEC_SHOW" | "exec_show" => {
            let fields = split_tab_fields(rest);
            if fields.len() < 2 {
                return err_line(&AsldError::InvalidArgument("exec_show"));
            }
            match runtime.show_exec(fields[0], fields[1]) {
                Ok(exec) => ok_lines(format_exec_lines(&exec)),
                Err(err) => err_line(&err),
            }
        }
        "EXEC_CLEAR" | "exec_clear" => {
            let Some(name) = first_tab_field(rest) else {
                return err_line(&AsldError::InvalidArgument("name"));
            };
            match runtime.clear_execs(name) {
                Ok(count) => ok_lines(alloc::vec![format!("cleared\t{}", count)]),
                Err(err) => err_line(&err),
            }
        }
        "MOUNT_LIST" | "mount_list" => {
            let Some(name) = first_tab_field(rest) else {
                return err_line(&AsldError::InvalidArgument("name"));
            };
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
        "MOUNT_VALIDATE" | "mount_validate" => {
            let Some(name) = first_tab_field(rest) else {
                return err_line(&AsldError::InvalidArgument("name"));
            };
            match runtime.validate_mounts(store, name) {
                Ok(report) => ok_lines(format_mount_validation_lines(&report)),
                Err(err) => err_line(&err),
            }
        }
        "PORT_LIST" | "port_list" => {
            let Some(name) = first_tab_field(rest) else {
                return err_line(&AsldError::InvalidArgument("name"));
            };
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

fn parse_create_fields(s: &str) -> Vec<&str> {
    if s.contains('\t') {
        split_tab_fields(s)
    } else {
        s.split_whitespace().collect()
    }
}

fn agent_update<S: ConfigStore>(
    runtime: &mut RuntimeService,
    store: &mut S,
    rest: &str,
    next_state: AgentState,
    arg_name: &'static str,
) -> String {
    let fields = split_tab_fields(rest);
    let Some(name) = fields.first().copied() else {
        return err_line(&AsldError::InvalidArgument(arg_name));
    };
    match runtime.update_agent_state(store, name, next_state, fields.get(1).copied()) {
        Ok(lines) => ok_lines(lines),
        Err(err) => err_line(&err),
    }
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

fn parse_optional_u32(s: &str) -> Result<Option<u32>, AsldError> {
    if s == "-" || s.is_empty() {
        return Ok(None);
    }
    parse_u32(s)
        .map(Some)
        .ok_or(AsldError::InvalidArgument("u32"))
}

fn parse_optional_u16(s: &str) -> Result<Option<u16>, AsldError> {
    if s == "-" || s.is_empty() {
        return Ok(None);
    }
    parse_u16(s)
        .map(Some)
        .ok_or(AsldError::InvalidArgument("u16"))
}

fn parse_optional_bool(s: &str) -> Result<Option<bool>, AsldError> {
    match s {
        "-" | "" => Ok(None),
        "true" | "1" | "yes" | "on" => Ok(Some(true)),
        "false" | "0" | "no" | "off" => Ok(Some(false)),
        _ => Err(AsldError::InvalidArgument("bool")),
    }
}

fn optional_field(value: &str) -> Option<&str> {
    if value == "-" || value.is_empty() {
        None
    } else {
        Some(value)
    }
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

fn format_shell_collection_lines(sessions: &[ShellSession]) -> Vec<String> {
    let mut out = Vec::new();
    for session in sessions {
        out.push(format!(
            "{}\t{}\t{}\t{}\t{}\t{}",
            session.session_id,
            session.session_name,
            session.mode.as_str(),
            session.console_pipe_name,
            session.stdin_pipe_name,
            session.attached_pid
        ));
    }
    out
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

fn format_exec_collection_lines(execs: &[ExecInvocation]) -> Vec<String> {
    let mut out = Vec::new();
    for exec in execs {
        out.push(format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            exec.exec_id,
            exec.mode.as_str(),
            exec.cwd,
            exec.env_count,
            exec.command_line,
            exec.stdout_pipe_name,
            exec.stdin_pipe_name,
            exec.attached_pid
        ));
    }
    out
}

fn sanitize_line(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\n' | '\r' => out.push(' '),
            '\t' => out.push(' '),
            _ => out.push(ch),
        }
    }
    out
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

fn format_mount_validation_lines(report: &[MountValidation]) -> Vec<String> {
    let mut out = Vec::new();
    for item in report {
        out.push(format!(
            "{}\t{}\t{}\t{}",
            item.id,
            item.guest_path,
            if item.valid { "true" } else { "false" },
            item.message
        ));
    }
    out
}

fn format_network_policy_lines(policy: &NetworkPolicy) -> Vec<String> {
    alloc::vec![
        format!("network.mode\t{}", policy.mode),
        format!("network.dns_mode\t{}", policy.dns_mode),
        format!("network.allow_outbound\t{}", policy.allow_outbound),
    ]
}

fn format_network_validation_lines(report: &[NetworkValidation]) -> Vec<String> {
    let mut out = Vec::new();
    for item in report {
        out.push(format!(
            "{}\t{}\t{}\t{}\t{}",
            item.id,
            item.listen,
            if item.valid { "true" } else { "false" },
            item.exposure,
            item.message
        ));
    }
    out
}

fn format_storage_validation_lines(report: &[StorageValidation]) -> Vec<String> {
    let mut out = Vec::new();
    for item in report {
        out.push(format!(
            "{}\t{}\t{}\t{}",
            item.role,
            item.path,
            if item.valid { "true" } else { "false" },
            item.message
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
        let create = dispatch(
            &mut runtime,
            &mut store,
            "CREATE ubuntu-dev ubuntu-24.04-x86_64-v1 strati",
        );
        assert!(create.starts_with("OK"));
        let status = dispatch(&mut runtime, &mut store, "STATUS ubuntu-dev");
        assert!(status.contains("name\tubuntu-dev"));
    }

    #[test]
    fn config_update_and_delete_roundtrip() {
        let mut runtime = RuntimeService::new();
        let mut store = FakeStore::default();
        let _ = dispatch(
            &mut runtime,
            &mut store,
            "CREATE ubuntu-dev ubuntu-24.04-x86_64-v1 strati",
        );

        let config = dispatch(&mut runtime, &mut store, "CONFIG_SHOW ubuntu-dev");
        assert!(config.contains("resources.memory_mb\t2048"));

        let updated = dispatch(
            &mut runtime,
            &mut store,
            "CONFIG_SET_RESOURCES ubuntu-dev\t4096\t4",
        );
        assert!(updated.contains("resources.memory_mb\t4096"));
        assert!(updated.contains("resources.vcpu_count\t4"));

        let deleted = dispatch(&mut runtime, &mut store, "DELETE ubuntu-dev\t0");
        assert!(deleted.contains("deleted\tubuntu-dev"));
        let status = dispatch(&mut runtime, &mut store, "STATUS ubuntu-dev");
        assert!(status.starts_with("ERR\t"));
    }

    #[test]
    fn clone_export_and_storage_commands_roundtrip() {
        let mut runtime = RuntimeService::new();
        let mut store = FakeStore::default();
        let _ = dispatch(
            &mut runtime,
            &mut store,
            "CREATE ubuntu-dev ubuntu-24.04-x86_64-v1 strati",
        );

        let clone = dispatch(
            &mut runtime,
            &mut store,
            "CLONE ubuntu-dev\tubuntu-copy\tops\t1",
        );
        assert!(clone.contains("name\tubuntu-copy"));
        assert!(clone.contains("owner\tops"));

        let export = dispatch(&mut runtime, &mut store, "EXPORT ubuntu-copy");
        assert!(export.contains("format\tasl-export-v1"));

        let storage = dispatch(&mut runtime, &mut store, "STORAGE_VALIDATE ubuntu-copy");
        assert!(storage
            .contains("overlay\t/System/var/asl/distros/ubuntu-copy/images/overlay.img\ttrue"));

        let import_missing_arg = dispatch(&mut runtime, &mut store, "STORAGE_IMPORT ubuntu-copy");
        assert!(import_missing_arg.starts_with("ERR\tINVALID_ARGUMENT"));

        let import = dispatch(
            &mut runtime,
            &mut store,
            "STORAGE_IMPORT ubuntu-copy\t/Users/Shared/debian.raw",
        );
        assert!(import.starts_with("ERR\tNOT_IMPLEMENTED"));
    }

    #[test]
    fn network_commands_roundtrip() {
        let mut runtime = RuntimeService::new();
        let mut store = FakeStore::default();
        let _ = dispatch(
            &mut runtime,
            &mut store,
            "CREATE ubuntu-dev ubuntu-24.04-x86_64-v1 strati",
        );

        let show = dispatch(&mut runtime, &mut store, "NETWORK_SHOW ubuntu-dev");
        assert!(show.contains("network.mode\tnat"));
        assert!(show.contains("network.dns_mode\thost-broker"));

        let updated = dispatch(
            &mut runtime,
            &mut store,
            "NETWORK_SET ubuntu-dev\tnat\thost-broker\tfalse",
        );
        assert!(updated.contains("network.allow_outbound\tfalse"));

        let validate = dispatch(&mut runtime, &mut store, "NETWORK_VALIDATE ubuntu-dev");
        assert!(validate.contains("policy\t\ttrue\tnat"));
    }

    #[test]
    fn mount_commands_roundtrip() {
        let mut runtime = RuntimeService::new();
        let mut store = FakeStore::default();
        let _ = dispatch(
            &mut runtime,
            &mut store,
            "CREATE ubuntu-dev ubuntu-24.04-x86_64-v1 strati",
        );
        let add = dispatch(
            &mut runtime,
            &mut store,
            "MOUNT_ADD ubuntu-dev\tworkspace\t/Users/strati/work\t/mnt/work\treadwrite\trelaxed\thost-native\tinherit\tbest-effort\tWorkspace",
        );
        assert!(add.contains("workspace\t/Users/strati/work"));
        let show = dispatch(&mut runtime, &mut store, "MOUNT_SHOW ubuntu-dev\tworkspace");
        assert!(show.contains("/mnt/work"));
        let validate = dispatch(&mut runtime, &mut store, "MOUNT_VALIDATE ubuntu-dev");
        assert!(validate.contains("workspace\t/mnt/work\t"));
        let remove = dispatch(
            &mut runtime,
            &mut store,
            "MOUNT_REMOVE ubuntu-dev\tworkspace",
        );
        assert!(remove.starts_with("OK\t0"));
    }

    #[test]
    fn port_commands_roundtrip() {
        let mut runtime = RuntimeService::new();
        let mut store = FakeStore::default();
        let _ = dispatch(
            &mut runtime,
            &mut store,
            "CREATE ubuntu-dev ubuntu-24.04-x86_64-v1 strati",
        );
        let add = dispatch(
            &mut runtime,
            &mut store,
            "PORT_ADD ubuntu-dev\tweb\t127.0.0.1\t3000\t3000\ttcp\tWeb",
        );
        assert!(add.contains("web\t127.0.0.1\t3000\t3000\ttcp"));
        let list = dispatch(&mut runtime, &mut store, "PORT_LIST ubuntu-dev");
        assert!(list.contains("web\t127.0.0.1"));
        let validate = dispatch(&mut runtime, &mut store, "NETWORK_VALIDATE ubuntu-dev");
        assert!(validate.contains("web\t127.0.0.1:3000\ttrue\tlocal"));
        let remove = dispatch(&mut runtime, &mut store, "PORT_REMOVE ubuntu-dev\tweb");
        assert!(remove.starts_with("OK\t0"));
    }

    #[test]
    fn shell_and_exec_roundtrip() {
        let mut runtime = RuntimeService::new();
        let mut store = FakeStore::default();
        let _ = dispatch(
            &mut runtime,
            &mut store,
            "CREATE ubuntu-dev ubuntu-24.04-x86_64-v1 strati",
        );
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
        let _ = dispatch(
            &mut runtime,
            &mut store,
            "CREATE ubuntu-dev ubuntu-24.04-x86_64-v1 strati",
        );
        let _ = dispatch(&mut runtime, &mut store, "START ubuntu-dev");
        let status = dispatch(&mut runtime, &mut store, "VM_STATUS ubuntu-dev");
        assert!(status.contains("backend\t"));
        assert!(status.contains("boot_summary\t"));
        let events = dispatch(&mut runtime, &mut store, "VM_EVENTS ubuntu-dev");
        assert!(events.starts_with("OK\t0"));
        let agent = dispatch(&mut runtime, &mut store, "AGENT_RESTART ubuntu-dev");
        assert!(agent.contains("restart\trequested"));
        let agent_status = dispatch(&mut runtime, &mut store, "AGENT_STATUS ubuntu-dev");
        assert!(agent_status.contains("agent\tstarting"));
        let hello = dispatch(&mut runtime, &mut store, "AGENT_HELLO ubuntu-dev\t1");
        assert!(hello.contains("agent\tconnected"));
        assert!(hello.contains("version\t1"));
        let ready = dispatch(&mut runtime, &mut store, "AGENT_READY ubuntu-dev\t1");
        assert!(ready.contains("agent\tready"));
        let disconnected = dispatch(&mut runtime, &mut store, "AGENT_DISCONNECT ubuntu-dev\t1");
        assert!(disconnected.contains("agent\tdisconnected"));
    }

    #[test]
    fn shell_exec_and_diagnose_roundtrip() {
        let mut runtime = RuntimeService::new();
        let mut store = FakeStore::default();
        let _ = dispatch(
            &mut runtime,
            &mut store,
            "CREATE ubuntu-dev ubuntu-24.04-x86_64-v1 strati",
        );
        let _ = dispatch(&mut runtime, &mut store, "START ubuntu-dev");
        let shell = dispatch(&mut runtime, &mut store, "SHELL_OPEN ubuntu-dev\tops\t0");
        assert!(shell.contains("sh-"));
        let shells = dispatch(&mut runtime, &mut store, "SHELL_LIST ubuntu-dev");
        assert!(shells.contains("ops"));
        let shell_id = shells
            .lines()
            .nth(1)
            .and_then(|line| line.split('\t').next())
            .unwrap_or("");
        let close = dispatch(
            &mut runtime,
            &mut store,
            &format!("SHELL_CLOSE ubuntu-dev\t{}", shell_id),
        );
        assert!(close.starts_with("OK\t0"));

        let _ = dispatch(
            &mut runtime,
            &mut store,
            "EXEC ubuntu-dev\t0\t/workspace\tRUST_BACKTRACE=1\tcargo\x1ftest",
        );
        let execs = dispatch(&mut runtime, &mut store, "EXEC_LIST ubuntu-dev");
        assert!(execs.contains("cargo test"));

        let diagnose = dispatch(&mut runtime, &mut store, "DIAGNOSE ubuntu-dev");
        assert!(diagnose.contains("vm_backend\t"));
        let cleared = dispatch(&mut runtime, &mut store, "VM_EVENTS_CLEAR ubuntu-dev");
        assert!(cleared.contains("cleared\t0"));
    }

    #[test]
    fn restart_show_and_clear_roundtrip() {
        let mut runtime = RuntimeService::new();
        let mut store = FakeStore::default();
        let _ = dispatch(
            &mut runtime,
            &mut store,
            "CREATE ubuntu-dev ubuntu-24.04-x86_64-v1 strati",
        );
        let _ = dispatch(&mut runtime, &mut store, "START ubuntu-dev");
        let restart = dispatch(&mut runtime, &mut store, "RESTART ubuntu-dev");
        assert!(restart.contains("state\tready"));

        let shell = dispatch(&mut runtime, &mut store, "SHELL_OPEN ubuntu-dev\tops\t0");
        let shell_id = shell
            .lines()
            .nth(1)
            .and_then(|line| line.split('\t').next())
            .unwrap_or("");
        let shell_show = dispatch(
            &mut runtime,
            &mut store,
            &format!("SHELL_SHOW ubuntu-dev\t{}", shell_id),
        );
        assert!(shell_show.contains("\tops\t"));

        let exec = dispatch(
            &mut runtime,
            &mut store,
            "EXEC ubuntu-dev\t0\t/workspace\tRUST_BACKTRACE=1\tcargo\x1ftest",
        );
        let exec_id = exec
            .lines()
            .nth(1)
            .and_then(|line| line.split('\t').next())
            .unwrap_or("");
        let exec_show = dispatch(
            &mut runtime,
            &mut store,
            &format!("EXEC_SHOW ubuntu-dev\t{}", exec_id),
        );
        assert!(exec_show.contains("\tcargo test\t"));

        let exec_clear = dispatch(&mut runtime, &mut store, "EXEC_CLEAR ubuntu-dev");
        assert!(exec_clear.contains("cleared\t1"));
        let events_tail = dispatch(&mut runtime, &mut store, "VM_EVENTS_TAIL ubuntu-dev\t5");
        assert!(events_tail.starts_with("OK\t0"));
    }

    // ---- LOGS dispatcher tests ------------------------------------------
    //
    // The pure tail+filter logic is exercised by `log::tests` with
    // synthetic buffers. Here we pin the wire-level contract:
    // dispatcher routes correctly, returns OK, accepts the documented
    // argument shapes. On Linux/host builds `read_tail` returns empty
    // (no on-disk log), so we assert OK+0 lines rather than content.

    #[test]
    fn logs_command_returns_ok_with_no_args() {
        let mut runtime = RuntimeService::new();
        let mut store = FakeStore::default();
        let resp = dispatch(&mut runtime, &mut store, "LOGS");
        assert!(resp.starts_with("OK\t"));
    }

    #[test]
    fn logs_command_accepts_distro_filter() {
        let mut runtime = RuntimeService::new();
        let mut store = FakeStore::default();
        let resp = dispatch(&mut runtime, &mut store, "LOGS ubuntu-dev\t50");
        assert!(resp.starts_with("OK\t"));
    }

    #[test]
    fn logs_command_accepts_wildcard_distro() {
        let mut runtime = RuntimeService::new();
        let mut store = FakeStore::default();
        let resp = dispatch(&mut runtime, &mut store, "LOGS *\t10");
        assert!(resp.starts_with("OK\t"));
    }

    #[test]
    fn logs_command_lowercase_alias_works() {
        // Wire commands are case-insensitive across asld; pin the
        // lowercase form so future refactors don't quietly drop it.
        let mut runtime = RuntimeService::new();
        let mut store = FakeStore::default();
        let resp = dispatch(&mut runtime, &mut store, "logs *\t5");
        assert!(resp.starts_with("OK\t"));
    }

    #[test]
    fn logs_command_garbage_limit_falls_back_to_default() {
        // ADR-0010 invariant: bad operator input must not crash the
        // daemon. Garbage limit -> default cap, still OK.
        let mut runtime = RuntimeService::new();
        let mut store = FakeStore::default();
        let resp = dispatch(&mut runtime, &mut store, "LOGS *\tnot-a-number");
        assert!(resp.starts_with("OK\t"));
    }

    // ---- DOCTOR dispatcher tests ----------------------------------------

    #[test]
    fn doctor_command_returns_check_lines_and_summary() {
        let mut runtime = RuntimeService::new();
        let mut store = FakeStore::default();
        let _ = dispatch(
            &mut runtime,
            &mut store,
            "CREATE ubuntu-dev ubuntu-24.04-x86_64-v1 strati",
        );
        let resp = dispatch(&mut runtime, &mut store, "DOCTOR ubuntu-dev");
        assert!(resp.starts_with("OK\t"));
        // All canonical subsystems present.
        for sub in ["storage", "boot", "network", "mounts", "agent", "vm"] {
            assert!(
                resp.contains(&alloc::format!("check\t{sub}\t")),
                "missing check for {sub}: {resp:?}"
            );
        }
        // Summary present.
        assert!(resp.contains("summary\t"));
    }

    #[test]
    fn doctor_command_lowercase_alias_works() {
        let mut runtime = RuntimeService::new();
        let mut store = FakeStore::default();
        let _ = dispatch(
            &mut runtime,
            &mut store,
            "CREATE ubuntu-dev ubuntu-24.04-x86_64-v1 strati",
        );
        let resp = dispatch(&mut runtime, &mut store, "doctor ubuntu-dev");
        assert!(resp.starts_with("OK\t"));
    }

    #[test]
    fn doctor_command_self_route() {
        let mut runtime = RuntimeService::new();
        let mut store = FakeStore::default();
        let resp = dispatch(&mut runtime, &mut store, "DOCTOR self");
        assert!(resp.starts_with("OK\t"));
        assert!(resp.contains("summary\t"));
    }

    #[test]
    fn doctor_command_missing_distro_yields_err() {
        let mut runtime = RuntimeService::new();
        let mut store = FakeStore::default();
        let resp = dispatch(&mut runtime, &mut store, "DOCTOR ");
        assert!(resp.starts_with("ERR"), "got: {resp:?}");
    }

    #[test]
    fn doctor_command_unknown_distro_yields_err() {
        let mut runtime = RuntimeService::new();
        let mut store = FakeStore::default();
        let resp = dispatch(&mut runtime, &mut store, "DOCTOR nonexistent");
        assert!(resp.starts_with("ERR"));
    }
}
