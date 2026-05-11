#![cfg_attr(not(target_os = "linux"), no_std)]

use anyos_std::{format, println, process, String, Vec};
#[cfg(not(target_os = "linux"))]
use anyos_std::{fs, ipc, sys};

const ASLD_PIPE: &str = "asld";
const RESPONSE_TIMEOUT_TICKS: u32 = 100;
const RESPONSE_SLEEP_MS: u32 = 20;

pub enum ClientCommand<'a> {
    List,
    Status(&'a str),
    Create {
        name: &'a str,
        image_ref: &'a str,
        owner: &'a str,
        kernel_profile: Option<&'a str>,
    },
    Delete {
        name: &'a str,
        force: bool,
    },
    Clone {
        source: &'a str,
        target: &'a str,
        owner: Option<&'a str>,
        include_mounts: bool,
    },
    Export(&'a str),
    Config(&'a str),
    ConfigSetResources {
        distro: &'a str,
        memory_mb: Option<&'a str>,
        vcpu_count: Option<&'a str>,
    },
    StorageImport {
        distro: &'a str,
        source_path: &'a str,
    },
    StorageValidate(&'a str),
    NetworkShow(&'a str),
    NetworkSet {
        distro: &'a str,
        mode: Option<&'a str>,
        dns_mode: Option<&'a str>,
        allow_outbound: Option<&'a str>,
    },
    NetworkValidate(&'a str),
    Start(&'a str),
    Restart(&'a str),
    Stop(&'a str),
    AgentStatus(&'a str),
    AgentRestart(&'a str),
    VmStatus(&'a str),
    VmEvents(&'a str),
    VmEventsTail {
        distro: &'a str,
        limit: &'a str,
    },
    VmEventsClear(&'a str),
    Diagnose(&'a str),
    Doctor(&'a str),
    Logs {
        /// Distro filter; "*" or empty means full asld log.
        distro: &'a str,
        /// Tail size; 0 means "default" on the daemon side (200).
        limit: u32,
    },
    SelfCheck,
    ConsoleCanvas(&'a str),
    ShellList(&'a str),
    ShellShow {
        distro: &'a str,
        session_id: &'a str,
    },
    ShellClose {
        distro: &'a str,
        session_id: &'a str,
    },
    Shell {
        distro: &'a str,
        fallback_console: bool,
        session_name: &'a str,
    },
    ExecList(&'a str),
    ExecShow {
        distro: &'a str,
        exec_id: &'a str,
    },
    ExecClear(&'a str),
    Exec {
        distro: &'a str,
        fallback_console: bool,
        cwd: &'a str,
        env: Vec<&'a str>,
        argv: Vec<&'a str>,
    },
    MountList(&'a str),
    MountShow {
        distro: &'a str,
        mount_id: &'a str,
    },
    MountAdd {
        distro: &'a str,
        mount_id: &'a str,
        host_path: &'a str,
        guest_path: &'a str,
        mode: &'a str,
        metadata_mode: &'a str,
        case_mode: &'a str,
        exec_policy: &'a str,
        watch_policy: &'a str,
        description: &'a str,
    },
    MountRemove {
        distro: &'a str,
        mount_id: &'a str,
    },
    MountValidate(&'a str),
    PortList(&'a str),
    PortAdd {
        distro: &'a str,
        rule_id: &'a str,
        listen_address: &'a str,
        listen_port: &'a str,
        guest_port: &'a str,
        protocol: &'a str,
        description: &'a str,
    },
    PortRemove {
        distro: &'a str,
        rule_id: &'a str,
    },
    PortValidate(&'a str),
}

pub enum WireResponse {
    Ok { count: usize, lines: Vec<String> },
    Err { code: String, message: String },
}

/// Output format selected by the global `--json` flag. Default is the
/// human-readable text renderer that has been the only mode prior to
/// 2026-05-07; `Json` emits one JSON object per response so callers
/// (aslmanager, scripts, monitoring agents) get a stable machine
/// surface that does not depend on parsing decorated text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputFormat {
    Text,
    Json,
}

/// Verbosity selected by the global `--quiet` / `--verbose` flags.
/// `Quiet` suppresses the per-section header lines (`distros: N`,
/// `mounts: N`, ...) — useful for piping into grep or for scripted
/// callers in `--text` mode that don't want the framing. `Verbose`
/// is reserved for future per-command extra detail; today it behaves
/// like `Normal`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verbosity {
    Quiet,
    Normal,
    Verbose,
}

/// Parsed global flags + the residual argv-style string with the
/// flags stripped. The stripped string is then handed to
/// `parse_cli_command` exactly as before, so existing subcommand
/// parsers stay untouched.
#[derive(Debug, Clone)]
pub struct GlobalFlags {
    pub format: OutputFormat,
    pub verbosity: Verbosity,
}

impl GlobalFlags {
    pub fn defaults() -> Self {
        Self {
            format: OutputFormat::Text,
            verbosity: Verbosity::Normal,
        }
    }
}

/// Strip leading global flags from `raw` and return the parsed flags
/// plus the remainder string. Recognised flags:
///
/// - `--json`             switch to JSON output
/// - `--quiet` / `-q`     drop framing/header lines
/// - `--verbose` / `-v`   reserved (no-op today, pinned for forward-compat)
///
/// Flags must come **before** the subcommand. This matches the
/// convention documented in `docs/aslctl-cli.md`. Unknown leading
/// flags are passed through unchanged so subcommand-specific flags
/// (e.g. `--cwd`) are not stolen by the global parser.
pub fn strip_global_flags(raw: &str) -> (GlobalFlags, String) {
    let mut flags = GlobalFlags::defaults();
    let mut tokens: Vec<&str> = raw.split_whitespace().collect();
    while let Some(first) = tokens.first().copied() {
        match first {
            "--json" => {
                flags.format = OutputFormat::Json;
                tokens.remove(0);
            }
            "--quiet" | "-q" => {
                flags.verbosity = Verbosity::Quiet;
                tokens.remove(0);
            }
            "--verbose" | "-v" => {
                flags.verbosity = Verbosity::Verbose;
                tokens.remove(0);
            }
            _ => break,
        }
    }
    // `join` returns String; no further conversion needed.
    (flags, tokens.join(" "))
}

pub fn run() {
    let mut args_buf = [0u8; 256];
    let raw = process::args(&mut args_buf);
    let (flags, remainder) = strip_global_flags(raw);
    let Some(command) = parse_cli_command(&remainder) else {
        print_usage();
        return;
    };

    match send_command(&command) {
        Ok(response) => print_response_with(&command, &response, flags.format, flags.verbosity),
        Err(message) => match flags.format {
            OutputFormat::Json => {
                // Keep the error shape identical to a daemon ERR so
                // callers have one parser to write.
                println!(
                    "{}",
                    json_error_response(&command, "transport_error", &message)
                );
            }
            OutputFormat::Text => println!("aslctl: {}", message),
        },
    }
}

pub fn parse_cli_command<'a>(raw: &'a str) -> Option<ClientCommand<'a>> {
    let tokens = tokenize(raw);
    let first = *tokens.first()?;
    match first {
        "create" => parse_create_tokens(&tokens),
        "delete" => parse_delete_tokens(&tokens),
        "clone" => parse_clone_tokens(&tokens),
        "export" => Some(ClientCommand::Export(*tokens.get(1)?)),
        "config" => parse_config_tokens(&tokens),
        "network" => parse_network_tokens(&tokens),
        "doctor" => Some(ClientCommand::Doctor(*tokens.get(1)?)),
        "self-check" | "service-check" => Some(ClientCommand::SelfCheck),
        "console-canvas" | "canvas" => Some(ClientCommand::ConsoleCanvas(*tokens.get(1)?)),
        "shell-list" => Some(ClientCommand::ShellList(*tokens.get(1)?)),
        "shell-show" => Some(ClientCommand::ShellShow {
            distro: *tokens.get(1)?,
            session_id: *tokens.get(2)?,
        }),
        "shell-close" => Some(ClientCommand::ShellClose {
            distro: *tokens.get(1)?,
            session_id: *tokens.get(2)?,
        }),
        "shell" => parse_shell_tokens(&tokens),
        "exec-list" => Some(ClientCommand::ExecList(*tokens.get(1)?)),
        "exec-show" => Some(ClientCommand::ExecShow {
            distro: *tokens.get(1)?,
            exec_id: *tokens.get(2)?,
        }),
        "exec" | "run" => parse_exec_tokens(&tokens),
        _ => {
            let args = anyos_std::args::parse(raw, b"");
            parse_command(&args)
        }
    }
}

pub fn parse_command<'a>(args: &anyos_std::args::ParsedArgs<'a>) -> Option<ClientCommand<'a>> {
    let cmd = args.pos(0)?;
    match cmd {
        "list" => Some(ClientCommand::List),
        "status" => Some(ClientCommand::Status(args.pos(1)?)),
        "create" => Some(ClientCommand::Create {
            name: args.pos(1)?,
            image_ref: args.pos(2)?,
            owner: args.pos(3)?,
            kernel_profile: None,
        }),
        "delete" => Some(ClientCommand::Delete {
            name: args.pos(1)?,
            force: false,
        }),
        "clone" => Some(ClientCommand::Clone {
            source: args.pos(1)?,
            target: args.pos(2)?,
            owner: args.pos(3),
            include_mounts: true,
        }),
        "export" => Some(ClientCommand::Export(args.pos(1)?)),
        "config" => Some(ClientCommand::Config(args.pos(1)?)),
        "storage" => parse_storage_command(args),
        "network" => parse_network_command(args),
        "agent" => parse_agent_command(args),
        "start" => Some(ClientCommand::Start(args.pos(1)?)),
        "restart" => Some(ClientCommand::Restart(args.pos(1)?)),
        "stop" => Some(ClientCommand::Stop(args.pos(1)?)),
        "agent-status" => Some(ClientCommand::AgentStatus(args.pos(1)?)),
        "agent-restart" => Some(ClientCommand::AgentRestart(args.pos(1)?)),
        "vm-status" => Some(ClientCommand::VmStatus(args.pos(1)?)),
        // `events` is the spec-aligned name (asl-control-plane-api.md
        // Op:ListEvents). `vm-events` and friends remain as backwards-
        // compatible aliases — they all hit the same VM_EVENTS wire
        // command on asld.
        "vm-events" | "events" => Some(ClientCommand::VmEvents(args.pos(1)?)),
        "vm-events-tail" | "events-tail" => Some(ClientCommand::VmEventsTail {
            distro: args.pos(1)?,
            limit: args.pos(2).unwrap_or("10"),
        }),
        "vm-events-clear" | "events-clear" => Some(ClientCommand::VmEventsClear(args.pos(1)?)),
        "logs" => {
            // `aslctl logs [<distro>] [<limit>]`
            //   distro defaults to "*" (no filter)
            //   limit  defaults to 200 (matches asld LOGS default)
            let distro = args.pos(1).unwrap_or("*");
            let limit = args
                .pos(2)
                .and_then(|raw| raw.parse::<u32>().ok())
                .unwrap_or(200);
            Some(ClientCommand::Logs { distro, limit })
        }
        "diagnose" => Some(ClientCommand::Diagnose(args.pos(1)?)),
        "doctor" => Some(ClientCommand::Doctor(args.pos(1)?)),
        "self-check" | "service-check" => Some(ClientCommand::SelfCheck),
        "console-canvas" | "canvas" => Some(ClientCommand::ConsoleCanvas(args.pos(1)?)),
        "exec-clear" => Some(ClientCommand::ExecClear(args.pos(1)?)),
        "mount" => parse_mount_command(args),
        "port" => parse_port_command(args),
        _ => None,
    }
}

fn parse_create_tokens<'a>(tokens: &[&'a str]) -> Option<ClientCommand<'a>> {
    let name = *tokens.get(1)?;
    let image_ref = *tokens.get(2)?;
    let owner = *tokens.get(3)?;
    let mut kernel_profile = None;
    let mut i = 4;
    while i < tokens.len() {
        match tokens[i] {
            "--kernel-profile" => {
                kernel_profile = Some(*tokens.get(i + 1)?);
                i += 1;
            }
            _ => return None,
        }
        i += 1;
    }
    Some(ClientCommand::Create {
        name,
        image_ref,
        owner,
        kernel_profile,
    })
}

fn parse_shell_tokens<'a>(tokens: &[&'a str]) -> Option<ClientCommand<'a>> {
    let distro = *tokens.get(1)?;
    let mut fallback_console = false;
    let mut session_name = "";
    let mut i = 2;
    while i < tokens.len() {
        match tokens[i] {
            "--fallback-console" => fallback_console = true,
            "--session" => {
                session_name = *tokens.get(i + 1)?;
                i += 1;
            }
            _ => return None,
        }
        i += 1;
    }
    Some(ClientCommand::Shell {
        distro,
        fallback_console,
        session_name,
    })
}

fn parse_exec_tokens<'a>(tokens: &[&'a str]) -> Option<ClientCommand<'a>> {
    let distro = *tokens.get(1)?;
    let mut fallback_console = false;
    let mut cwd = "";
    let mut env = Vec::new();
    let mut argv = Vec::new();
    let mut passthrough = false;
    let mut i = 2;
    while i < tokens.len() {
        let token = tokens[i];
        if passthrough {
            argv.push(token);
            i += 1;
            continue;
        }
        match token {
            "--fallback-console" => fallback_console = true,
            "--cwd" => {
                cwd = *tokens.get(i + 1)?;
                i += 1;
            }
            "--env" => {
                env.push(*tokens.get(i + 1)?);
                i += 1;
            }
            "--" => passthrough = true,
            _ => return None,
        }
        i += 1;
    }
    if argv.is_empty() {
        return None;
    }
    Some(ClientCommand::Exec {
        distro,
        fallback_console,
        cwd,
        env,
        argv,
    })
}

fn parse_mount_command<'a>(args: &anyos_std::args::ParsedArgs<'a>) -> Option<ClientCommand<'a>> {
    match args.pos(1)? {
        "list" => Some(ClientCommand::MountList(args.pos(2)?)),
        "validate" => Some(ClientCommand::MountValidate(args.pos(2)?)),
        "show" => Some(ClientCommand::MountShow {
            distro: args.pos(2)?,
            mount_id: args.pos(3)?,
        }),
        "add" => Some(ClientCommand::MountAdd {
            distro: args.pos(2)?,
            mount_id: args.pos(3)?,
            host_path: args.pos(4)?,
            guest_path: args.pos(5)?,
            mode: args.pos(6)?,
            metadata_mode: args.pos(7)?,
            case_mode: args.pos(8)?,
            exec_policy: args.pos(9)?,
            watch_policy: args.pos(10)?,
            description: args.pos(11).unwrap_or(""),
        }),
        "remove" => Some(ClientCommand::MountRemove {
            distro: args.pos(2)?,
            mount_id: args.pos(3)?,
        }),
        _ => None,
    }
}

fn parse_network_command<'a>(args: &anyos_std::args::ParsedArgs<'a>) -> Option<ClientCommand<'a>> {
    match args.pos(1)? {
        "show" => Some(ClientCommand::NetworkShow(args.pos(2)?)),
        "validate" => Some(ClientCommand::NetworkValidate(args.pos(2)?)),
        _ => None,
    }
}

fn parse_agent_command<'a>(args: &anyos_std::args::ParsedArgs<'a>) -> Option<ClientCommand<'a>> {
    match args.pos(1)? {
        "status" => Some(ClientCommand::AgentStatus(args.pos(2)?)),
        "restart" => Some(ClientCommand::AgentRestart(args.pos(2)?)),
        _ => None,
    }
}

fn parse_storage_command<'a>(args: &anyos_std::args::ParsedArgs<'a>) -> Option<ClientCommand<'a>> {
    match args.pos(1)? {
        "import" => Some(ClientCommand::StorageImport {
            distro: args.pos(2)?,
            source_path: args.pos(3)?,
        }),
        "validate" => Some(ClientCommand::StorageValidate(args.pos(2)?)),
        _ => None,
    }
}

