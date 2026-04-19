#![no_std]
#![no_main]

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use libami::{AmiClient, AmiError, AmiValue};
use libconf::{ConfClient, ConfValue, NodeKind, RegistryScope};
use libconf_schema::{default_bool, default_int, default_string, manifest, ServiceSchema};

anyos_std::entry!(main);

const SVC_NAMESPACE: &str = "services";
const THREAD_ENTRY_SIZE: usize = 80;
const MAX_THREADS: usize = 256;
const WATCH_POLL_MS: u32 = 100;

const SERVICE_NAMES: &[&str] = &[
    "crond",
    "dnsd",
    "ftpd",
    "httpd",
    "logd",
    "networkd",
    "sshd",
    "vdagent",
    "vncd",
];

const SVC_DIRS: &[&str] = &[
    "crond",
    "crond/config",
    "dnsd",
    "dnsd/config",
    "ftpd",
    "ftpd/config",
    "httpd",
    "httpd/config",
    "logd",
    "logd/config",
    "networkd",
    "networkd/config",
    "sshd",
    "sshd/config",
    "vdagent",
    "vdagent/config",
    "vncd",
    "vncd/config",
];

const SVC_DEFAULTS: &[libconf_schema::DefaultEntry<'static>] = &[
    default_string("crond/config/exec", "/System/bin/crond"),
    default_string("crond/config/args", ""),
    default_bool("crond/config/enabled", true),
    default_string("crond/config/depends", ""),
    default_string("crond/config/wants", "logd"),
    default_string("crond/config/after", "logd"),
    default_int("crond/config/startup_timeout_ms", 3000),
    default_string("dnsd/config/exec", "/System/bin/dnsd"),
    default_string("dnsd/config/args", ""),
    default_bool("dnsd/config/enabled", true),
    default_string("dnsd/config/depends", "networkd"),
    default_string("dnsd/config/wants", ""),
    default_string("dnsd/config/after", ""),
    default_int("dnsd/config/startup_timeout_ms", 5000),
    default_string("ftpd/config/exec", "/System/bin/ftpd"),
    default_string("ftpd/config/args", ""),
    default_bool("ftpd/config/enabled", true),
    default_string("ftpd/config/depends", ""),
    default_string("ftpd/config/wants", "networkd,logd"),
    default_string("ftpd/config/after", "networkd,logd"),
    default_int("ftpd/config/startup_timeout_ms", 5000),
    default_string("httpd/config/exec", "/System/bin/httpd"),
    default_string("httpd/config/args", ""),
    default_bool("httpd/config/enabled", true),
    default_string("httpd/config/depends", ""),
    default_string("httpd/config/wants", "networkd,logd"),
    default_string("httpd/config/after", "networkd,logd"),
    default_int("httpd/config/startup_timeout_ms", 5000),
    default_string("logd/config/exec", "/System/bin/logd"),
    default_string("logd/config/args", ""),
    default_bool("logd/config/enabled", true),
    default_string("logd/config/depends", ""),
    default_string("logd/config/wants", ""),
    default_string("logd/config/after", ""),
    default_int("logd/config/startup_timeout_ms", 3000),
    default_string("networkd/config/exec", "/System/bin/networkd"),
    default_string("networkd/config/args", ""),
    default_bool("networkd/config/enabled", true),
    default_string("networkd/config/depends", ""),
    default_string("networkd/config/wants", ""),
    default_string("networkd/config/after", ""),
    default_int("networkd/config/startup_timeout_ms", 15000),
    default_string("sshd/config/exec", "/System/bin/sshd"),
    default_string("sshd/config/args", ""),
    default_bool("sshd/config/enabled", true),
    default_string("sshd/config/depends", ""),
    default_string("sshd/config/wants", "networkd,logd"),
    default_string("sshd/config/after", "networkd,logd"),
    default_int("sshd/config/startup_timeout_ms", 0),
    default_string("vdagent/config/exec", "/System/bin/vdagent"),
    default_string("vdagent/config/args", ""),
    default_bool("vdagent/config/enabled", true),
    default_string("vdagent/config/depends", ""),
    default_string("vdagent/config/wants", ""),
    default_string("vdagent/config/after", ""),
    default_int("vdagent/config/startup_timeout_ms", 5000),
    default_string("vncd/config/exec", "/System/bin/vncd"),
    default_string("vncd/config/args", ""),
    default_bool("vncd/config/enabled", true),
    default_string("vncd/config/depends", ""),
    default_string("vncd/config/wants", "networkd,logd"),
    default_string("vncd/config/after", "networkd,logd"),
    default_int("vncd/config/startup_timeout_ms", 5000),
];

