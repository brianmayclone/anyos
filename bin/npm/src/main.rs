#![cfg_attr(not(feature = "host"), no_std)]
#![cfg_attr(not(feature = "host"), no_main)]

use alloc::string::String;
use libnode::npm::{
    PackageInstaller, PackageManifest, PackageSpec, RegistryClient, RegistryConfig,
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
    let registry = RegistryConfig {
        url: cli.registry,
    };
    if cli.global {
        anyos_std::println!("npm: global mode requested; installing into the current prefix is not implemented yet");
    }

    match cli.command {
        cli::NpmCommand::Help => usage(),
        cli::NpmCommand::Version => anyos_std::println!("{}", libnode::VERSION),
        cli::NpmCommand::Init { yes } => npm_init(yes),
        cli::NpmCommand::Install { packages } => {
            if packages.is_empty() {
                npm_install_manifest(registry);
            } else {
                for package in packages {
                    npm_install(&package, registry.clone());
                }
            }
        }
        cli::NpmCommand::Uninstall { packages } => npm_uninstall(&packages),
        cli::NpmCommand::Update { packages } => npm_update(&packages, registry),
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
    if changed
        && anyos_std::fs::write_bytes("package.json", manifest.as_str().as_bytes()).is_err()
    {
        anyos_std::println!("npm: could not update package.json");
    }
}

fn npm_update(packages: &[String], registry: RegistryConfig) {
    if packages.is_empty() {
        npm_install_manifest(registry);
    } else {
        for package in packages {
            npm_install(package, registry.clone());
        }
    }
}

fn npm_list() {
    let data = anyos_std::fs::read_to_string("package.json").ok();
    let manifest = PackageManifest::parse_or_new(data);
    for dep in manifest.dependencies() {
        anyos_std::println!("{}@{}", dep.name, dep.version);
    }
}

fn npm_outdated(registry: RegistryConfig) {
    let data = anyos_std::fs::read_to_string("package.json").ok();
    let manifest = PackageManifest::parse_or_new(data);
    let client = RegistryClient::new(registry);
    for dep in manifest.dependencies() {
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

fn npm_install(package: &str, registry: RegistryConfig) {
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
    manifest.add_dependency(&install_spec);
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

fn npm_install_manifest(registry: RegistryConfig) {
    let installer = PackageInstaller::new(registry);
    match installer.install_manifest_dependencies_result(".") {
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
    anyos_std::println!("  npm uninstall <package...>");
    anyos_std::println!("  npm update [package...]");
    anyos_std::println!("  npm list");
    anyos_std::println!("  npm outdated");
    anyos_std::println!("  npm info <package>");
    anyos_std::println!("  npm search <query>");
}