fn parse_delete_tokens<'a>(tokens: &[&'a str]) -> Option<ClientCommand<'a>> {
    let name = *tokens.get(1)?;
    let mut force = false;
    let mut i = 2;
    while i < tokens.len() {
        match tokens[i] {
            "--force" | "-f" => force = true,
            _ => return None,
        }
        i += 1;
    }
    Some(ClientCommand::Delete { name, force })
}

fn parse_clone_tokens<'a>(tokens: &[&'a str]) -> Option<ClientCommand<'a>> {
    let source = *tokens.get(1)?;
    let target = *tokens.get(2)?;
    let mut owner = None;
    let mut include_mounts = true;
    let mut i = 3;
    while i < tokens.len() {
        match tokens[i] {
            "--owner" => {
                owner = Some(*tokens.get(i + 1)?);
                i += 1;
            }
            "--no-mounts" => include_mounts = false,
            _ => return None,
        }
        i += 1;
    }
    Some(ClientCommand::Clone {
        source,
        target,
        owner,
        include_mounts,
    })
}

fn parse_network_tokens<'a>(tokens: &[&'a str]) -> Option<ClientCommand<'a>> {
    match *tokens.get(1)? {
        "show" => Some(ClientCommand::NetworkShow(*tokens.get(2)?)),
        "validate" => Some(ClientCommand::NetworkValidate(*tokens.get(2)?)),
        "set" => {
            let distro = *tokens.get(2)?;
            let mut mode = None;
            let mut dns_mode = None;
            let mut allow_outbound = None;
            let mut i = 3;
            while i < tokens.len() {
                match tokens[i] {
                    "--mode" => {
                        mode = Some(*tokens.get(i + 1)?);
                        i += 1;
                    }
                    "--dns-mode" => {
                        dns_mode = Some(*tokens.get(i + 1)?);
                        i += 1;
                    }
                    "--allow-outbound" => {
                        allow_outbound = Some(*tokens.get(i + 1)?);
                        i += 1;
                    }
                    _ => return None,
                }
                i += 1;
            }
            Some(ClientCommand::NetworkSet {
                distro,
                mode,
                dns_mode,
                allow_outbound,
            })
        }
        _ => None,
    }
}

fn parse_config_tokens<'a>(tokens: &[&'a str]) -> Option<ClientCommand<'a>> {
    match *tokens.get(1)? {
        "set-resources" => {
            let distro = *tokens.get(2)?;
            let mut memory_mb = None;
            let mut vcpu_count = None;
            let mut i = 3;
            while i < tokens.len() {
                match tokens[i] {
                    "--memory" | "--memory-mb" => {
                        memory_mb = Some(*tokens.get(i + 1)?);
                        i += 1;
                    }
                    "--vcpus" | "--vcpu-count" => {
                        vcpu_count = Some(*tokens.get(i + 1)?);
                        i += 1;
                    }
                    _ => return None,
                }
                i += 1;
            }
            Some(ClientCommand::ConfigSetResources {
                distro,
                memory_mb,
                vcpu_count,
            })
        }
        name => {
            if tokens.len() == 2 {
                Some(ClientCommand::Config(name))
            } else {
                None
            }
        }
    }
}