const SVC_MANIFEST: libconf_schema::RegistryManifest<'static> = manifest(
    SVC_NAMESPACE,
    RegistryScope::System,
    1,
    SVC_DIRS,
    SVC_DEFAULTS,
    &[],
);

const SVC_SCHEMA: ServiceSchema<'static> = ServiceSchema::new("svc", &SVC_MANIFEST);

struct ServiceConfig {
    exec: String,
    args: String,
    enabled: bool,
    depends: String,
    wants: String,
    after: String,
    startup_timeout_ms: u32,
}

fn register_manifest() {
    anyos_std::println!("svc: register_manifest begin");
    let _ = SVC_SCHEMA.register();
    anyos_std::println!("svc: register_manifest end");
}

fn service_path(name: &str, field: &str) -> String {
    format!("{}/{}/config/{}", SVC_NAMESPACE, name, field)
}

fn read_service_string(client: &mut ConfClient, name: &str, field: &str) -> Option<String> {
    match client.get(RegistryScope::System, &service_path(name, field)).ok()?.value {
        Some(libconf::ConfValue::String(value)) => Some(value),
        Some(libconf::ConfValue::ExternalRef(value)) => Some(value),
        _ => None,
    }
}

fn read_service_u32(client: &mut ConfClient, name: &str, field: &str) -> Option<u32> {
    match client.get(RegistryScope::System, &service_path(name, field)).ok()?.value {
        Some(libconf::ConfValue::Int(value)) if value >= 0 => Some(value as u32),
        _ => None,
    }
}

fn read_service_bool(client: &mut ConfClient, name: &str, field: &str) -> Option<bool> {
    match client.get(RegistryScope::System, &service_path(name, field)).ok()?.value {
        Some(libconf::ConfValue::Bool(value)) => Some(value),
        Some(libconf::ConfValue::Int(value)) => Some(value != 0),
        _ => None,
    }
}

fn read_config(name: &str) -> Option<ServiceConfig> {
    anyos_std::println!("svc: read_config('{}') begin", name);
    register_manifest();
    anyos_std::println!("svc: read_config('{}') connect", name);
    let mut client = ConfClient::connect("svc").ok()?;
    anyos_std::println!("svc: read_config('{}') connected", name);
    read_config_with_client(&mut client, name)
}

fn read_config_with_client(client: &mut ConfClient, name: &str) -> Option<ServiceConfig> {
    let exec = read_service_string(client, name, "exec")?;
    anyos_std::println!("svc: read_config('{}') exec='{}'", name, exec);
    let args = read_service_string(client, name, "args").unwrap_or_default();
    let enabled = read_service_bool(client, name, "enabled").unwrap_or(true);
    let removed = read_service_bool(client, name, "removed").unwrap_or(false);
    let depends = read_service_string(client, name, "depends").unwrap_or_default();
    let wants = read_service_string(client, name, "wants").unwrap_or_default();
    let after = read_service_string(client, name, "after").unwrap_or_default();
    let startup_timeout_ms = read_service_u32(client, name, "startup_timeout_ms").unwrap_or(0);

    if exec.is_empty() || removed {
        return None;
    }

    Some(ServiceConfig {
        exec,
        args,
        enabled,
        depends,
        wants,
        after,
        startup_timeout_ms,
    })
}

fn parse_list(value: &str) -> Vec<String> {
    let mut items = Vec::new();
    for item in value.split(',') {
        let item = item.trim();
        if !item.is_empty() {
            push_unique(&mut items, item);
        }
    }
    items
}

fn list_service_names(client: &mut ConfClient) -> Vec<String> {
    let mut emitted = Vec::new();
    let mut discovered = Vec::new();

    if let Ok(items) = client.list_children(RegistryScope::System, SVC_NAMESPACE) {
        anyos_std::println!("svc: list_service_names -> {} items", items.len());
        for item in items {
            if item.kind != NodeKind::Directory {
                continue;
            }
            let Some(rest) = item.path.strip_prefix("services/") else {
                continue;
            };
            let Some(name) = rest.split('/').next() else {
                continue;
            };
            if name.is_empty() || contains_name(&emitted, name) {
                continue;
            }
            emitted.push(String::from(name));
            discovered.push(String::from(name));
        }
    }

    for name in SERVICE_NAMES {
        if !contains_name(&emitted, name) {
            emitted.push(String::from(*name));
            discovered.push(String::from(*name));
        }
    }

    discovered
}

