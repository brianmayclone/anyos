#![cfg_attr(not(feature = "host"), no_std)]
#![cfg_attr(not(feature = "host"), no_main)]

use alloc::string::String;
use libnode::npm::{
    InstallManifestOptions, PackageInstaller, PackageManifest, PackageSpec, RegistryClient,
    RegistryConfig,
};

mod cli;

#[cfg(feature = "host")]
extern crate alloc;

anyos_std::entry!(npm_main);

fn npm_main() -> u32 {
    let mut args_buf = [0u8; 1024];
    let raw = anyos_std::process::args(&mut args_buf);
    let cli = match cli::parse(raw) {
        Ok(cli) => cli,
        Err(err) => {
            anyos_std::println!("npm: {}", err);
            return 1;
        }
    };
    let registry = RegistryConfig { url: cli.registry };
    let global_prefix = configured_global_prefix(cli.prefix);

    match cli.command {
        cli::NpmCommand::Help => usage(),
        cli::NpmCommand::Version => anyos_std::println!("{}", libnode::VERSION),
        cli::NpmCommand::Init { yes } => npm_init(yes),
        cli::NpmCommand::Install { packages } => {
            if cli.global {
                npm_install_global(&packages, registry, &global_prefix);
            } else if packages.is_empty() {
                npm_install_manifest(registry, cli.include_dev);
            } else {
                for package in packages {
                    npm_install(&package, registry.clone(), cli.save_dev);
                }
            }
        }
        cli::NpmCommand::Uninstall { packages } => npm_uninstall(&packages),
        cli::NpmCommand::Update { packages } => npm_update(&packages, registry),
        cli::NpmCommand::Run { script, args } => return npm_run_script(&script, &args),
        cli::NpmCommand::List => npm_list(),
        cli::NpmCommand::Outdated => npm_outdated(registry),
        cli::NpmCommand::Info { package } => {
            let client = RegistryClient::new(registry);
            match client.fetch_metadata(&package) {
                Some(metadata) => {
                    let latest = metadata
                        .resolve_version("latest")
                        .unwrap_or_else(|| String::from("unknown"));
                    anyos_std::println!("{} latest {}", package, latest);
                    anyos_std::println!("{}", client.package_metadata_url(&package));
                }
                None => anyos_std::println!("npm: could not fetch {}", package),
            }
        }
        cli::NpmCommand::Search { query } => {
            anyos_std::println!("Searching {} for '{}'", registry.normalized_url(), query);
            anyos_std::println!("network registry search is planned for npm transport v1");
        }
    }
    0
}

fn npm_init(_yes: bool) {
    if anyos_std::fs::read_to_string("package.json").is_ok() {
        anyos_std::println!("package.json already exists");
        return;
    }
    let manifest = PackageManifest::new_app("anyos-js-app");
    if anyos_std::fs::write_bytes("package.json", manifest.as_str().as_bytes()).is_ok() {
        anyos_std::println!("created package.json");
    } else {
        anyos_std::println!("npm: could not write package.json");
    }
}

fn npm_uninstall(packages: &[String]) {
    if packages.is_empty() {
        anyos_std::println!("npm: uninstall requires one or more packages");
        return;
    }
    let data = anyos_std::fs::read_to_string("package.json").ok();
    let mut manifest = PackageManifest::parse_or_new(data);
    let mut changed = false;
    for package in packages {
        let spec = PackageSpec::parse(package);
        if manifest.remove_dependency(&spec.name) {
            anyos_std::println!("removed {}", spec.name);
            changed = true;
        } else {
            anyos_std::println!("up to date, audited 0 packages");
        }
    }
    if changed && anyos_std::fs::write_bytes("package.json", manifest.as_str().as_bytes()).is_err()
    {
        anyos_std::println!("npm: could not update package.json");
    }
}