fn parse_port_command<'a>(args: &anyos_std::args::ParsedArgs<'a>) -> Option<ClientCommand<'a>> {
    match args.pos(1)? {
        "list" => Some(ClientCommand::PortList(args.pos(2)?)),
        "validate" => Some(ClientCommand::PortValidate(args.pos(2)?)),
        "add" => Some(ClientCommand::PortAdd {
            distro: args.pos(2)?,
            rule_id: args.pos(3)?,
            listen_address: args.pos(4)?,
            listen_port: args.pos(5)?,
            guest_port: args.pos(6)?,
            protocol: args.pos(7)?,
            description: args.pos(8).unwrap_or(""),
        }),
        "remove" => Some(ClientCommand::PortRemove {
            distro: args.pos(2)?,
            rule_id: args.pos(3)?,
        }),
        _ => None,
    }
}

fn send_command(command: &ClientCommand<'_>) -> Result<WireResponse, &'static str> {
    let pid = process::getpid();
    let reply_name = format!("asld-{}", pid);

    let old_reply = anyos_std::ipc::pipe_open(&reply_name);
    if old_reply != 0 {
        let _ = anyos_std::ipc::pipe_close(old_reply);
    }

    let reply_pipe = anyos_std::ipc::pipe_create(&reply_name);
    if reply_pipe == 0 || reply_pipe == u32::MAX {
        return Err("failed to create reply pipe");
    }

    let request_pipe = anyos_std::ipc::pipe_open(ASLD_PIPE);
    if request_pipe == 0 || request_pipe == u32::MAX {
        let _ = anyos_std::ipc::pipe_close(reply_pipe);
        return Err("asld is not running");
    }

    let request = format!("{}\t{}\n", pid, command.as_wire());
    if anyos_std::ipc::pipe_write(request_pipe, request.as_bytes()) == u32::MAX {
        let _ = anyos_std::ipc::pipe_close(reply_pipe);
        return Err("failed to write request");
    }

    let mut raw = String::new();
    let mut buf = [0u8; 1024];
    let mut ticks = 0;
    while ticks < RESPONSE_TIMEOUT_TICKS {
        let n = anyos_std::ipc::pipe_read(reply_pipe, &mut buf);
        if n == u32::MAX {
            let _ = anyos_std::ipc::pipe_close(reply_pipe);
            return Err("failed to read response");
        }
        if n > 0 {
            let chunk = core::str::from_utf8(&buf[..n as usize])
                .map_err(|_| "response was not valid UTF-8")?;
            raw.push_str(chunk);
            if raw.ends_with("\n\n") {
                let _ = anyos_std::ipc::pipe_close(reply_pipe);
                return parse_response(&raw);
            }
        }
        process::sleep(RESPONSE_SLEEP_MS);
        ticks += 1;
    }

    let _ = anyos_std::ipc::pipe_close(reply_pipe);
    Err("timed out waiting for asld response")
}

pub fn parse_response(raw: &str) -> Result<WireResponse, &'static str> {
    let trimmed = raw.trim_matches('\n');
    let mut lines = trimmed.split('\n');
    let header = lines.next().ok_or("empty response")?;
    let mut header_parts = header.split('\t');
    match header_parts.next() {
        Some("OK") => {
            let count =
                parse_usize(header_parts.next().unwrap_or("0")).ok_or("invalid OK header")?;
            let mut body = Vec::new();
            for line in lines {
                if !line.is_empty() {
                    body.push(String::from(line));
                }
            }
            Ok(WireResponse::Ok { count, lines: body })
        }
        Some("ERR") => {
            let code = String::from(header_parts.next().unwrap_or("unknown"));
            let message = join_tab_fields(&mut header_parts);
            Ok(WireResponse::Err { code, message })
        }
        _ => Err("unknown response header"),
    }
}

fn print_response(command: &ClientCommand<'_>, response: &WireResponse) {
    print_response_with(command, response, OutputFormat::Text, Verbosity::Normal);
}

/// Print a section header (e.g. "distros: 5") only when the verbosity
/// level allows it. In `--quiet` mode header lines are suppressed so
/// scripts can pipe the body lines straight into grep without having
/// to skip the count line.
fn print_header(verbosity: Verbosity, label: &str, count: usize) {
    if !matches!(verbosity, Verbosity::Quiet) {
        println!("{}: {}", label, count);
    }
}

fn print_response_with(
    command: &ClientCommand<'_>,
    response: &WireResponse,
    format: OutputFormat,
    verbosity: Verbosity,
) {
    if matches!(format, OutputFormat::Json) {
        // JSON mode bypasses the per-command renderer entirely — one
        // line, one stable shape, machine-readable. Verbosity is not
        // applied because JSON consumers don't want a different
        // schema based on a flag.
        println!("{}", format_json_response(command, response));
        return;
    }
    match response {
        WireResponse::Err { code, message } => {
            if message.is_empty() {
                println!("ERR {}", code);
            } else {
                println!("ERR {} {}", code, message);
            }
        }
        WireResponse::Ok { count, lines } => match command {
            ClientCommand::List => {
                print_header(verbosity, "distros", *count);
                for line in lines {
                    let mut parts = line.split('\t');
                    let name = parts.next().unwrap_or("-");
                    let state = parts.next().unwrap_or("-");
                    let health = parts.next().unwrap_or("-");
                    println!("{}\t{}\t{}", name, state, health);
                }
            }
            ClientCommand::MountList(_)
            | ClientCommand::MountShow { .. }
            | ClientCommand::MountAdd { .. }
            | ClientCommand::MountRemove { .. } => {
                print_header(verbosity, "mounts", *count);
                for line in lines {
                    let mut parts = line.split('\t');
                    println!(
                        "{}\thost={}\tguest={}\tmode={}\tmetadata={}\tcase={}\texec={}\twatch={}\tdesc={}",
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or(""),
                    );
                }
            }
            ClientCommand::MountValidate(_) => {
                print_header(verbosity, "mount-validation", *count);
                for line in lines {
                    let mut parts = line.split('\t');
                    println!(
                        "{}\tguest={}\tvalid={}\t{}",
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("false"),
                        parts.next().unwrap_or(""),
                    );
                }
            }
            ClientCommand::NetworkValidate(_) | ClientCommand::PortValidate(_) => {
                print_header(verbosity, "network-validation", *count);
                for line in lines {
                    let mut parts = line.split('\t');
                    println!(
                        "{}\tlisten={}\tvalid={}\texposure={}\t{}",
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("false"),
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or(""),
                    );
                }
            }
            ClientCommand::StorageValidate(_) => {
                print_header(verbosity, "storage-validation", *count);
                for line in lines {
                    let mut parts = line.split('\t');
                    println!(
                        "{}\tpath={}\tvalid={}\t{}",
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("false"),
                        parts.next().unwrap_or(""),
                    );
                }
            }
            ClientCommand::PortList(_)
            | ClientCommand::PortAdd { .. }
            | ClientCommand::PortRemove { .. } => {
                print_header(verbosity, "ports", *count);
                for line in lines {
                    let mut parts = line.split('\t');
                    println!(
                        "{}\tlisten={}:{}\tguest={}\tproto={}\tdesc={}",
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or(""),
                    );
                }
            }
            ClientCommand::Status(_)
            | ClientCommand::Create { .. }
            | ClientCommand::Delete { .. }
            | ClientCommand::Clone { .. }
            | ClientCommand::Export(_)
            | ClientCommand::Config(_)
            | ClientCommand::ConfigSetResources { .. }
            | ClientCommand::StorageImport { .. }
            | ClientCommand::NetworkShow(_)
            | ClientCommand::NetworkSet { .. }
            | ClientCommand::Start(_)
            | ClientCommand::Restart(_)
            | ClientCommand::Stop(_)
            | ClientCommand::AgentStatus(_)
            | ClientCommand::AgentRestart(_)
            | ClientCommand::VmStatus(_) => {
                for line in lines {
                    println!("{}", line);
                }
            }
            ClientCommand::VmEvents(_) | ClientCommand::VmEventsTail { .. } => {
                print_header(verbosity, "vm-events", *count);
                for line in lines {
                    let mut parts = line.split('\t');
                    println!(
                        "{}\treason={}\tlevel={}\tsummary={}\tqual={}\tgpa={}\tgva={}",
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("-"),
                    );
                }
            }
            ClientCommand::Logs { .. } => {
                // Each OK line is a verbatim log entry — print as-is.
                // Operators expect to be able to grep this output, so
                // we don't add any prefix decoration.
                for line in lines {
                    println!("{}", line);
                }
            }
            ClientCommand::VmEventsClear(_)
            | ClientCommand::Diagnose(_)
            | ClientCommand::Doctor(_)
            | ClientCommand::SelfCheck => {
                for line in lines {
                    println!("{}", line);
                }
            }
            ClientCommand::ConsoleCanvas(_) => {
                for line in lines {
                    if let Some(row) = line.strip_prefix("row\t") {
                        let mut parts = row.splitn(2, '\t');
                        let _ = parts.next();
                        println!("{}", parts.next().unwrap_or(""));
                    } else {
                        println!("{}", line);
                    }
                }
            }
            ClientCommand::ShellList(_) | ClientCommand::ShellClose { .. } => {
                print_header(verbosity, "shell-sessions", *count);
                for line in lines {
                    let mut parts = line.split('\t');
                    println!(
                        "{}\tname={}\tmode={}\tstdout={}\tstdin={}\tpid={}",
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("-"),
                    );
                }
            }
            ClientCommand::ShellShow { .. } => {
                for line in lines {
                    println!("{}", line);
                }
            }
            ClientCommand::Shell { .. } => {
                print_or_attach_shell(lines);
            }
            ClientCommand::Exec { .. } => {
                for line in lines {
                    let mut parts = line.split('\t');
                    let head = parts.next().unwrap_or("");
                    if head == "pid" {
                        println!("pid\t{}", parts.next().unwrap_or("0"));
                        continue;
                    }
                    println!(
                        "exec\t{}\tmode={}\tcwd={}\tenv={}\tcmd={}\tstdout={}\tstdin={}",
                        head,
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("-"),
                    );
                }
            }
            ClientCommand::ExecShow { .. } | ClientCommand::ExecClear(_) => {
                for line in lines {
                    println!("{}", line);
                }
            }
            ClientCommand::ExecList(_) => {
                print_header(verbosity, "execs", *count);
                for line in lines {
                    let mut parts = line.split('\t');
                    println!(
                        "{}\tmode={}\tcwd={}\tenv={}\tcmd={}\tstdout={}\tstdin={}\tpid={}",
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("-"),
                        parts.next().unwrap_or("-"),
                    );
                }
            }
        },
    }
}