fn for_each_service(mut f: impl FnMut(&str)) {
    anyos_std::println!("svc: for_each_service begin");
    register_manifest();
    anyos_std::println!("svc: for_each_service connect");
    let Ok(mut client) = ConfClient::connect("svc") else {
        anyos_std::println!("svc: for_each_service connect FAILED");
        return;
    };
    anyos_std::println!("svc: for_each_service connected");

    let discovered = list_service_names(&mut client);
    for name in &discovered {
        f(name);
    }
    anyos_std::println!("svc: for_each_service end");
}

fn collect_enabled_service_names() -> Vec<String> {
    anyos_std::println!("svc: collect_enabled_service_names begin");
    let mut names = Vec::new();
    register_manifest();
    let mut client = match ConfClient::connect("svc") {
        Ok(client) => client,
        Err(_) => return names,
    };
    let discovered = list_service_names(&mut client);
    for name in &discovered {
        if read_config_with_client(&mut client, name)
            .map(|config| config.enabled)
            .unwrap_or(false)
        {
            names.push(name.clone());
        }
    }
    anyos_std::println!("svc: collect_enabled_service_names -> {}", names.len());
    names
}

fn topo_sort_services(names: &[String]) -> Option<Vec<String>> {
    let mut order = Vec::new();
    let mut visiting = Vec::new();
    let mut visited = Vec::new();

    for name in names {
        if !visit_service(name, names, &mut visiting, &mut visited, &mut order) {
            return None;
        }
    }

    Some(order)
}

fn visit_service(
    name: &str,
    names: &[String],
    visiting: &mut Vec<String>,
    visited: &mut Vec<String>,
    order: &mut Vec<String>,
) -> bool {
    if contains_name(visited, name) {
        return true;
    }
    if contains_name(visiting, name) {
        anyos_std::println!(
            "svc: dependency cycle detected at '{}' - ignoring one back-edge and continuing",
            name
        );
        return true;
    }

    visiting.push(String::from(name));

    if let Some(config) = read_config(name) {
        let mut edges = parse_list(&config.depends);
        for want in parse_list(&config.wants) {
            push_unique(&mut edges, &want);
        }
        for after in parse_list(&config.after) {
            push_unique(&mut edges, &after);
        }

        for dep in edges {
            if contains_name(names, &dep)
                && !visit_service(&dep, names, visiting, visited, order)
            {
                return false;
            }
        }
    }

    let _ = visiting.pop();
    visited.push(String::from(name));
    order.push(String::from(name));
    true
}

struct WaveService {
    name: String,
    config: ServiceConfig,
    was_running: bool,
}

fn dependencies_satisfied_for_wave(
    name: &str,
    active_names: &[String],
    finished: &[String],
) -> bool {
    let Some(config) = read_config(name) else {
        return false;
    };
    let mut predecessors = parse_list(&config.depends);
    for want in parse_list(&config.wants) {
        push_unique(&mut predecessors, &want);
    }
    for after in parse_list(&config.after) {
        push_unique(&mut predecessors, &after);
    }
    predecessors
        .into_iter()
        .filter(|dep| contains_name(active_names, dep))
        .all(|dep| contains_name(finished, &dep))
}

fn check_service_preconditions(
    name: &str,
    config: &ServiceConfig,
    finished: &[String],
    succeeded: &[String],
) -> bool {
    for dep in parse_list(&config.depends) {
        let dep_config = match read_config(&dep) {
            Some(dep_config) => dep_config,
            None => {
                anyos_std::println!("svc: required dependency '{}' has no config", dep);
                return false;
            }
        };
        if !dep_config.enabled && find_thread_by_name(&dep) == 0 {
            anyos_std::println!("svc: required dependency '{}' is disabled", dep);
            return false;
        }
        if !contains_name(succeeded, &dep) && find_thread_by_name(&dep) == 0 {
            anyos_std::println!("svc: dependency '{}' for '{}' is not ready", dep, name);
            return false;
        }
    }

    for dep in parse_list(&config.after) {
        if contains_name(finished, &dep) || find_thread_by_name(&dep) != 0 {
            continue;
        }
        if read_config(&dep).is_some() {
            anyos_std::println!("svc: after dependency '{}' for '{}' is not ready", dep, name);
            return false;
        }
    }

    true
}