fn npm_update(packages: &[String], registry: RegistryConfig) {
    if packages.is_empty() {
        npm_install_manifest(registry, true);
    } else {
        for package in packages {
            npm_install(package, registry.clone(), false);
        }
    }
}

fn npm_install_global(packages: &[String], registry: RegistryConfig, prefix: &str) {
    if packages.is_empty() {
        anyos_std::println!("npm: global install requires one or more packages");
        return;
    }
    let installer = PackageInstaller::new(registry);
    for package in packages {
        let spec = PackageSpec::parse(package);
        match installer.install_global_package_result(prefix, &spec) {
            Ok(report) => {
                anyos_std::println!("added {}@{} -g", spec.name, spec.version);
                anyos_std::println!("installed packages: {}", report.installed.len());
                anyos_std::println!("{}/bin", prefix.trim_end_matches('/'));
            }
            Err(err) => anyos_std::println!("npm: {}", err),
        }
    }
}

fn npm_list() {
    let data = anyos_std::fs::read_to_string("package.json").ok();
    let manifest = PackageManifest::parse_or_new(data);
    for dep in manifest.manifest_dependencies(true) {
        anyos_std::println!("{}@{}", dep.name, dep.version);
    }
}

fn configured_global_prefix(cli_prefix: Option<String>) -> String {
    if let Some(prefix) = cli_prefix {
        return prefix;
    }
    let mut buf = [0u8; 512];
    let len = anyos_std::env::get("NPM_CONFIG_PREFIX", &mut buf);
    if len != u32::MAX && len > 0 {
        let len = (len as usize).min(buf.len());
        let value = core::str::from_utf8(&buf[..len])
            .unwrap_or("/System")
            .trim_end_matches('\0');
        if !value.is_empty() {
            return String::from(value);
        }
    }
    String::from("/System")
}

fn npm_outdated(registry: RegistryConfig) {
    let data = anyos_std::fs::read_to_string("package.json").ok();
    let manifest = PackageManifest::parse_or_new(data);
    let client = RegistryClient::new(registry);
    for dep in manifest.manifest_dependencies(true) {
        match client.fetch_metadata(&dep.name) {
            Some(metadata) => {
                let latest = metadata
                    .resolve_version("latest")
                    .unwrap_or_else(|| String::from("unknown"));
                anyos_std::println!("{} current {} latest {}", dep.name, dep.version, latest);
            }
            None => anyos_std::println!("{} current {} latest unknown", dep.name, dep.version),
        }
    }
}

fn npm_install(package: &str, registry: RegistryConfig, save_dev: bool) {
    let spec = PackageSpec::parse(package);
    let data = anyos_std::fs::read_to_string("package.json").ok();
    let mut manifest = PackageManifest::parse_or_new(data);
    let client = RegistryClient::new(registry.clone());
    let resolved = client.fetch_metadata(&spec.name).and_then(|metadata| {
        let version = metadata.resolve_version(&spec.version)?;
        let tarball = metadata.tarball_url(&version);
        let deps = metadata.dependencies(&version);
        Some((version, tarball, deps))
    });
    let install_spec = if let Some((version, tarball, deps)) = resolved {
        anyos_std::println!("resolved {}@{}", spec.name, version);
        if let Some(tarball) = tarball {
            anyos_std::println!("tarball: {}", tarball);
        }
        if !deps.is_empty() {
            anyos_std::println!("dependencies: {}", deps.len());
        }
        PackageSpec {
            name: spec.name.clone(),
            version,
        }
    } else {
        anyos_std::println!("npm: registry metadata unavailable, recording requested spec");
        spec.clone()
    };
    if save_dev {
        manifest.add_dev_dependency(&install_spec);
    } else {
        manifest.add_dependency(&install_spec);
    }
    if anyos_std::fs::write_bytes("package.json", manifest.as_str().as_bytes()).is_err() {
        anyos_std::println!("npm: could not update package.json");
        return;
    }

    let installer = PackageInstaller::new(registry);
    match installer.install_package_result(".", &install_spec) {
        Ok(report) => {
            anyos_std::println!("added {}@{}", install_spec.name, install_spec.version);
            anyos_std::println!("installed packages: {}", report.installed.len());
            anyos_std::println!("registry: {}", client.package_metadata_url(&spec.name));
        }
        Err(err) => anyos_std::println!("npm: {}", err),
    }
}