#[cfg(not(target_os = "linux"))]
#[derive(Clone, Debug, PartialEq, Eq)]
struct ShellAttachInfo {
    session_id: String,
    session_name: String,
    mode: String,
    console_pipe: String,
    stdin_pipe: String,
    attached_pid: u32,
    reused: bool,
}

#[cfg(not(target_os = "linux"))]
fn parse_shell_attach_info(lines: &[String]) -> Option<ShellAttachInfo> {
    let mut info = None;
    let mut reused = false;
    for line in lines {
        let mut parts = line.split('\t');
        let head = parts.next().unwrap_or("");
        if head == "reused" {
            reused = parts.next().unwrap_or("false") == "true";
            continue;
        }
        info = Some(ShellAttachInfo {
            session_id: String::from(head),
            session_name: String::from(parts.next().unwrap_or("-")),
            mode: String::from(parts.next().unwrap_or("-")),
            console_pipe: String::from(parts.next().unwrap_or("-")),
            stdin_pipe: String::from(parts.next().unwrap_or("-")),
            attached_pid: parse_u32(parts.next().unwrap_or("0")).unwrap_or(0),
            reused,
        });
    }
    info.map(|mut value| {
        value.reused = reused;
        value
    })
}

#[cfg(target_os = "linux")]
fn print_or_attach_shell(lines: &[String]) {
    for line in lines {
        let mut parts = line.split('\t');
        let head = parts.next().unwrap_or("");
        if head == "reused" {
            println!("reused\t{}", parts.next().unwrap_or("false"));
            continue;
        }
        println!(
            "session\t{}\tname={}\tmode={}\tstdout={}\tstdin={}\tpid={}",
            head,
            parts.next().unwrap_or("-"),
            parts.next().unwrap_or("-"),
            parts.next().unwrap_or("-"),
            parts.next().unwrap_or("-"),
            parts.next().unwrap_or("-"),
        );
    }
}

#[cfg(not(target_os = "linux"))]
fn print_or_attach_shell(lines: &[String]) {
    let Some(info) = parse_shell_attach_info(lines) else {
        println!("aslctl: malformed shell response");
        return;
    };
    if info.console_pipe == "-" || info.stdin_pipe == "-" {
        println!(
            "session\t{}\tname={}\tmode={}\tstdout={}\tstdin={}\tpid={}",
            info.session_id,
            info.session_name,
            info.mode,
            info.console_pipe,
            info.stdin_pipe,
            info.attached_pid
        );
        return;
    }
    attach_shell(&info);
}

#[cfg(not(target_os = "linux"))]
fn attach_shell(info: &ShellAttachInfo) {
    let output_pipe = ipc::pipe_open(&info.console_pipe);
    let input_pipe = ipc::pipe_open(&info.stdin_pipe);
    if output_pipe == 0 || output_pipe == u32::MAX || input_pipe == 0 || input_pipe == u32::MAX {
        println!("aslctl: failed to attach ASL console pipes");
        return;
    }

    fs::set_fd_nonblock(0, true);
    let _ = fs::write(
        1,
        format!(
            "Connected to ASL {} ({}, {}). Ctrl+D detaches.\n",
            info.session_id, info.mode, info.session_name
        )
        .as_bytes(),
    );

    let mut output = [0u8; 512];
    let mut input = [0u8; 128];
    loop {
        drain_shell_output(output_pipe, &mut output);
        if !pump_shell_stdin(input_pipe, &mut input) {
            break;
        }
        if info.attached_pid != 0
            && process::try_waitpid(info.attached_pid) != process::STILL_RUNNING
        {
            drain_shell_output(output_pipe, &mut output);
            break;
        }
        process::sleep(10);
    }

    fs::set_fd_nonblock(0, false);
    ipc::pipe_close(output_pipe);
    ipc::pipe_close(input_pipe);
}

#[cfg(not(target_os = "linux"))]
fn drain_shell_output(pipe: u32, buf: &mut [u8]) {
    loop {
        let n = ipc::pipe_read(pipe, buf);
        if n == 0 || n == u32::MAX {
            break;
        }
        let _ = fs::write(1, &buf[..n as usize]);
    }
}

#[cfg(not(target_os = "linux"))]
fn pump_shell_stdin(pipe: u32, buf: &mut [u8]) -> bool {
    let n = fs::read_nonblock(0, buf);
    if n > 0 {
        for &byte in &buf[..n as usize] {
            if byte == 0x04 {
                let _ = fs::write(1, b"\nDetached.\n");
                return false;
            }
        }
        let _ = ipc::pipe_write(pipe, &buf[..n as usize]);
    }

    let key = sys::con_poll_key();
    if key == 0 {
        return true;
    }
    if key == 0x04 {
        let _ = fs::write(1, b"\nDetached.\n");
        return false;
    }
    let mut encoded = [0u8; 4];
    let bytes = encode_key(key, &mut encoded);
    if !bytes.is_empty() {
        let _ = ipc::pipe_write(pipe, bytes);
    }
    true
}

#[cfg(not(target_os = "linux"))]
fn encode_key(key: u32, buf: &mut [u8; 4]) -> &[u8] {
    if key <= 0x7f {
        buf[0] = key as u8;
        return &buf[..1];
    }
    let Some(ch) = char::from_u32(key) else {
        return &[];
    };
    ch.encode_utf8(buf).as_bytes()
}

// ---------------------------------------------------------------------------
//  JSON output (--json)
// ---------------------------------------------------------------------------
//
// aslctl is no_std with only `anyos_std` available — no serde. The JSON
// shape is small and stable, so a hand-rolled encoder with proper string
// escaping is sufficient and stays testable.
//
// Response shape:
//   { "ok": true,  "command": "<verb>", "lines": ["...", "..."] }
//   { "ok": false, "command": "<verb>", "code": "...", "message": "..." }

/// Escape a Rust `&str` into a JSON string body (no surrounding quotes).
/// Handles the JSON-mandatory escapes (\\, \", \b, \f, \n, \r, \t) and
/// escapes any remaining control character (< 0x20) as \u00XX. UTF-8
/// multi-byte sequences pass through unchanged — JSON allows raw UTF-8
/// in strings, only ASCII control characters and the two reserved
/// characters need escaping.
pub fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let code = c as u32;
                const HEX: &[u8; 16] = b"0123456789abcdef";
                out.push_str("\\u00");
                out.push(HEX[((code >> 4) & 0xf) as usize] as char);
                out.push(HEX[(code & 0xf) as usize] as char);
            }
            c => out.push(c),
        }
    }
    out
}

/// Stable verb name for a command, used as the `command` field in JSON
/// output so consumers don't have to parse the wire command back from
/// `as_wire`. Names mirror the CLI subcommand spelling where one exists.
pub fn json_command_name(command: &ClientCommand<'_>) -> &'static str {
    match command {
        ClientCommand::List => "list",
        ClientCommand::Status(_) => "status",
        ClientCommand::Create { .. } => "create",
        ClientCommand::Delete { .. } => "delete",
        ClientCommand::Clone { .. } => "clone",
        ClientCommand::Export(_) => "export",
        ClientCommand::Config(_) => "config",
        ClientCommand::ConfigSetResources { .. } => "config-set-resources",
        ClientCommand::StorageImport { .. } => "storage-import",
        ClientCommand::StorageValidate(_) => "storage-validate",
        ClientCommand::NetworkShow(_) => "network-show",
        ClientCommand::NetworkSet { .. } => "network-set",
        ClientCommand::NetworkValidate(_) => "network-validate",
        ClientCommand::Start(_) => "start",
        ClientCommand::Restart(_) => "restart",
        ClientCommand::Stop(_) => "stop",
        ClientCommand::AgentStatus(_) => "agent-status",
        ClientCommand::AgentRestart(_) => "agent-restart",
        ClientCommand::VmStatus(_) => "vm-status",
        ClientCommand::VmEvents(_) => "events",
        ClientCommand::VmEventsTail { .. } => "events-tail",
        ClientCommand::VmEventsClear(_) => "events-clear",
        ClientCommand::Diagnose(_) => "diagnose",
        ClientCommand::Doctor(_) => "doctor",
        ClientCommand::Logs { .. } => "logs",
        ClientCommand::SelfCheck => "self-check",
        ClientCommand::ConsoleCanvas(_) => "console-canvas",
        ClientCommand::ShellList(_) => "shell-list",
        ClientCommand::ShellShow { .. } => "shell-show",
        ClientCommand::ShellClose { .. } => "shell-close",
        ClientCommand::Shell { .. } => "shell",
        ClientCommand::ExecList(_) => "exec-list",
        ClientCommand::ExecShow { .. } => "exec-show",
        ClientCommand::ExecClear(_) => "exec-clear",
        ClientCommand::Exec { .. } => "exec",
        ClientCommand::MountList(_) => "mount-list",
        ClientCommand::MountShow { .. } => "mount-show",
        ClientCommand::MountAdd { .. } => "mount-add",
        ClientCommand::MountRemove { .. } => "mount-remove",
        ClientCommand::MountValidate(_) => "mount-validate",
        ClientCommand::PortList(_) => "port-list",
        ClientCommand::PortAdd { .. } => "port-add",
        ClientCommand::PortRemove { .. } => "port-remove",
        ClientCommand::PortValidate(_) => "port-validate",
    }
}

/// Render a successful response as JSON. `lines` go through string
/// escaping so embedded tabs and newlines round-trip cleanly.
pub fn format_json_ok(command: &ClientCommand<'_>, lines: &[String]) -> String {
    let mut out = String::with_capacity(64 + lines.iter().map(|l| l.len() + 4).sum::<usize>());
    out.push_str("{\"ok\":true,\"command\":\"");
    out.push_str(json_command_name(command));
    out.push_str("\",\"lines\":[");
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('"');
        out.push_str(&json_escape(line));
        out.push('"');
    }
    out.push_str("]}");
    out
}