fn wait_for_wave(
    wave: &[WaveService],
    finished: &mut Vec<String>,
    succeeded: &mut Vec<String>,
    started_count: &mut u32,
    already_count: &mut u32,
) {
    for service in wave {
        let ok = if service.was_running {
            wait_for_service_start(&service.name, &service.config)
        } else {
            wait_for_service_start(&service.name, &service.config)
        };

        push_unique(finished, &service.name);
        if ok {
            push_unique(succeeded, &service.name);
            if service.was_running {
                *already_count = already_count.wrapping_add(1);
            } else {
                *started_count = started_count.wrapping_add(1);
            }
        }
    }
}

fn cmd_start_all_parallel() {
    let names = collect_enabled_service_names();
    let order = match topo_sort_services(&names) {
        Some(order) => order,
        None => return,
    };

    let mut remaining = order;
    let mut finished = Vec::new();
    let mut succeeded = Vec::new();
    let mut started = 0u32;
    let mut already = 0u32;

    while !remaining.is_empty() {
        let active_names = remaining.clone();
        let mut wave_names = Vec::new();
        for name in &remaining {
            if dependencies_satisfied_for_wave(name, &active_names, &finished) {
                wave_names.push(name.clone());
            }
        }

        if wave_names.is_empty() {
            let forced = remaining.remove(0);
            anyos_std::println!(
                "svc: dependency cycle or unresolved optional ordering around '{}' - forcing progress",
                forced
            );
            wave_names.push(forced);
        } else {
            remaining.retain(|name| !contains_name(&wave_names, name));
        }

        let mut wave = Vec::new();
        for name in &wave_names {
            let Some(config) = read_config(name) else {
                anyos_std::println!("svc: unknown service '{}' (no registry config in {})", name, SVC_NAMESPACE);
                push_unique(&mut finished, name);
                continue;
            };
            if !check_service_preconditions(name, &config, &finished, &succeeded) {
                push_unique(&mut finished, name);
                continue;
            }

            let was_running = find_thread_by_name(name) != 0;
            if !was_running && spawn_service(name, &config, "").is_none() {
                push_unique(&mut finished, name);
                continue;
            }

            wave.push(WaveService {
                name: name.clone(),
                config,
                was_running,
            });
        }

        wait_for_wave(&wave, &mut finished, &mut succeeded, &mut started, &mut already);
    }

    anyos_std::println!("svc: {} started, {} already running", started, already);
}

fn ensure_service_started(
    name: &str,
    extra_args: &str,
    started: &mut Vec<String>,
    stack: &mut Vec<String>,
) -> bool {
    if contains_name(started, name) {
        return true;
    }
    if contains_name(stack, name) {
        anyos_std::println!(
            "svc: dependency cycle detected while starting '{}' - ignoring recursive back-edge",
            name
        );
        return true;
    }

    let config = match read_config(name) {
        Some(cfg) => cfg,
        None => {
            anyos_std::println!("svc: unknown service '{}' (no registry config in {})", name, SVC_NAMESPACE);
            return false;
        }
    };

    stack.push(String::from(name));

    for dep in parse_list(&config.depends) {
        if !ensure_named_dependency(&dep, true, true, started, stack) {
            let _ = stack.pop();
            return false;
        }
    }

    for dep in parse_list(&config.wants) {
        let _ = ensure_named_dependency(&dep, false, true, started, stack);
    }

    for dep in parse_list(&config.after) {
        if !ensure_named_dependency(&dep, false, false, started, stack) {
            let _ = stack.pop();
            return false;
        }
    }

    let result = if find_thread_by_name(name) != 0 {
        wait_for_service_start(name, &config)
    } else {
        match spawn_service(name, &config, extra_args) {
            Some(_) => wait_for_service_start(name, &config),
            None => false,
        }
    };

    let _ = stack.pop();

    if result {
        push_unique(started, name);
    }

    result
}

fn ensure_named_dependency(
    name: &str,
    required: bool,
    start_if_missing: bool,
    started: &mut Vec<String>,
    stack: &mut Vec<String>,
) -> bool {
    let config = match read_config(name) {
        Some(cfg) => cfg,
        None => {
            if required {
                anyos_std::println!("svc: required dependency '{}' has no config", name);
                anyos_std::println!("svc: expected path {}/{}", SVC_NAMESPACE, name);
                return false;
            }
            return true;
        }
    };

    if start_if_missing {
        if !ensure_service_started(name, "", started, stack) && required {
            return false;
        }
        return true;
    }

    if find_thread_by_name(name) != 0 && !wait_for_service_start(name, &config) {
        if required {
            anyos_std::println!("svc: dependency '{}' is not ready", name);
            return false;
        }
    }

    true
}