fn npm_install_manifest(registry: RegistryConfig, include_dev: bool) {
    let installer = PackageInstaller::new(registry);
    match installer.install_manifest_dependencies_with_options_result(
        ".",
        InstallManifestOptions { include_dev },
    ) {
        Ok(report) => {
            anyos_std::println!("installed packages: {}", report.installed.len());
        }
        Err(err) => anyos_std::println!("npm: {}", err),
    }
}

fn usage() {
    anyos_std::println!("npm {}", libnode::VERSION);
    anyos_std::println!("Usage:");
    anyos_std::println!("  npm init [-y]");
    anyos_std::println!("  npm install [package[@version] ...] [--registry url]");
    anyos_std::println!("  npm install -g <package[@version] ...> [--prefix /System]");
    anyos_std::println!("  npm run <script> [args...]");
    anyos_std::println!("  npm start");
    anyos_std::println!("  npm uninstall <package...>");
    anyos_std::println!("  npm update [package...]");
    anyos_std::println!("  npm list");
    anyos_std::println!("  npm outdated");
    anyos_std::println!("  npm info <package>");
    anyos_std::println!("  npm search <query>");
}

fn npm_run_script(script: &str, extra_args: &[String]) -> u32 {
    let data = anyos_std::fs::read_to_string("package.json").ok();
    let manifest = PackageManifest::parse_or_new(data);
    let Some(command_line) = manifest.script(script) else {
        anyos_std::println!("npm: missing script: {}", script);
        return 1;
    };
    anyos_std::println!("> {}", script);
    anyos_std::println!("> {}", command_line);

    let mut tokens = anyos_std::args::tokenize(&command_line);
    if tokens.is_empty() {
        anyos_std::println!("npm: script '{}' is empty", script);
        return 1;
    }
    for arg in extra_args {
        tokens.push(arg.clone());
    }

    if is_node_command(&tokens[0]) {
        return npm_run_node_script(&tokens[1..]);
    }

    let command = resolve_script_command(&tokens[0]);
    let argv = script_argv_string(&command, &tokens[1..]);
    let pipe_id = anyos_std::ipc::pipe_create("npm:run");
    if pipe_id == 0 {
        anyos_std::println!("npm: could not create output pipe");
        return 1;
    }
    let tid = anyos_std::process::spawn_piped(&command, &argv, pipe_id);
    if tid == u32::MAX {
        anyos_std::ipc::pipe_close(pipe_id);
        anyos_std::println!("npm: could not run script command: {}", command);
        return 1;
    }

    let mut buf = [0u8; 1024];
    loop {
        let n = anyos_std::ipc::pipe_read(pipe_id, &mut buf);
        if n != 0 && n != u32::MAX {
            if let Ok(text) = core::str::from_utf8(&buf[..n as usize]) {
                anyos_std::print!("{}", text);
            }
        }
        let status = anyos_std::process::try_waitpid(tid);
        if status != anyos_std::process::STILL_RUNNING && status != u32::MAX {
            anyos_std::ipc::pipe_close(pipe_id);
            return status;
        }
        anyos_std::process::yield_cpu();
    }
}