/// Render an error response as JSON. Used for both daemon-side ERR
/// responses and aslctl-internal transport errors.
pub fn format_json_err(command: &ClientCommand<'_>, code: &str, message: &str) -> String {
    let mut out = String::with_capacity(96);
    out.push_str("{\"ok\":false,\"command\":\"");
    out.push_str(json_command_name(command));
    out.push_str("\",\"code\":\"");
    out.push_str(&json_escape(code));
    out.push_str("\",\"message\":\"");
    out.push_str(&json_escape(message));
    out.push_str("\"}");
    out
}

/// Wrap any `WireResponse` for JSON output.
pub fn format_json_response(command: &ClientCommand<'_>, response: &WireResponse) -> String {
    match response {
        WireResponse::Ok { lines, .. } => format_json_ok(command, lines),
        WireResponse::Err { code, message } => format_json_err(command, code, message),
    }
}

/// Convenience: emit a transport-error JSON object when send_command
/// fails outright (no daemon response). Same shape as a daemon ERR.
pub fn json_error_response(command: &ClientCommand<'_>, code: &str, message: &str) -> String {
    format_json_err(command, code, message)
}

fn print_usage() {
    println!("aslctl - control ASL distros");
    println!();
    println!("Global flags (must come before the subcommand):");
    println!("  --json                machine-readable JSON output");
    println!("  --quiet | -q          suppress section headers");
    println!("  --verbose | -v        reserved for future per-command detail");
    println!();
    println!("Usage:");
    println!("  aslctl list");
    println!("  aslctl status <name>");
    println!("  aslctl create <name> <image-ref> <owner> [--kernel-profile <profile>]");
    println!("  aslctl delete <name> [--force]");
    println!("  aslctl clone <source> <target> [--owner <owner>] [--no-mounts]");
    println!("  aslctl export <name>");
    println!("  aslctl config <name>");
    println!("  aslctl config set-resources <name> [--memory-mb <MiB>] [--vcpus <count>]");
    println!("  aslctl storage import <name> <source-raw-image>");
    println!("  aslctl storage validate <name>");
    println!("  aslctl network show <name>");
    println!("  aslctl network set <name> [--mode nat] [--dns-mode host-broker] [--allow-outbound true|false]");
    println!("  aslctl network validate <name>");
    println!("  aslctl start <name>");
    println!("  aslctl restart <name>");
    println!("  aslctl stop <name>");
    println!("  aslctl agent-status <name>");
    println!("  aslctl agent-restart <name>");
    println!("  aslctl agent status <name>");
    println!("  aslctl agent restart <name>");
    println!("  aslctl vm-status <name>");
    println!("  aslctl events <name>                 (alias: vm-events)");
    println!("  aslctl events-tail <name> [limit]    (alias: vm-events-tail)");
    println!("  aslctl events-clear <name>           (alias: vm-events-clear)");
    println!("  aslctl logs [<distro>|*] [<limit>]   (tail asld log; default 200 lines)");
    println!("  aslctl diagnose <name>");
    println!("  aslctl doctor <name>");
    println!("  aslctl self-check");
    println!("  aslctl console-canvas <name>");
    println!("  aslctl shell-list <name>");
    println!("  aslctl shell-show <name> <session-id>");
    println!("  aslctl shell-close <name> <session-id>");
    println!("  aslctl shell <name> [--fallback-console] [--session <name>]");
    println!("  aslctl exec-list <name>");
    println!("  aslctl exec-show <name> <exec-id>");
    println!("  aslctl exec-clear <name>");
    println!("  aslctl exec <name> [--fallback-console] [--cwd <path>] [--env KEY=VALUE] -- <command> [args...]");
    println!("  aslctl run  <name> [--fallback-console] [--cwd <path>] [--env KEY=VALUE] -- <command> [args...]   (alias for exec)");
    println!("  aslctl mount list <name>");
    println!("  aslctl mount validate <name>");
    println!("  aslctl mount show <name> <mount-id>");
    println!("  aslctl mount add <name> <mount-id> <host> <guest> <mode> <metadata> <case> <exec> <watch> [description]");
    println!("  aslctl mount remove <name> <mount-id>");
    println!("  aslctl port list <name>");
    println!("  aslctl port validate <name>");
    println!("  aslctl port add <name> <rule-id> <listen-address> <listen-port> <guest-port> <protocol> [description]");
    println!("  aslctl port remove <name> <rule-id>");
}

impl ClientCommand<'_> {
    fn as_wire(&self) -> String {
        match self {
            Self::List => String::from("LIST"),
            Self::Status(name) => format!("STATUS {}", name),
            Self::Create {
                name,
                image_ref,
                owner,
                kernel_profile,
            } => format!(
                "CREATE {}\t{}\t{}\t{}",
                name,
                image_ref,
                owner,
                kernel_profile.unwrap_or("-")
            ),
            Self::Delete { name, force } => {
                format!("DELETE {}\t{}", name, if *force { 1 } else { 0 })
            }
            Self::Clone {
                source,
                target,
                owner,
                include_mounts,
            } => format!(
                "CLONE {}\t{}\t{}\t{}",
                source,
                target,
                owner.unwrap_or("-"),
                if *include_mounts { 1 } else { 0 }
            ),
            Self::Export(name) => format!("EXPORT {}", name),
            Self::Config(name) => format!("CONFIG_SHOW {}", name),
            Self::ConfigSetResources {
                distro,
                memory_mb,
                vcpu_count,
            } => format!(
                "CONFIG_SET_RESOURCES {}\t{}\t{}",
                distro,
                memory_mb.unwrap_or("-"),
                vcpu_count.unwrap_or("-")
            ),
            Self::StorageImport {
                distro,
                source_path,
            } => format!("STORAGE_IMPORT {}\t{}", distro, source_path),
            Self::StorageValidate(name) => format!("STORAGE_VALIDATE {}", name),
            Self::NetworkShow(name) => format!("NETWORK_SHOW {}", name),
            Self::NetworkSet {
                distro,
                mode,
                dns_mode,
                allow_outbound,
            } => format!(
                "NETWORK_SET {}\t{}\t{}\t{}",
                distro,
                mode.unwrap_or("-"),
                dns_mode.unwrap_or("-"),
                allow_outbound.unwrap_or("-")
            ),
            Self::NetworkValidate(name) => format!("NETWORK_VALIDATE {}", name),
            Self::Start(name) => format!("START {}", name),
            Self::Restart(name) => format!("RESTART {}", name),
            Self::Stop(name) => format!("STOP {}", name),
            Self::AgentStatus(name) => format!("AGENT_STATUS {}", name),
            Self::AgentRestart(name) => format!("AGENT_RESTART {}", name),
            Self::VmStatus(name) => format!("VM_STATUS {}", name),
            Self::VmEvents(name) => format!("VM_EVENTS {}", name),
            Self::VmEventsTail { distro, limit } => format!("VM_EVENTS_TAIL {}\t{}", distro, limit),
            Self::Logs { distro, limit } => format!("LOGS {}\t{}", distro, limit),
            Self::VmEventsClear(name) => format!("VM_EVENTS_CLEAR {}", name),
            Self::Diagnose(name) => format!("DIAGNOSE {}", name),
            Self::Doctor(name) => format!("DOCTOR {}", name),
            Self::SelfCheck => String::from("SELF_CHECK"),
            Self::ConsoleCanvas(name) => format!("CONSOLE_CANVAS {}", name),
            Self::ShellList(name) => format!("SHELL_LIST {}", name),
            Self::ShellShow { distro, session_id } => {
                format!("SHELL_SHOW {}\t{}", distro, session_id)
            }
            Self::ShellClose { distro, session_id } => {
                format!("SHELL_CLOSE {}\t{}", distro, session_id)
            }
            Self::Shell {
                distro,
                fallback_console,
                session_name,
            } => format!(
                "SHELL_OPEN {}\t{}\t{}",
                distro,
                session_name,
                if *fallback_console { 1 } else { 0 }
            ),
            Self::Exec {
                distro,
                fallback_console,
                cwd,
                env,
                argv,
            } => format!(
                "EXEC {}\t{}\t{}\t{}\t{}",
                distro,
                if *fallback_console { 1 } else { 0 },
                cwd,
                join_control_field(env),
                join_control_field(argv)
            ),
            Self::ExecList(distro) => format!("EXEC_LIST {}", distro),
            Self::ExecShow { distro, exec_id } => format!("EXEC_SHOW {}\t{}", distro, exec_id),
            Self::ExecClear(distro) => format!("EXEC_CLEAR {}", distro),
            Self::MountList(distro) => format!("MOUNT_LIST {}", distro),
            Self::MountShow { distro, mount_id } => format!("MOUNT_SHOW {}\t{}", distro, mount_id),
            Self::MountAdd {
                distro,
                mount_id,
                host_path,
                guest_path,
                mode,
                metadata_mode,
                case_mode,
                exec_policy,
                watch_policy,
                description,
            } => format!(
                "MOUNT_ADD {}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                distro,
                mount_id,
                host_path,
                guest_path,
                mode,
                metadata_mode,
                case_mode,
                exec_policy,
                watch_policy,
                description
            ),
            Self::MountRemove { distro, mount_id } => {
                format!("MOUNT_REMOVE {}\t{}", distro, mount_id)
            }
            Self::MountValidate(distro) => format!("MOUNT_VALIDATE {}", distro),
            Self::PortList(distro) => format!("PORT_LIST {}", distro),
            Self::PortAdd {
                distro,
                rule_id,
                listen_address,
                listen_port,
                guest_port,
                protocol,
                description,
            } => format!(
                "PORT_ADD {}\t{}\t{}\t{}\t{}\t{}\t{}",
                distro, rule_id, listen_address, listen_port, guest_port, protocol, description
            ),
            Self::PortRemove { distro, rule_id } => format!("PORT_REMOVE {}\t{}", distro, rule_id),
            Self::PortValidate(distro) => format!("NETWORK_VALIDATE {}", distro),
        }
    }
}