fn spawn_service(name: &str, config: &ServiceConfig, extra_args: &str) -> Option<u32> {
    let mut stat_buf = [0u32; 7];
    if anyos_std::fs::stat(&config.exec, &mut stat_buf) != 0 {
        anyos_std::println!(
            "svc: cannot start '{}' - executable missing: {}",
            name,
            config.exec
        );
        return None;
    }

    let mut args = String::from(config.exec.as_str());
    if !config.args.is_empty() {
        args.push(' ');
        args.push_str(&config.args);
    }
    if !extra_args.is_empty() {
        args.push(' ');
        args.push_str(extra_args);
    }

    let tid = anyos_std::process::spawn(&config.exec, &args);
    if tid == 0 || tid == u32::MAX {
        anyos_std::println!("svc: failed to start {} ({})", name, config.exec);
        return None;
    }

    anyos_std::process::detach(tid);
    anyos_std::println!("{}: started (TID {})", name, tid);
    Some(tid)
}

fn wait_for_service_start(name: &str, config: &ServiceConfig) -> bool {
    if config.startup_timeout_ms == 0 {
        return find_thread_by_name(name) != 0;
    }
    wait_for_service_ready(name, config.startup_timeout_ms)
}

fn wait_for_service_ready(name: &str, timeout_ms: u32) -> bool {
    let prefix = format!("svc.{}.", name);
    let ready_key = format!("svc.{}.ready", name);
    let state_key = format!("svc.{}.state", name);
    let error_key = format!("svc.{}.error", name);

    let mut ami = match AmiClient::connect("svc") {
        Ok(client) => client,
        Err(_) => return wait_for_thread_liveness(name, timeout_ms),
    };

    let watch_id = ami.watch(&prefix).ok();
    if let Some(done) = read_service_readiness(&mut ami, &ready_key, &state_key, &error_key, name) {
        if let Some(id) = watch_id {
            let _ = ami.unwatch(id);
        }
        return done;
    }

    let deadline = anyos_std::sys::uptime_ms().wrapping_add(timeout_ms);
    loop {
        if find_thread_by_name(name) == 0 {
            anyos_std::println!("svc: '{}' exited before becoming ready", name);
            if let Some(id) = watch_id {
                let _ = ami.unwatch(id);
            }
            return false;
        }

        if deadline_reached(anyos_std::sys::uptime_ms(), deadline) {
            anyos_std::println!("svc: timeout waiting for '{}' readiness", name);
            if let Some(id) = watch_id {
                let _ = ami.unwatch(id);
            }
            return false;
        }

        match ami.poll_event(WATCH_POLL_MS) {
            Ok(Some(_)) | Ok(None) => {
                if let Some(done) =
                    read_service_readiness(&mut ami, &ready_key, &state_key, &error_key, name)
                {
                    if let Some(id) = watch_id {
                        let _ = ami.unwatch(id);
                    }
                    return done;
                }
            }
            Err(_) => {
                if let Some(id) = watch_id {
                    let _ = ami.unwatch(id);
                }
                return wait_for_thread_liveness(name, timeout_ms);
            }
        }
    }
}

fn read_service_readiness(
    ami: &mut AmiClient,
    ready_key: &str,
    state_key: &str,
    error_key: &str,
    service: &str,
) -> Option<bool> {
    if let Ok(item) = ami.get(ready_key) {
        if matches!(item.value, AmiValue::Bool(true)) {
            return Some(true);
        }
    }

    match ami.get(state_key) {
        Ok(item) => match item.value {
            AmiValue::String(ref state) if state == "ready" => return Some(true),
            AmiValue::String(ref state) if state == "failed" => {
                if let Some(error) = read_service_error(ami, error_key) {
                    anyos_std::println!("svc: '{}' failed to start: {}", service, error);
                } else {
                    anyos_std::println!("svc: '{}' reported startup failure", service);
                }
                return Some(false);
            }
            _ => {}
        },
        Err(err) if !is_not_found_error(&err) => return Some(false),
        Err(_) => {}
    }

    None
}

fn read_service_error(ami: &mut AmiClient, error_key: &str) -> Option<String> {
    let item = ami.get(error_key).ok()?;
    match item.value {
        AmiValue::String(message) if !message.is_empty() => Some(message),
        _ => None,
    }
}

fn is_not_found_error(err: &AmiError) -> bool {
    matches!(err, AmiError::Remote(message) if message == "not_found")
}

fn wait_for_thread_liveness(name: &str, timeout_ms: u32) -> bool {
    let deadline = anyos_std::sys::uptime_ms().wrapping_add(timeout_ms);
    loop {
        if find_thread_by_name(name) != 0 {
            return true;
        }
        if deadline_reached(anyos_std::sys::uptime_ms(), deadline) {
            anyos_std::println!("svc: timeout waiting for '{}' thread", name);
            return false;
        }
        anyos_std::process::sleep(20);
    }
}

