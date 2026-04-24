#![cfg_attr(not(target_os = "linux"), no_std)]

use anyos_std::{format, println, process, String, Vec};

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

pub fn run() {
    let mut args_buf = [0u8; 256];
    let raw = process::args(&mut args_buf);
    let Some(command) = parse_cli_command(raw) else {
        print_usage();
        return;
    };

    match send_command(&command) {
        Ok(response) => print_response(&command, &response),
        Err(message) => println!("aslctl: {}", message),
    }
}

pub fn parse_cli_command<'a>(raw: &'a str) -> Option<ClientCommand<'a>> {
    let tokens = tokenize(raw);
    let first = *tokens.first()?;
    match first {
        "delete" => parse_delete_tokens(&tokens),
        "clone" => parse_clone_tokens(&tokens),
        "export" => Some(ClientCommand::Export(*tokens.get(1)?)),
        "config" => parse_config_tokens(&tokens),
        "network" => parse_network_tokens(&tokens),
        "doctor" => Some(ClientCommand::Doctor(*tokens.get(1)?)),
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
        "exec" => parse_exec_tokens(&tokens),
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
        "vm-events" => Some(ClientCommand::VmEvents(args.pos(1)?)),
        "vm-events-tail" => Some(ClientCommand::VmEventsTail {
            distro: args.pos(1)?,
            limit: args.pos(2).unwrap_or("10"),
        }),
        "vm-events-clear" => Some(ClientCommand::VmEventsClear(args.pos(1)?)),
        "diagnose" => Some(ClientCommand::Diagnose(args.pos(1)?)),
        "doctor" => Some(ClientCommand::Doctor(args.pos(1)?)),
        "exec-clear" => Some(ClientCommand::ExecClear(args.pos(1)?)),
        "mount" => parse_mount_command(args),
        "port" => parse_port_command(args),
        _ => None,
    }
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
        let _ = anyos_std::ipc::pipe_close(request_pipe);
        let _ = anyos_std::ipc::pipe_close(reply_pipe);
        return Err("failed to write request");
    }
    let _ = anyos_std::ipc::pipe_close(request_pipe);

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
                println!("distros: {}", count);
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
                println!("mounts: {}", count);
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
                println!("mount-validation: {}", count);
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
                println!("network-validation: {}", count);
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
                println!("storage-validation: {}", count);
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
                println!("ports: {}", count);
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
                println!("vm-events: {}", count);
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
            ClientCommand::VmEventsClear(_)
            | ClientCommand::Diagnose(_)
            | ClientCommand::Doctor(_) => {
                for line in lines {
                    println!("{}", line);
                }
            }
            ClientCommand::ShellList(_) | ClientCommand::ShellClose { .. } => {
                println!("shell-sessions: {}", count);
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
                println!("execs: {}", count);
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

fn print_usage() {
    println!("aslctl - control ASL distros");
    println!();
    println!("Usage:");
    println!("  aslctl list");
    println!("  aslctl status <name>");
    println!("  aslctl create <name> <image-ref> <owner>");
    println!("  aslctl delete <name> [--force]");
    println!("  aslctl clone <source> <target> [--owner <owner>] [--no-mounts]");
    println!("  aslctl export <name>");
    println!("  aslctl config <name>");
    println!("  aslctl config set-resources <name> [--memory-mb <MiB>] [--vcpus <count>]");
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
    println!("  aslctl vm-events <name>");
    println!("  aslctl vm-events-tail <name> [limit]");
    println!("  aslctl vm-events-clear <name>");
    println!("  aslctl diagnose <name>");
    println!("  aslctl doctor <name>");
    println!("  aslctl shell-list <name>");
    println!("  aslctl shell-show <name> <session-id>");
    println!("  aslctl shell-close <name> <session-id>");
    println!("  aslctl shell <name> [--fallback-console] [--session <name>]");
    println!("  aslctl exec-list <name>");
    println!("  aslctl exec-show <name> <exec-id>");
    println!("  aslctl exec-clear <name>");
    println!("  aslctl exec <name> [--fallback-console] [--cwd <path>] [--env KEY=VALUE] -- <command> [args...]");
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
            } => format!("CREATE {} {} {}", name, image_ref, owner),
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
            Self::VmEventsClear(name) => format!("VM_EVENTS_CLEAR {}", name),
            Self::Diagnose(name) => format!("DIAGNOSE {}", name),
            Self::Doctor(name) => format!("DIAGNOSE {}", name),
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
    use super::{parse_cli_command, parse_command, parse_response, ClientCommand, WireResponse};

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
            }) => {
                assert_eq!(name, "ubuntu-dev");
                assert_eq!(image_ref, "ubuntu-24.04-x86_64-v1");
                assert_eq!(owner, "strati");
            }
            _ => panic!("expected create command"),
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
}
