use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::errors::AsldError;
use crate::model::{DistroConfig, StorageSpec, StorageValidation};

pub fn validate_storage(name: &str, storage: &StorageSpec) -> Vec<StorageValidation> {
    let mut out = Vec::new();
    out.push(validate_layout(storage));
    out.push(validate_layer_path(
        name,
        "base",
        &storage.base_image_path,
        true,
    ));
    out.push(validate_layer_path(
        name,
        "overlay",
        &storage.overlay_image_path,
        true,
    ));
    if storage.state_image_enabled {
        out.push(validate_layer_path(
            name,
            "state",
            &storage.state_image_path,
            true,
        ));
    } else {
        out.push(StorageValidation {
            role: String::from("state"),
            path: storage.state_image_path.clone(),
            valid: true,
            message: String::from("state layer disabled"),
        });
    }
    annotate_distinct_paths(&mut out, storage);
    out
}

pub fn validate_storage_policy(name: &str, storage: &StorageSpec) -> Result<(), AsldError> {
    let report = validate_storage(name, storage);
    if report.iter().all(|item| item.valid) {
        Ok(())
    } else {
        Err(AsldError::InvalidArgument("storage"))
    }
}

pub fn export_manifest_lines(cfg: &DistroConfig) -> Vec<String> {
    let mut lines = alloc::vec![
        String::from("format\tasl-export-v1"),
        format!("name\t{}", cfg.name),
        format!("id\t{}", cfg.id),
        format!("owner\t{}", cfg.owner),
        format!("base_image_ref\t{}", cfg.base_image_ref),
        format!("kernel_profile\t{}", cfg.kernel_profile),
        format!("resources.memory_mb\t{}", cfg.resources.memory_mb),
        format!("resources.vcpu_count\t{}", cfg.resources.vcpu_count),
        format!("resources.autostart\t{}", cfg.resources.autostart),
        format!("storage.layout\t{}", cfg.storage.layout),
        format!("storage.base_image_path\t{}", cfg.storage.base_image_path),
        format!(
            "storage.overlay_image_path\t{}",
            cfg.storage.overlay_image_path
        ),
        format!("storage.state_image_path\t{}", cfg.storage.state_image_path),
        format!(
            "storage.state_image_enabled\t{}",
            cfg.storage.state_image_enabled
        ),
        format!("network.mode\t{}", cfg.network.mode),
        format!("network.dns_mode\t{}", cfg.network.dns_mode),
        format!("network.allow_outbound\t{}", cfg.network.allow_outbound),
        format!("agent.enabled\t{}", cfg.agent.enabled),
        format!(
            "agent.required_for_rich_integration\t{}",
            cfg.agent.required_for_rich_integration
        ),
        format!(
            "agent.fallback_console_enabled\t{}",
            cfg.agent.fallback_console_enabled
        ),
        format!("mounts.count\t{}", cfg.mounts.len()),
        format!("port_forwards.count\t{}", cfg.port_forwards.len()),
    ];
    for mount in &cfg.mounts {
        lines.push(format!(
            "mount\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
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
    for rule in &cfg.port_forwards {
        lines.push(format!(
            "port\t{}\t{}\t{}\t{}\t{}\t{}",
            rule.id,
            rule.listen_address,
            rule.listen_port,
            rule.guest_port,
            rule.protocol,
            rule.description
        ));
    }
    lines
}

fn validate_layout(storage: &StorageSpec) -> StorageValidation {
    let valid = storage.layout == "layered-v1";
    StorageValidation {
        role: String::from("layout"),
        path: storage.layout.clone(),
        valid,
        message: if valid {
            String::from("layered rootfs layout")
        } else {
            String::from("unsupported storage layout")
        },
    }
}

fn validate_layer_path(name: &str, role: &str, path: &str, required: bool) -> StorageValidation {
    let expected_prefix = format!("/System/var/asl/distros/{name}/images/");
    let valid = (!required || !path.is_empty())
        && path.starts_with('/')
        && !path.contains("/../")
        && !path.ends_with("/..")
        && path.starts_with(&expected_prefix);
    StorageValidation {
        role: String::from(role),
        path: String::from(path),
        valid,
        message: if valid {
            String::from("layer path scoped to distro images")
        } else {
            format!("layer path must be under {}", expected_prefix)
        },
    }
}

fn annotate_distinct_paths(report: &mut [StorageValidation], storage: &StorageSpec) {
    let base = &storage.base_image_path;
    let overlay = &storage.overlay_image_path;
    let state = &storage.state_image_path;
    if base == overlay || (storage.state_image_enabled && (base == state || overlay == state)) {
        for item in report.iter_mut() {
            if item.role == "base" || item.role == "overlay" || item.role == "state" {
                item.valid = false;
                item.message = String::from("storage layer paths must be distinct");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::distro::build_distro_config;
    use crate::model::default_storage_for;

    use super::{export_manifest_lines, validate_storage, validate_storage_policy};

    #[test]
    fn validates_default_layered_storage() {
        let storage = default_storage_for("ubuntu-dev");
        let report = validate_storage("ubuntu-dev", &storage);
        assert!(report.iter().all(|item| item.valid));
    }

    #[test]
    fn rejects_cross_distro_storage_path() {
        let mut storage = default_storage_for("ubuntu-dev");
        storage.overlay_image_path =
            alloc::string::String::from("/System/var/asl/distros/other/images/overlay.img");
        assert!(validate_storage_policy("ubuntu-dev", &storage).is_err());
    }

    #[test]
    fn export_manifest_is_logical_and_complete() {
        let cfg = build_distro_config("ubuntu-dev", "ubuntu-24.04-x86_64-v1", "strati").unwrap();
        let lines = export_manifest_lines(&cfg);
        assert!(lines.iter().any(|line| line == "format\tasl-export-v1"));
        assert!(lines
            .iter()
            .any(|line| line == "base_image_ref\tubuntu-24.04-x86_64-v1"));
    }
}