fn deadline_reached(now: u32, deadline: u32) -> bool {
    now.wrapping_sub(deadline) < 0x8000_0000
}

fn find_thread_by_name(name: &str) -> u32 {
    let mut buf = [0u8; THREAD_ENTRY_SIZE * MAX_THREADS];
    let count = anyos_std::sys::sysinfo(1, &mut buf) as usize;
    let name_bytes = name.as_bytes();

    for i in 0..count {
        let off = i * THREAD_ENTRY_SIZE;
        if off + THREAD_ENTRY_SIZE > buf.len() {
            break;
        }
        let name_start = off + 8;
        let mut len = 0usize;
        for j in 0..23 {
            if buf[name_start + j] == 0 {
                break;
            }
            len += 1;
        }
        if len == name_bytes.len() && &buf[name_start..name_start + len] == name_bytes {
            let state = buf[off + 5];
            if state <= 2 {
                return u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]);
            }
        }
    }

    0
}

fn read_service_state(name: &str) -> Option<String> {
    let mut ami = AmiClient::connect("svc").ok()?;
    let key = format!("svc.{}.state", name);
    match ami.get(&key).ok()?.value {
        AmiValue::String(state) => Some(state),
        _ => None,
    }
}

fn cmd_list() {
    let mut found = false;
    anyos_std::println!("{:<16} {:<10} {}", "SERVICE", "STATUS", "EXEC");
    anyos_std::println!("{:<16} {:<10} {}", "-------", "------", "----");
    for_each_service(|name| {
        found = true;
        let exec_str = match read_config(name) {
            Some(cfg) => cfg.exec,
            None => String::from("(invalid config)"),
        };
        let tid = find_thread_by_name(name);
        let status = match read_config(name) {
            Some(cfg) if !cfg.enabled && tid == 0 => String::from("disabled"),
            _ if tid == 0 => String::from("stopped"),
            _ => read_service_state(name).unwrap_or_else(|| String::from("running")),
        };
        anyos_std::println!("{:<16} {:<10} {}", name, status, exec_str);
    });
    if !found {
        anyos_std::println!("No services configured in {}", SVC_NAMESPACE);
    }
}

fn cmd_start_all() {
    anyos_std::println!("svc: start-all begin");
    let names = collect_enabled_service_names();
    anyos_std::println!("svc: start-all names collected");
    let order = match topo_sort_services(&names) {
        Some(order) => order,
        None => return,
    };
    anyos_std::println!("svc: start-all topo order -> {}", order.len());

    let mut started = Vec::new();
    let mut started_count = 0u32;
    let mut already_count = 0u32;
    for name in &order {
        let was_running = find_thread_by_name(name) != 0;
        if ensure_service_started(name, "", &mut started, &mut Vec::new()) {
            if was_running {
                already_count = already_count.wrapping_add(1);
            } else {
                started_count = started_count.wrapping_add(1);
            }
        }
    }

    anyos_std::println!(
        "svc: {} started, {} already running",
        started_count,
        already_count,
    );
}

fn cmd_start(name: &str, extra_args: &str) {
    let existing = find_thread_by_name(name);
    if existing != 0 {
        if let Some(config) = read_config(name) {
            anyos_std::println!("{}: already running (TID {})", name, existing);
            let _ = wait_for_service_start(name, &config);
        } else {
            anyos_std::println!("svc: unknown service '{}' (no registry config in {})", name, SVC_NAMESPACE);
        }
        return;
    }

    if ensure_service_started(name, extra_args, &mut Vec::new(), &mut Vec::new()) {
        if let Some(config) = read_config(name) {
            if config.startup_timeout_ms > 0 {
                anyos_std::println!("{}: ready", name);
            }
        }
    }
}

fn cmd_stop(name: &str) {
    let tid = find_thread_by_name(name);
    if tid == 0 {
        anyos_std::println!("{}: not running", name);
        return;
    }

    anyos_std::process::kill(tid);
    anyos_std::println!("{}: stopped (TID {})", name, tid);
}

fn cmd_status(name: &str) {
    let tid = find_thread_by_name(name);
    if tid != 0 {
        if let Some(state) = read_service_state(name) {
            anyos_std::println!("{}: {} (TID {})", name, state, tid);
        } else {
            anyos_std::println!("{}: running (TID {})", name, tid);
        }
    } else {
        anyos_std::println!("{}: stopped", name);
    }
}