fn tokenize<'a>(raw: &'a str) -> Vec<&'a str> {
    let bytes = raw.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let start;
        let end;
        if bytes[i] == b'"' {
            i += 1;
            start = i;
            while i < bytes.len() && bytes[i] != b'"' {
                i += 1;
            }
            end = i;
            if i < bytes.len() {
                i += 1;
            }
        } else {
            start = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            end = i;
        }
        out.push(&raw[start..end]);
    }
    out
}

fn join_control_field(parts: &[&str]) -> String {
    let mut out = String::new();
    for part in parts {
        if !out.is_empty() {
            out.push('\x1f');
        }
        out.push_str(part);
    }
    out
}

fn parse_usize(s: &str) -> Option<usize> {
    let mut out = 0usize;
    if s.is_empty() {
        return None;
    }
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        out = out.checked_mul(10)?.checked_add((b - b'0') as usize)?;
    }
    Some(out)
}

#[cfg(not(target_os = "linux"))]
fn parse_u32(s: &str) -> Option<u32> {
    parse_usize(s).and_then(|value| u32::try_from(value).ok())
}

fn join_tab_fields<'a, I>(parts: &mut I) -> String
where
    I: Iterator<Item = &'a str>,
{
    let mut out = String::new();
    for part in parts {
        if !out.is_empty() {
            out.push('\t');
        }
        out.push_str(part);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{
        format_json_err, format_json_ok, format_json_response, json_command_name, json_escape,
        parse_cli_command, parse_command, parse_response, strip_global_flags, ClientCommand,
        OutputFormat, Vec, Verbosity, WireResponse,
    };
    use anyos_std::String;

    #[test]
    fn parses_list_command() {
        let args = anyos_std::args::parse("list", b"");
        match parse_command(&args) {
            Some(ClientCommand::List) => {}
            _ => panic!("expected list command"),
        }
    }

    #[test]
    fn parses_create_command() {
        let args = anyos_std::args::parse("create ubuntu-dev ubuntu-24.04-x86_64-v1 strati", b"");
        match parse_command(&args) {
            Some(ClientCommand::Create {
                name,
                image_ref,
                owner,
                kernel_profile,
            }) => {
                assert_eq!(name, "ubuntu-dev");
                assert_eq!(image_ref, "ubuntu-24.04-x86_64-v1");
                assert_eq!(owner, "strati");
                assert_eq!(kernel_profile, None);
            }
            _ => panic!("expected create command"),
        }

        match parse_cli_command(
            "create pcboot ubuntu-24.04-x86_64-v1 strati --kernel-profile seabios-x86_64",
        ) {
            Some(ClientCommand::Create { kernel_profile, .. }) => {
                assert_eq!(kernel_profile, Some("seabios-x86_64"));
            }
            _ => panic!("expected create command with kernel profile"),
        }
    }

    #[test]
    fn parses_delete_config_and_doctor_commands() {
        match parse_cli_command("delete ubuntu-dev --force") {
            Some(ClientCommand::Delete { name, force }) => {
                assert_eq!(name, "ubuntu-dev");
                assert!(force);
            }
            _ => panic!("expected delete command"),
        }
        match parse_cli_command("config ubuntu-dev") {
            Some(ClientCommand::Config(name)) => assert_eq!(name, "ubuntu-dev"),
            _ => panic!("expected config command"),
        }
        match parse_cli_command("doctor ubuntu-dev") {
            Some(ClientCommand::Doctor(name)) => assert_eq!(name, "ubuntu-dev"),
            _ => panic!("expected doctor command"),
        }
        match parse_cli_command("self-check") {
            Some(ClientCommand::SelfCheck) => {}
            _ => panic!("expected self-check command"),
        }
    }

    #[test]
    fn parses_clone_export_and_storage_commands() {
        match parse_cli_command("clone ubuntu-dev ubuntu-copy --owner ops --no-mounts") {
            Some(ClientCommand::Clone {
                source,
                target,
                owner,
                include_mounts,
            }) => {
                assert_eq!(source, "ubuntu-dev");
                assert_eq!(target, "ubuntu-copy");
                assert_eq!(owner, Some("ops"));
                assert!(!include_mounts);
            }
            _ => panic!("expected clone command"),
        }
        match parse_cli_command("export ubuntu-copy") {
            Some(ClientCommand::Export(name)) => assert_eq!(name, "ubuntu-copy"),
            _ => panic!("expected export command"),
        }
        let args = anyos_std::args::parse("storage validate ubuntu-copy", b"");
        match parse_command(&args) {
            Some(ClientCommand::StorageValidate(name)) => assert_eq!(name, "ubuntu-copy"),
            _ => panic!("expected storage validate command"),
        }
        let args =
            anyos_std::args::parse("storage import ubuntu-copy /Users/Shared/debian.raw", b"");
        match parse_command(&args) {
            Some(ClientCommand::StorageImport {
                distro,
                source_path,
            }) => {
                assert_eq!(distro, "ubuntu-copy");
                assert_eq!(source_path, "/Users/Shared/debian.raw");
            }
            _ => panic!("expected storage import command"),
        }
    }

    #[test]
    fn parses_config_set_resources_command() {
        match parse_cli_command("config set-resources ubuntu-dev --memory-mb 4096 --vcpus 4") {
            Some(ClientCommand::ConfigSetResources {
                distro,
                memory_mb,
                vcpu_count,
            }) => {
                assert_eq!(distro, "ubuntu-dev");
                assert_eq!(memory_mb, Some("4096"));
                assert_eq!(vcpu_count, Some("4"));
            }
            _ => panic!("expected config set-resources command"),
        }
    }

    #[test]
    fn parses_network_commands() {
        match parse_cli_command("network show ubuntu-dev") {
            Some(ClientCommand::NetworkShow(name)) => assert_eq!(name, "ubuntu-dev"),
            _ => panic!("expected network show command"),
        }
        match parse_cli_command("network validate ubuntu-dev") {
            Some(ClientCommand::NetworkValidate(name)) => assert_eq!(name, "ubuntu-dev"),
            _ => panic!("expected network validate command"),
        }
        match parse_cli_command(
            "network set ubuntu-dev --mode nat --dns-mode host-broker --allow-outbound false",
        ) {
            Some(ClientCommand::NetworkSet {
                distro,
                mode,
                dns_mode,
                allow_outbound,
            }) => {
                assert_eq!(distro, "ubuntu-dev");
                assert_eq!(mode, Some("nat"));
                assert_eq!(dns_mode, Some("host-broker"));
                assert_eq!(allow_outbound, Some("false"));
            }
            _ => panic!("expected network set command"),
        }
    }

    #[test]
    fn parses_mount_add_command() {
        let args = anyos_std::args::parse(
            "mount add ubuntu-dev workspace /Users/strati/work /mnt/work readwrite relaxed host-native inherit best-effort Workspace",
            b"",
        );
        match parse_command(&args) {
            Some(ClientCommand::MountAdd {
                distro,
                mount_id,
                host_path,
                guest_path,
                mode,
                description,
                ..
            }) => {
                assert_eq!(distro, "ubuntu-dev");
                assert_eq!(mount_id, "workspace");
                assert_eq!(host_path, "/Users/strati/work");
                assert_eq!(guest_path, "/mnt/work");
                assert_eq!(mode, "readwrite");
                assert_eq!(description, "Workspace");
            }
            _ => panic!("expected mount add command"),
        }
    }

    #[test]
    fn parses_mount_validate_command() {
        let args = anyos_std::args::parse("mount validate ubuntu-dev", b"");
        match parse_command(&args) {
            Some(ClientCommand::MountValidate(name)) => assert_eq!(name, "ubuntu-dev"),
            _ => panic!("expected mount validate command"),
        }
    }

    #[test]
    fn parses_port_add_command() {
        let args =
            anyos_std::args::parse("port add ubuntu-dev web 127.0.0.1 3000 3000 tcp Web", b"");
        match parse_command(&args) {
            Some(ClientCommand::PortAdd {
                distro,
                rule_id,
                listen_address,
                listen_port,
                guest_port,
                protocol,
                description,
            }) => {
                assert_eq!(distro, "ubuntu-dev");
                assert_eq!(rule_id, "web");
                assert_eq!(listen_address, "127.0.0.1");
                assert_eq!(listen_port, "3000");
                assert_eq!(guest_port, "3000");
                assert_eq!(protocol, "tcp");
                assert_eq!(description, "Web");
            }
            _ => panic!("expected port add command"),
        }
    }

    #[test]
    fn parses_port_validate_command() {
        let args = anyos_std::args::parse("port validate ubuntu-dev", b"");
        match parse_command(&args) {
            Some(ClientCommand::PortValidate(name)) => assert_eq!(name, "ubuntu-dev"),
            _ => panic!("expected port validate command"),
        }
    }

    #[test]
    fn parses_vm_status_command() {
        let args = anyos_std::args::parse("vm-status ubuntu-dev", b"");
        match parse_command(&args) {
            Some(ClientCommand::VmStatus(name)) => assert_eq!(name, "ubuntu-dev"),
            _ => panic!("expected vm status command"),
        }
        let args = anyos_std::args::parse("agent-restart ubuntu-dev", b"");
        match parse_command(&args) {
            Some(ClientCommand::AgentRestart(name)) => assert_eq!(name, "ubuntu-dev"),
            _ => panic!("expected agent restart command"),
        }
        let args = anyos_std::args::parse("agent status ubuntu-dev", b"");
        match parse_command(&args) {
            Some(ClientCommand::AgentStatus(name)) => assert_eq!(name, "ubuntu-dev"),
            _ => panic!("expected agent status command"),
        }
    }

    #[test]
    fn events_is_alias_for_vm_events() {
        // ADR-style naming pin: `events` and `vm-events` parse to the
        // same variant so they hit the same wire command. This keeps
        // the spec-aligned name working without a dual code path.
        let a = anyos_std::args::parse("events ubuntu-dev", b"");
        let b = anyos_std::args::parse("vm-events ubuntu-dev", b"");
        let parsed_a = parse_command(&a);
        let parsed_b = parse_command(&b);
        assert!(matches!(
            parsed_a,
            Some(ClientCommand::VmEvents("ubuntu-dev"))
        ));
        assert!(matches!(
            parsed_b,
            Some(ClientCommand::VmEvents("ubuntu-dev"))
        ));
        // Wire output must be byte-identical so a script that uses
        // either form gets the same response.
        assert_eq!(parsed_a.unwrap().as_wire(), parsed_b.unwrap().as_wire());
    }

    #[test]
    fn events_tail_alias_parses_with_limit() {
        let args = anyos_std::args::parse("events-tail ubuntu-dev 5", b"");
        match parse_command(&args) {
            Some(ClientCommand::VmEventsTail { distro, limit }) => {
                assert_eq!(distro, "ubuntu-dev");
                assert_eq!(limit, "5");
            }
            _ => panic!("expected events-tail command"),
        }
    }

    #[test]
    fn events_clear_alias_parses() {
        let args = anyos_std::args::parse("events-clear ubuntu-dev", b"");
        assert!(matches!(
            parse_command(&args),
            Some(ClientCommand::VmEventsClear("ubuntu-dev"))
        ));
    }

    #[test]
    fn parses_logs_command_with_distro_and_limit() {
        let args = anyos_std::args::parse("logs ubuntu-dev 50", b"");
        match parse_command(&args) {
            Some(ClientCommand::Logs { distro, limit }) => {
                assert_eq!(distro, "ubuntu-dev");
                assert_eq!(limit, 50);
            }
            _ => panic!("expected logs command"),
        }
    }

    #[test]
    fn parses_logs_command_defaults() {
        let args = anyos_std::args::parse("logs", b"");
        match parse_command(&args) {
            Some(ClientCommand::Logs { distro, limit }) => {
                assert_eq!(distro, "*");
                assert_eq!(limit, 200);
            }
            _ => panic!("expected logs command with defaults"),
        }
    }

    #[test]
    fn parses_logs_command_with_explicit_wildcard() {
        let args = anyos_std::args::parse("logs * 5", b"");
        match parse_command(&args) {
            Some(ClientCommand::Logs { distro, limit }) => {
                assert_eq!(distro, "*");
                assert_eq!(limit, 5);
            }
            _ => panic!("expected logs command"),
        }
    }

    #[test]
    fn logs_command_garbage_limit_falls_back_to_default() {
        let args = anyos_std::args::parse("logs ubuntu-dev nonsense", b"");
        match parse_command(&args) {
            Some(ClientCommand::Logs { distro, limit }) => {
                assert_eq!(distro, "ubuntu-dev");
                // unparseable limit -> default 200, not 0 (would mean
                // "unlimited" on the daemon side and could overwhelm
                // the operator's terminal).
                assert_eq!(limit, 200);
            }
            _ => panic!("expected logs command"),
        }
    }

    #[test]
    fn logs_command_emits_correct_wire_format() {
        let cmd = ClientCommand::Logs {
            distro: "ubuntu-dev",
            limit: 100,
        };
        assert_eq!(cmd.as_wire(), "LOGS ubuntu-dev\t100");

        let cmd = ClientCommand::Logs {
            distro: "*",
            limit: 200,
        };
        assert_eq!(cmd.as_wire(), "LOGS *\t200");
    }

    #[test]
    fn parses_shell_list_and_exec_list_commands() {
        match parse_cli_command("shell-list ubuntu-dev") {
            Some(ClientCommand::ShellList(name)) => assert_eq!(name, "ubuntu-dev"),
            _ => panic!("expected shell-list command"),
        }
        match parse_cli_command("shell-show ubuntu-dev sh-00000001") {
            Some(ClientCommand::ShellShow { distro, session_id }) => {
                assert_eq!(distro, "ubuntu-dev");
                assert_eq!(session_id, "sh-00000001");
            }
            _ => panic!("expected shell-show command"),
        }
        match parse_cli_command("exec-list ubuntu-dev") {
            Some(ClientCommand::ExecList(name)) => assert_eq!(name, "ubuntu-dev"),
            _ => panic!("expected exec-list command"),
        }
        match parse_cli_command("exec-show ubuntu-dev exec-00000001") {
            Some(ClientCommand::ExecShow { distro, exec_id }) => {
                assert_eq!(distro, "ubuntu-dev");
                assert_eq!(exec_id, "exec-00000001");
            }
            _ => panic!("expected exec-show command"),
        }
        match parse_cli_command("restart ubuntu-dev") {
            Some(ClientCommand::Restart(name)) => assert_eq!(name, "ubuntu-dev"),
            _ => panic!("expected restart command"),
        }
        match parse_cli_command("vm-events-tail ubuntu-dev 5") {
            Some(ClientCommand::VmEventsTail { distro, limit }) => {
                assert_eq!(distro, "ubuntu-dev");
                assert_eq!(limit, "5");
            }
            _ => panic!("expected vm-events-tail command"),
        }
    }

    #[test]
    fn parses_ok_wire_response() {
        let response = parse_response("OK\t1\nname\tubuntu-dev\nstate\tcreated\n\n").unwrap();
        match response {
            WireResponse::Ok { count, lines } => {
                assert_eq!(count, 1);
                assert_eq!(lines.len(), 2);
                assert_eq!(lines[0], "name\tubuntu-dev");
            }
            _ => panic!("expected OK response"),
        }
    }

    #[test]
    fn parses_error_wire_response() {
        let response = parse_response("ERR\tnot_found\tdistro missing\n\n").unwrap();
        match response {
            WireResponse::Err { code, message } => {
                assert_eq!(code, "not_found");
                assert_eq!(message, "distro missing");
            }
            _ => panic!("expected error response"),
        }
    }

    #[test]
    fn parses_shell_command_with_session() {
        match parse_cli_command("shell ubuntu-dev --session dev --fallback-console") {
            Some(ClientCommand::Shell {
                distro,
                fallback_console,
                session_name,
            }) => {
                assert_eq!(distro, "ubuntu-dev");
                assert!(fallback_console);
                assert_eq!(session_name, "dev");
            }
            _ => panic!("expected shell command"),
        }
    }

    #[test]
    fn parses_exec_command_with_env_and_cwd() {
        match parse_cli_command(
            "exec ubuntu-dev --cwd /workspace --env RUST_BACKTRACE=1 -- cargo test",
        ) {
            Some(ClientCommand::Exec {
                distro,
                cwd,
                env,
                argv,
                ..
            }) => {
                assert_eq!(distro, "ubuntu-dev");
                assert_eq!(cwd, "/workspace");
                assert_eq!(env.len(), 1);
                assert_eq!(argv, Vec::from(["cargo", "test"]));
            }
            _ => panic!("expected exec command"),
        }
    }

    #[test]
    fn parses_run_command_with_env_and_cwd() {
        match parse_cli_command(
            "run ubuntu-dev --cwd /workspace --env RUST_BACKTRACE=1 -- cargo test",
        ) {
            Some(ClientCommand::Exec {
                distro,
                cwd,
                env,
                argv,
                ..
            }) => {
                assert_eq!(distro, "ubuntu-dev");
                assert_eq!(cwd, "/workspace");
                assert_eq!(env.len(), 1);
                assert_eq!(argv, Vec::from(["cargo", "test"]));
            }
            _ => panic!("expected run -> Exec command"),
        }
    }

    #[test]
    fn run_and_exec_emit_identical_wire_command() {
        let exec = parse_cli_command(
            "exec ubuntu-dev --cwd /workspace --env A=1 --env B=2 -- cargo build --release",
        )
        .expect("exec parses");
        let run = parse_cli_command(
            "run ubuntu-dev --cwd /workspace --env A=1 --env B=2 -- cargo build --release",
        )
        .expect("run parses");
        assert_eq!(exec.as_wire(), run.as_wire());
        assert!(exec.as_wire().starts_with("EXEC "));
    }

    #[test]
    fn parses_run_with_multiple_env_pairs() {
        match parse_cli_command("run ubuntu-dev --env FOO=1 --env BAR=baz -- printenv") {
            Some(ClientCommand::Exec { env, argv, .. }) => {
                assert_eq!(env, Vec::from(["FOO=1", "BAR=baz"]));
                assert_eq!(argv, Vec::from(["printenv"]));
            }
            _ => panic!("expected run command"),
        }
    }

    #[test]
    fn run_without_argv_is_rejected() {
        // Doc spec says `run <name> ... -- <command>` — without command, must fail.
        assert!(parse_cli_command("run ubuntu-dev --cwd /workspace --").is_none());
        assert!(parse_cli_command("run ubuntu-dev").is_none());
    }

    #[test]
    fn run_with_unknown_flag_is_rejected() {
        // Defense-in-depth: unknown flags must not silently be swallowed as argv.
        assert!(parse_cli_command("run ubuntu-dev --bogus -- cargo test").is_none());
    }

    #[test]
    fn run_passes_double_dash_separator_through() {
        // Tokens after `--` must be argv even if they look like flags.
        match parse_cli_command("run ubuntu-dev -- cargo --version") {
            Some(ClientCommand::Exec { argv, .. }) => {
                assert_eq!(argv, Vec::from(["cargo", "--version"]));
            }
            _ => panic!("expected run command"),
        }
    }

    #[test]
    fn run_with_fallback_console_flag() {
        match parse_cli_command("run ubuntu-dev --fallback-console -- bash -lc 'echo hi'") {
            Some(ClientCommand::Exec {
                fallback_console, ..
            }) => {
                assert!(fallback_console);
            }
            _ => panic!("expected run command"),
        }
    }

    // ========================================================================
    //  Global flags (--json / --quiet / --verbose)
    // ========================================================================

    #[test]
    fn strip_global_flags_default_on_no_flags() {
        let (flags, rest) = strip_global_flags("list");
        assert_eq!(flags.format, OutputFormat::Text);
        assert_eq!(flags.verbosity, Verbosity::Normal);
        assert_eq!(rest, "list");
    }

    #[test]
    fn strip_global_flags_recognises_json() {
        let (flags, rest) = strip_global_flags("--json list");
        assert_eq!(flags.format, OutputFormat::Json);
        assert_eq!(rest, "list");
    }

    #[test]
    fn strip_global_flags_recognises_quiet_short_and_long() {
        let (flags, _) = strip_global_flags("--quiet list");
        assert_eq!(flags.verbosity, Verbosity::Quiet);
        let (flags, _) = strip_global_flags("-q list");
        assert_eq!(flags.verbosity, Verbosity::Quiet);
    }

    #[test]
    fn strip_global_flags_recognises_verbose_short_and_long() {
        let (flags, _) = strip_global_flags("--verbose list");
        assert_eq!(flags.verbosity, Verbosity::Verbose);
        let (flags, _) = strip_global_flags("-v list");
        assert_eq!(flags.verbosity, Verbosity::Verbose);
    }

    #[test]
    fn strip_global_flags_combines_in_any_order() {
        let (flags, rest) = strip_global_flags("--json --quiet status ubuntu-dev");
        assert_eq!(flags.format, OutputFormat::Json);
        assert_eq!(flags.verbosity, Verbosity::Quiet);
        assert_eq!(rest, "status ubuntu-dev");

        let (flags, rest) = strip_global_flags("-q --json mount list ubuntu-dev");
        assert_eq!(flags.format, OutputFormat::Json);
        assert_eq!(flags.verbosity, Verbosity::Quiet);
        assert_eq!(rest, "mount list ubuntu-dev");
    }

    #[test]
    fn strip_global_flags_does_not_steal_subcommand_flags() {
        // `--cwd` belongs to the `exec` subcommand, not the global
        // parser. The stripper must stop as soon as it sees something
        // it doesn't recognise.
        let (flags, rest) =
            strip_global_flags("exec ubuntu-dev --cwd /workspace --env A=1 -- cargo test");
        assert_eq!(flags.format, OutputFormat::Text);
        assert_eq!(
            rest,
            "exec ubuntu-dev --cwd /workspace --env A=1 -- cargo test"
        );
    }

    #[test]
    fn strip_global_flags_stops_at_first_non_global() {
        // A global flag *after* the subcommand is left in place — the
        // stripper only consumes leading globals. This keeps the
        // contract one-directional and predictable.
        let (flags, rest) = strip_global_flags("list --json");
        assert_eq!(flags.format, OutputFormat::Text);
        assert_eq!(rest, "list --json");
    }

    #[test]
    fn strip_global_flags_handles_empty_input() {
        let (flags, rest) = strip_global_flags("");
        assert_eq!(flags.format, OutputFormat::Text);
        assert_eq!(rest, "");
    }

    #[test]
    fn strip_global_flags_handles_only_global_flags() {
        // Pathological case: user runs `aslctl --json` with no
        // subcommand. Strip the flags, return an empty remainder so
        // print_usage() fires.
        let (flags, rest) = strip_global_flags("--json");
        assert_eq!(flags.format, OutputFormat::Json);
        assert_eq!(rest, "");
    }

    // ========================================================================
    //  JSON encoder
    // ========================================================================

    #[test]
    fn json_escape_basic_chars() {
        assert_eq!(json_escape(""), "");
        assert_eq!(json_escape("hello"), "hello");
        assert_eq!(json_escape("a/b"), "a/b"); // forward slash is not escaped
    }

    #[test]
    fn json_escape_handles_quotes_and_backslash() {
        assert_eq!(json_escape("\""), "\\\"");
        assert_eq!(json_escape("\\"), "\\\\");
        assert_eq!(json_escape("path\\to\"file"), "path\\\\to\\\"file");
    }

    #[test]
    fn json_escape_handles_whitespace_controls() {
        assert_eq!(json_escape("\n"), "\\n");
        assert_eq!(json_escape("\r"), "\\r");
        assert_eq!(json_escape("\t"), "\\t");
        assert_eq!(json_escape("\u{0008}"), "\\b");
        assert_eq!(json_escape("\u{000c}"), "\\f");
    }

    #[test]
    fn json_escape_emits_unicode_escape_for_other_controls() {
        // 0x01 (SOH) is not one of the named escapes — must use \u00XX.
        assert_eq!(json_escape("\u{0001}"), "\\u0001");
        assert_eq!(json_escape("\u{001f}"), "\\u001f");
    }

    #[test]
    fn json_escape_passes_utf8_through_unchanged() {
        // JSON allows raw UTF-8 in strings — only ASCII controls and
        // the two reserved chars need escaping. Avoids ballooning
        // sizes for log lines containing Unicode.
        assert_eq!(json_escape("Grüße"), "Grüße");
        assert_eq!(json_escape("日本語"), "日本語");
    }

    #[test]
    fn json_ok_response_has_expected_shape() {
        let lines = Vec::from([String::from("alpha"), String::from("beta\twith\ttab")]);
        let s = format_json_ok(&ClientCommand::List, &lines);
        assert_eq!(
            s,
            "{\"ok\":true,\"command\":\"list\",\"lines\":[\"alpha\",\"beta\\twith\\ttab\"]}"
        );
    }

    #[test]
    fn json_err_response_has_expected_shape() {
        let s = format_json_err(
            &ClientCommand::Status("ubuntu-dev"),
            "not_found",
            "no such distro",
        );
        assert_eq!(
            s,
            "{\"ok\":false,\"command\":\"status\",\"code\":\"not_found\",\"message\":\"no such distro\"}"
        );
    }

    #[test]
    fn json_response_dispatches_on_variant() {
        let ok = WireResponse::Ok {
            count: 0,
            lines: Vec::new(),
        };
        let err = WireResponse::Err {
            code: String::from("oops"),
            message: String::from("bad"),
        };
        let s_ok = format_json_response(&ClientCommand::List, &ok);
        let s_err = format_json_response(&ClientCommand::List, &err);
        assert!(s_ok.contains("\"ok\":true"));
        assert!(s_err.contains("\"ok\":false"));
    }

    #[test]
    fn json_command_names_cover_recent_additions() {
        // Spot-check the command names introduced in this round so
        // the JSON consumer surface stays stable.
        assert_eq!(
            json_command_name(&ClientCommand::Logs {
                distro: "*",
                limit: 200,
            }),
            "logs"
        );
        assert_eq!(json_command_name(&ClientCommand::Doctor("x")), "doctor");
        // `events` is the spec-aligned name for the VmEvents variant.
        assert_eq!(json_command_name(&ClientCommand::VmEvents("x")), "events");
    }

    #[test]
    fn json_ok_with_empty_lines_emits_empty_array() {
        let s = format_json_ok(&ClientCommand::List, &[]);
        assert_eq!(s, "{\"ok\":true,\"command\":\"list\",\"lines\":[]}");
    }

    #[test]
    fn json_err_with_empty_message_still_valid() {
        // The text renderer omits the message when empty; the JSON
        // shape always emits the field — consumers can rely on it.
        let s = format_json_err(&ClientCommand::List, "code_only", "");
        assert!(s.contains("\"message\":\"\""));
    }
}
