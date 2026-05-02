#![cfg_attr(not(feature = "host"), no_std)]
#![cfg_attr(not(feature = "host"), no_main)]

use alloc::string::String;
use libnode::npm::{
    PackageInstaller, PackageManifest, PackageSpec, RegistryClient, RegistryConfig,
};

#[cfg(feature = "host")]
extern crate alloc;

anyos_std::entry!(npm_main);

fn npm_main() {
    let mut args_buf = [0u8; 1024];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"r");
    let registry = RegistryConfig {
        url: String::from(args.opt(b'r').unwrap_or(libnode::DEFAULT_NPM_REGISTRY)),
    };

    match args.pos(0).unwrap_or("") {
        "init" => npm_init(),
        "install" | "i" => {
            if let Some(package) = args.pos(1) {
                npm_install(package, registry);
            } else {
                npm_install_manifest(registry);
            }
        }
        "info" | "view" => {
            let Some(package) = args.pos(1) else {
                anyos_std::println!("npm: info requires a package name");
                return;
            };
            let client = RegistryClient::new(registry);
            match client.fetch_metadata(package) {
                Some(metadata) => {
                    let latest = metadata
                        .resolve_version("latest")
                        .unwrap_or_else(|| String::from("unknown"));
                    anyos_std::println!("{} latest {}", package, latest);
                    anyos_std::println!("{}", client.package_metadata_url(package));
                }
                None => anyos_std::println!("npm: could not fetch {}", package),
            }
        }
        "search" => {
            let query = args.pos(1).unwrap_or("");
            anyos_std::println!("Searching {} for '{}'", registry.normalized_url(), query);
            anyos_std::println!("network registry search is planned for npm transport v1");
        }
        "--version" | "-v" => anyos_std::println!("{}", libnode::VERSION),
        _ => usage(),
    }
}

fn npm_init() {
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
    anyos_std::println!("  npm init");
    anyos_std::println!("  npm install [package[@version]] [-r registry]");
    anyos_std::println!("  npm info <package>");
    anyos_std::println!("  npm search <query>");
}