fn cmd_restart(name: &str, extra_args: &str) {
    cmd_stop(name);
    cmd_start(name, extra_args);
}

fn ensure_service_dirs(client: &mut ConfClient, name: &str) -> bool {
    for path in [format!("{}/{}", SVC_NAMESPACE, name), format!("{}/{}/config", SVC_NAMESPACE, name)] {
        if client.mkdir(RegistryScope::System, &path).is_err() {
            let exists = client
                .get(RegistryScope::System, &path)
                .map(|item| item.kind == NodeKind::Directory)
                .unwrap_or(false);
            if !exists {
                anyos_std::println!("svc: failed to create '{}'", path);
                return false;
            }
        }
    }
    true
}

fn write_service_value(client: &mut ConfClient, name: &str, field: &str, value: ConfValue) -> bool {
    client
        .set(RegistryScope::System, &service_path(name, field), value)
        .is_ok()
}

fn cmd_install(name: &str, exec: &str, extra_args: &str) {
    register_manifest();
    let mut client = match ConfClient::connect("svc") {
        Ok(client) => client,
        Err(_) => {
            anyos_std::println!("svc: confd is not available");
            return;
        }
    };

    let existing = read_config(name);
    if !ensure_service_dirs(&mut client, name) {
        return;
    }

    let effective_exec = if !exec.is_empty() {
        exec
    } else if let Some(cfg) = existing.as_ref() {
        cfg.exec.as_str()
    } else {
        anyos_std::println!("svc: install for new service '{}' requires an executable path", name);
        return;
    };

    let effective_args = if !extra_args.is_empty() {
        extra_args
    } else if let Some(cfg) = existing.as_ref() {
        cfg.args.as_str()
    } else {
        ""
    };

    let mut ok = true;
    ok &= write_service_value(&mut client, name, "exec", ConfValue::String(String::from(effective_exec)));
    ok &= write_service_value(&mut client, name, "args", ConfValue::String(String::from(effective_args)));
    ok &= write_service_value(&mut client, name, "enabled", ConfValue::Bool(true));
    ok &= write_service_value(&mut client, name, "removed", ConfValue::Bool(false));

    if existing.is_none() {
        ok &= write_service_value(&mut client, name, "depends", ConfValue::String(String::new()));
        ok &= write_service_value(&mut client, name, "wants", ConfValue::String(String::new()));
        ok &= write_service_value(&mut client, name, "after", ConfValue::String(String::new()));
        ok &= write_service_value(&mut client, name, "startup_timeout_ms", ConfValue::Int(5_000));
    }

    if ok {
        anyos_std::println!("svc: '{}' installed", name);
    } else {
        anyos_std::println!("svc: failed to install '{}'", name);
    }
}

fn cmd_uninstall(name: &str) {
    register_manifest();
    let mut client = match ConfClient::connect("svc") {
        Ok(client) => client,
        Err(_) => {
            anyos_std::println!("svc: confd is not available");
            return;
        }
    };

    if read_config(name).is_none() {
        anyos_std::println!("svc: unknown service '{}' (no registry config in {})", name, SVC_NAMESPACE);
        return;
    }

    match client.set(RegistryScope::System, &service_path(name, "enabled"), ConfValue::Bool(false)) {
        Ok(_) => anyos_std::println!("svc: '{}' uninstalled from startup", name),
        Err(_) => anyos_std::println!("svc: failed to uninstall '{}'", name),
    }
}

fn cmd_set_enabled(name: &str, enabled: bool) {
    register_manifest();
    let mut client = match ConfClient::connect("svc") {
        Ok(client) => client,
        Err(_) => {
            anyos_std::println!("svc: confd is not available");
            return;
        }
    };

    if read_config(name).is_none() {
        anyos_std::println!("svc: unknown service '{}' (no registry config in {})", name, SVC_NAMESPACE);
        return;
    }

    let mut ok = true;
    ok &= client
        .set(
            RegistryScope::System,
            &service_path(name, "enabled"),
            ConfValue::Bool(enabled),
        )
        .is_ok();
    ok &= client
        .set(
            RegistryScope::System,
            &service_path(name, "removed"),
            ConfValue::Bool(false),
        )
        .is_ok();

    if ok {
        anyos_std::println!(
            "svc: '{}' {}",
            name,
            if enabled { "enabled for auto-start" } else { "disabled for auto-start" }
        );
    } else {
        anyos_std::println!("svc: failed to update '{}'", name);
    }
}