fn npm_run_node_script(args: &[String]) -> u32 {
    if args.is_empty() {
        anyos_std::println!("npm: node script is missing");
        return 1;
    }
    if args[0].starts_with('-') {
        return npm_run_external_script_command("node", args);
    }

    let script = &args[0];
    let mut options = libnode::NodeOptions::default();
    options.cwd = current_dir();
    options.argv = {
        let mut argv = alloc::vec::Vec::new();
        argv.push(String::from("node"));
        argv.push(script.clone());
        argv.extend(args.iter().skip(1).cloned());
        argv
    };

    let mut runtime = libnode::NodeRuntime::new(options);
    match runtime.run_file(script) {
        Ok(_) => {
            runtime.run_event_loop();
            flush_node_console(&mut runtime);
            if let Some(exception) = runtime.engine().last_exception() {
                anyos_std::println!("{}", format_node_exception(exception));
                1
            } else {
                0
            }
        }
        Err(err) => {
            anyos_std::println!("node: {}", err);
            1
        }
    }
}

fn npm_run_external_script_command(command: &str, args: &[String]) -> u32 {
    let command = resolve_script_command(command);
    let argv = script_argv_string(&command, args);
    let pipe_id = anyos_std::ipc::pipe_create("npm:run");
    if pipe_id == 0 {
        anyos_std::println!("npm: could not create output pipe");
        return 1;
    }
    let tid = anyos_std::process::spawn_piped(&command, &argv, pipe_id);
    if tid == u32::MAX {
        anyos_std::ipc::pipe_close(pipe_id);
        anyos_std::println!("npm: could not run script command: {}", command);
        return 1;
    }
    pump_process_output(pipe_id, tid)
}

fn is_node_command(command: &str) -> bool {
    matches!(basename(command).as_str(), "node" | "node.elf")
}

fn flush_node_console(runtime: &mut libnode::NodeRuntime) {
    for msg in runtime.engine().console_output() {
        anyos_std::println!("{}", msg);
    }
    runtime.engine().clear_console();
}

fn format_node_exception(exception: &libjs::JsValue) -> String {
    let stack = exception.get_property("stack").to_js_string();
    if !stack.is_empty() && stack != "undefined" {
        return stack;
    }
    let name = exception.get_property("name").to_js_string();
    let message = exception.get_property("message").to_js_string();
    match (name.as_str(), message.as_str()) {
        ("undefined", "undefined") | ("", "") => exception.to_js_string(),
        ("undefined", message) | ("", message) => String::from(message),
        (name, "undefined") | (name, "") => String::from(name),
        (name, message) => alloc::format!("{}: {}", name, message),
    }
}

fn current_dir() -> String {
    let mut buf = [0u8; 512];
    let len = anyos_std::fs::getcwd(&mut buf);
    if len == u32::MAX {
        return String::from(".");
    }
    let len = (len as usize).min(buf.len());
    String::from(core::str::from_utf8(&buf[..len]).unwrap_or("."))
}

fn pump_process_output(pipe_id: u32, tid: u32) -> u32 {
    let mut buf = [0u8; 1024];
    loop {
        let n = anyos_std::ipc::pipe_read(pipe_id, &mut buf);
        if n != 0 && n != u32::MAX {
            if let Ok(text) = core::str::from_utf8(&buf[..n as usize]) {
                anyos_std::print!("{}", text);
            }
        }
        let status = anyos_std::process::try_waitpid(tid);
        if status != anyos_std::process::STILL_RUNNING && status != u32::MAX {
            anyos_std::ipc::pipe_close(pipe_id);
            return status;
        }
        anyos_std::process::yield_cpu();
    }
}

fn resolve_script_command(command: &str) -> String {
    if command.contains('/') {
        return String::from(command);
    }
    let system = alloc::format!("/System/bin/{}", command);
    let mut stat = [0u32; 7];
    if anyos_std::fs::stat(&system, &mut stat) == 0 {
        system
    } else {
        String::from(command)
    }
}

fn script_argv_string(command: &str, args: &[String]) -> String {
    let mut out = basename(command);
    for arg in args {
        out.push(' ');
        out.push_str(arg);
    }
    out
}

fn basename(path: &str) -> String {
    match path.rsplit('/').next() {
        Some(name) if !name.is_empty() => String::from(name),
        _ => String::from(path),
    }
}