fn cmd_remove(name: &str) {
    register_manifest();
    let mut client = match ConfClient::connect("svc") {
        Ok(client) => client,
        Err(_) => {
            anyos_std::println!("svc: confd is not available");
            return;
        }
    };

    let existed = read_config(name).is_some()
        || client
            .get(RegistryScope::System, &format!("{}/{}", SVC_NAMESPACE, name))
            .is_ok();
    if !existed {
        anyos_std::println!("svc: unknown service '{}' (no registry config in {})", name, SVC_NAMESPACE);
        return;
    }

    if find_thread_by_name(name) != 0 {
        cmd_stop(name);
    }

    let mut ok = true;
    ok &= ensure_service_dirs(&mut client, name);
    ok &= client
        .set(
            RegistryScope::System,
            &service_path(name, "enabled"),
            ConfValue::Bool(false),
        )
        .is_ok();
    ok &= client
        .set(
            RegistryScope::System,
            &service_path(name, "removed"),
            ConfValue::Bool(true),
        )
        .is_ok();

    if ok {
        anyos_std::println!("svc: '{}' removed", name);
    } else {
        anyos_std::println!("svc: failed to remove '{}'", name);
    }
}

fn main() {
    anyos_std::println!("svc: main entered");
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    anyos_std::println!("svc: raw args='{}'", raw);
    if raw.contains("--help") {
        anyos_std::println!("svc - Service manager\n\nUsage: svc COMMAND [SERVICE]");
        return;
    }

    let mut arg_buf = [0u8; 256];
    let args_str = anyos_std::process::args(&mut arg_buf);
    let parts: Vec<&str> = args_str.split_whitespace().collect();
    anyos_std::println!("svc: parsed {} arg parts", parts.len());

    if parts.is_empty() || (parts.len() < 2 && !matches!(parts[0], "list" | "start-all")) {
        anyos_std::println!("Usage: svc <command> [service] [args...]");
        anyos_std::println!("");
        anyos_std::println!("Commands:");
        anyos_std::println!("  start <service> [args]   Start a service");
        anyos_std::println!("  stop <service>           Stop a running service");
        anyos_std::println!("  status <service>         Check if a service is running");
        anyos_std::println!("  restart <service> [args] Restart a service");
        anyos_std::println!("  install <service> [exec] [args]");
        anyos_std::println!("                           Install or enable a service");
        anyos_std::println!("  uninstall <service>      Remove a service from auto-start");
        anyos_std::println!("  enable <service>         Enable auto-start for a service");
        anyos_std::println!("  disable <service>        Disable auto-start for a service");
        anyos_std::println!("  remove <service>         Remove a service from the registry");
        anyos_std::println!("  list                     List all configured services");
        anyos_std::println!("  start-all                Start all configured services");
        anyos_std::println!("");
        anyos_std::println!("Services are stored in confd under {}/", SVC_NAMESPACE);
        return;
    }

    let cmd = parts[0];
    if cmd == "list" {
        cmd_list();
        return;
    }
    if cmd == "start-all" {
        cmd_start_all();
        return;
    }

    let name = parts[1];
    let extra = if parts.len() > 2 {
        let mut s = String::new();
        for part in parts.iter().skip(2) {
            if !s.is_empty() {
                s.push(' ');
            }
            s.push_str(part);
        }
        s
    } else {
        String::new()
    };

    match cmd {
        "start" => cmd_start(name, &extra),
        "stop" => cmd_stop(name),
        "status" => cmd_status(name),
        "restart" => cmd_restart(name, &extra),
        "install" => {
            let exec = if parts.len() > 2 { parts[2] } else { "" };
            let install_args = if parts.len() > 3 {
                let mut s = String::new();
                for part in parts.iter().skip(3) {
                    if !s.is_empty() {
                        s.push(' ');
                    }
                    s.push_str(part);
                }
                s
            } else {
                String::new()
            };
            cmd_install(name, exec, &install_args);
        }
        "uninstall" => cmd_uninstall(name),
        "enable" => cmd_set_enabled(name, true),
        "disable" => cmd_set_enabled(name, false),
        "remove" => cmd_remove(name),
        _ => anyos_std::println!(
            "svc: unknown command '{}' (use start/stop/status/restart/install/uninstall/enable/disable/remove/list)",
            cmd
        ),
    }
}

fn contains_name(list: &[String], name: &str) -> bool {
    list.iter().any(|item| item == name)
}

fn push_unique(list: &mut Vec<String>, name: &str) {
    if !contains_name(list, name) {
        list.push(String::from(name));
    }
}
