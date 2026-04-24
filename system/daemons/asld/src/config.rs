use alloc::string::{String, ToString};
use alloc::vec::Vec;

use libconf::{ConfClient, ConfError, ConfItem, ConfValue, RegistryScope};

use crate::errors::AsldError;
use crate::model::{
    default_storage_for, is_valid_distro_name, AgentPolicy, DistroConfig, DistroMetadata,
    LifecyclePolicy, MountSpec, NetworkPolicy, PortForwardSpec, Resources, StorageSpec,
};
use crate::{mounts, network, storage};

const DISTROS_ROOT: &str = "platform/asl/distros";

pub trait ConfigStore {
    fn mkdir(&mut self, path: &str) -> Result<(), AsldError>;
    fn del(&mut self, path: &str) -> Result<(), AsldError>;
    fn set_string(&mut self, path: &str, value: &str) -> Result<(), AsldError>;
    fn set_int(&mut self, path: &str, value: i64) -> Result<(), AsldError>;
    fn set_bool(&mut self, path: &str, value: bool) -> Result<(), AsldError>;
    fn get_string(&mut self, path: &str) -> Result<Option<String>, AsldError>;
    fn get_int(&mut self, path: &str) -> Result<Option<i64>, AsldError>;
    fn get_bool(&mut self, path: &str) -> Result<Option<bool>, AsldError>;
    fn list_children(&mut self, path: &str) -> Result<Vec<String>, AsldError>;
}

pub struct ConfdStore {
    client: ConfClient,
}

impl ConfdStore {
    pub fn connect(client_name: &str) -> Result<Self, AsldError> {
        Ok(Self {
            client: ConfClient::connect(client_name)?,
        })
    }
}

impl ConfigStore for ConfdStore {
    fn mkdir(&mut self, path: &str) -> Result<(), AsldError> {
        self.client.mkdir(RegistryScope::System, path)?;
        Ok(())
    }

    fn del(&mut self, path: &str) -> Result<(), AsldError> {
        self.client.del(RegistryScope::System, path)?;
        Ok(())
    }

    fn set_string(&mut self, path: &str, value: &str) -> Result<(), AsldError> {
        self.client.set(
            RegistryScope::System,
            path,
            ConfValue::String(String::from(value)),
        )?;
        Ok(())
    }

    fn set_int(&mut self, path: &str, value: i64) -> Result<(), AsldError> {
        self.client
            .set(RegistryScope::System, path, ConfValue::Int(value))?;
        Ok(())
    }

    fn set_bool(&mut self, path: &str, value: bool) -> Result<(), AsldError> {
        self.client
            .set(RegistryScope::System, path, ConfValue::Bool(value))?;
        Ok(())
    }

    fn get_string(&mut self, path: &str) -> Result<Option<String>, AsldError> {
        get_string_item(self.client.get(RegistryScope::System, path))
    }

    fn get_int(&mut self, path: &str) -> Result<Option<i64>, AsldError> {
        get_int_item(self.client.get(RegistryScope::System, path))
    }

    fn get_bool(&mut self, path: &str) -> Result<Option<bool>, AsldError> {
        get_bool_item(self.client.get(RegistryScope::System, path))
    }

    fn list_children(&mut self, path: &str) -> Result<Vec<String>, AsldError> {
        let items = self.client.list_children(RegistryScope::System, path)?;
        Ok(items.iter().filter_map(last_segment_of).collect())
    }
}

pub fn distro_root(name: &str) -> String {
    alloc::format!("{DISTROS_ROOT}/{name}")
}

pub fn list_distros<S: ConfigStore>(store: &mut S) -> Result<Vec<String>, AsldError> {
    store.list_children(DISTROS_ROOT)
}

pub fn ensure_distro_tree<S: ConfigStore>(
    store: &mut S,
    config: &DistroConfig,
) -> Result<(), AsldError> {
    let root = distro_root(&config.name);
    network::validate_network_policy(&config.network)?;
    storage::validate_storage_policy(&config.name, &config.storage)?;
    for dir in [
        root.clone(),
        alloc::format!("{root}/resources"),
        alloc::format!("{root}/storage"),
        alloc::format!("{root}/network"),
        alloc::format!("{root}/agent"),
        alloc::format!("{root}/lifecycle"),
        alloc::format!("{root}/mounts"),
        alloc::format!("{root}/port_forwards"),
        alloc::format!("{root}/metadata"),
    ] {
        store.mkdir(&dir)?;
    }

    write_scalar_fields(store, &root, config)?;

    for mount in &config.mounts {
        mounts::validate_mount(mount)?;
        let mount_root = alloc::format!("{root}/mounts/{}", mount.id);
        store.mkdir(&mount_root)?;
        store.set_string(&alloc::format!("{mount_root}/host_path"), &mount.host_path)?;
        store.set_string(
            &alloc::format!("{mount_root}/guest_path"),
            &mount.guest_path,
        )?;
        store.set_string(&alloc::format!("{mount_root}/mode"), &mount.mode)?;
        store.set_string(
            &alloc::format!("{mount_root}/metadata_mode"),
            &mount.metadata_mode,
        )?;
        store.set_string(&alloc::format!("{mount_root}/case_mode"), &mount.case_mode)?;
        store.set_string(
            &alloc::format!("{mount_root}/exec_policy"),
            &mount.exec_policy,
        )?;
        store.set_string(
            &alloc::format!("{mount_root}/watch_policy"),
            &mount.watch_policy,
        )?;
        store.set_string(
            &alloc::format!("{mount_root}/description"),
            &mount.description,
        )?;
    }

    for rule in &config.port_forwards {
        network::validate_port_forward(rule)?;
        let rule_root = alloc::format!("{root}/port_forwards/{}", rule.id);
        store.mkdir(&rule_root)?;
        store.set_string(
            &alloc::format!("{rule_root}/listen_address"),
            &rule.listen_address,
        )?;
        store.set_int(
            &alloc::format!("{rule_root}/listen_port"),
            rule.listen_port as i64,
        )?;
        store.set_int(
            &alloc::format!("{rule_root}/guest_port"),
            rule.guest_port as i64,
        )?;
        store.set_string(&alloc::format!("{rule_root}/protocol"), &rule.protocol)?;
        store.set_string(
            &alloc::format!("{rule_root}/description"),
            &rule.description,
        )?;
    }

    Ok(())
}

pub fn load_distro<S: ConfigStore>(store: &mut S, name: &str) -> Result<DistroConfig, AsldError> {
    let root = distro_root(name);
    let id = required_string(store, &alloc::format!("{root}/id"), "id")?;
    let owner = required_string(store, &alloc::format!("{root}/owner"), "owner")?;
    let base_image_ref = required_string(
        store,
        &alloc::format!("{root}/base_image_ref"),
        "base_image_ref",
    )?;
    let kernel_profile = required_string(
        store,
        &alloc::format!("{root}/kernel_profile"),
        "kernel_profile",
    )?;

    let resources = Resources {
        memory_mb: store
            .get_int(&alloc::format!("{root}/resources/memory_mb"))?
            .unwrap_or(2048)
            .max(256) as u32,
        vcpu_count: store
            .get_int(&alloc::format!("{root}/resources/vcpu_count"))?
            .unwrap_or(2)
            .max(1) as u16,
        autostart: store
            .get_bool(&alloc::format!("{root}/resources/autostart"))?
            .unwrap_or(false),
    };
    let storage = StorageSpec {
        layout: required_string(
            store,
            &alloc::format!("{root}/storage/layout"),
            "storage.layout",
        )?,
        base_image_path: required_string(
            store,
            &alloc::format!("{root}/storage/base_image_path"),
            "storage.base_image_path",
        )?,
        overlay_image_path: required_string(
            store,
            &alloc::format!("{root}/storage/overlay_image_path"),
            "storage.overlay_image_path",
        )?,
        state_image_path: store
            .get_string(&alloc::format!("{root}/storage/state_image_path"))?
            .unwrap_or_default(),
        state_image_enabled: store
            .get_bool(&alloc::format!("{root}/storage/state_image_enabled"))?
            .unwrap_or(false),
    };
    storage::validate_storage_policy(name, &storage)?;
    let network = NetworkPolicy {
        mode: store
            .get_string(&alloc::format!("{root}/network/mode"))?
            .unwrap_or_else(|| String::from("nat")),
        dns_mode: store
            .get_string(&alloc::format!("{root}/network/dns_mode"))?
            .unwrap_or_else(|| String::from("host-broker")),
        allow_outbound: store
            .get_bool(&alloc::format!("{root}/network/allow_outbound"))?
            .unwrap_or(true),
    };
    network::validate_network_policy(&network)?;
    let agent = AgentPolicy {
        enabled: store
            .get_bool(&alloc::format!("{root}/agent/enabled"))?
            .unwrap_or(true),
        required_for_rich_integration: store
            .get_bool(&alloc::format!(
                "{root}/agent/required_for_rich_integration"
            ))?
            .unwrap_or(true),
        fallback_console_enabled: store
            .get_bool(&alloc::format!("{root}/agent/fallback_console_enabled"))?
            .unwrap_or(true),
    };
    let lifecycle = LifecyclePolicy {
        restart_on_failure: store
            .get_bool(&alloc::format!("{root}/lifecycle/restart_on_failure"))?
            .unwrap_or(true),
        shutdown_timeout_ms: store
            .get_int(&alloc::format!("{root}/lifecycle/shutdown_timeout_ms"))?
            .unwrap_or(10_000) as u32,
        boot_timeout_ms: store
            .get_int(&alloc::format!("{root}/lifecycle/boot_timeout_ms"))?
            .unwrap_or(30_000) as u32,
    };
    let metadata = DistroMetadata {
        distro_family: store
            .get_string(&alloc::format!("{root}/metadata/distro_family"))?
            .unwrap_or_default(),
        distro_version: store
            .get_string(&alloc::format!("{root}/metadata/distro_version"))?
            .unwrap_or_default(),
        notes: store
            .get_string(&alloc::format!("{root}/metadata/notes"))?
            .unwrap_or_default(),
    };
    let mounts = load_mounts(store, &root)?;
    let port_forwards = load_port_forwards(store, &root)?;

    Ok(DistroConfig {
        schema_version: 1,
        id,
        name: String::from(name),
        owner,
        base_image_ref,
        kernel_profile,
        resources,
        storage,
        network,
        mounts,
        port_forwards,
        agent,
        lifecycle,
        metadata,
    })
}

pub fn add_mount<S: ConfigStore>(
    store: &mut S,
    distro_name: &str,
    mount: &MountSpec,
) -> Result<(), AsldError> {
    mounts::validate_mount(mount)?;
    let root = distro_root(distro_name);
    required_string(store, &alloc::format!("{root}/id"), "id")?;
    let mount_root = alloc::format!("{root}/mounts/{}", mount.id);
    store.mkdir(&alloc::format!("{root}/mounts"))?;
    store.mkdir(&mount_root)?;
    store.set_string(&alloc::format!("{mount_root}/host_path"), &mount.host_path)?;
    store.set_string(
        &alloc::format!("{mount_root}/guest_path"),
        &mount.guest_path,
    )?;
    store.set_string(&alloc::format!("{mount_root}/mode"), &mount.mode)?;
    store.set_string(
        &alloc::format!("{mount_root}/metadata_mode"),
        &mount.metadata_mode,
    )?;
    store.set_string(&alloc::format!("{mount_root}/case_mode"), &mount.case_mode)?;
    store.set_string(
        &alloc::format!("{mount_root}/exec_policy"),
        &mount.exec_policy,
    )?;
    store.set_string(
        &alloc::format!("{mount_root}/watch_policy"),
        &mount.watch_policy,
    )?;
    store.set_string(
        &alloc::format!("{mount_root}/description"),
        &mount.description,
    )?;
    Ok(())
}

pub fn remove_mount<S: ConfigStore>(
    store: &mut S,
    distro_name: &str,
    mount_id: &str,
) -> Result<(), AsldError> {
    let root = distro_root(distro_name);
    required_string(store, &alloc::format!("{root}/id"), "id")?;
    let mount_root = alloc::format!("{root}/mounts/{mount_id}");
    delete_mount_subtree(store, &mount_root)
}

pub fn add_port_forward<S: ConfigStore>(
    store: &mut S,
    distro_name: &str,
    rule: &PortForwardSpec,
) -> Result<(), AsldError> {
    network::validate_port_forward(rule)?;
    let root = distro_root(distro_name);
    required_string(store, &alloc::format!("{root}/id"), "id")?;
    let rule_root = alloc::format!("{root}/port_forwards/{}", rule.id);
    store.mkdir(&alloc::format!("{root}/port_forwards"))?;
    store.mkdir(&rule_root)?;
    store.set_string(
        &alloc::format!("{rule_root}/listen_address"),
        &rule.listen_address,
    )?;
    store.set_int(
        &alloc::format!("{rule_root}/listen_port"),
        rule.listen_port as i64,
    )?;
    store.set_int(
        &alloc::format!("{rule_root}/guest_port"),
        rule.guest_port as i64,
    )?;
    store.set_string(&alloc::format!("{rule_root}/protocol"), &rule.protocol)?;
    store.set_string(
        &alloc::format!("{rule_root}/description"),
        &rule.description,
    )?;
    Ok(())
}

pub fn remove_port_forward<S: ConfigStore>(
    store: &mut S,
    distro_name: &str,
    rule_id: &str,
) -> Result<(), AsldError> {
    let root = distro_root(distro_name);
    required_string(store, &alloc::format!("{root}/id"), "id")?;
    let rule_root = alloc::format!("{root}/port_forwards/{rule_id}");
    delete_port_forward_subtree(store, &rule_root)
}

pub fn delete_distro<S: ConfigStore>(
    store: &mut S,
    distro_name: &str,
) -> Result<DistroConfig, AsldError> {
    let config = load_distro(store, distro_name)?;
    let root = distro_root(distro_name);

    for mount in &config.mounts {
        delete_mount_subtree(store, &alloc::format!("{root}/mounts/{}", mount.id))?;
    }
    for rule in &config.port_forwards {
        delete_port_forward_subtree(store, &alloc::format!("{root}/port_forwards/{}", rule.id))?;
    }

    for key in [
        "schema_version",
        "id",
        "name",
        "owner",
        "base_image_ref",
        "kernel_profile",
    ] {
        delete_if_present(store, &alloc::format!("{root}/{key}"))?;
    }

    delete_known_subtree(
        store,
        &alloc::format!("{root}/resources"),
        &["memory_mb", "vcpu_count", "autostart"],
    )?;
    delete_known_subtree(
        store,
        &alloc::format!("{root}/storage"),
        &[
            "layout",
            "base_image_path",
            "overlay_image_path",
            "state_image_path",
            "state_image_enabled",
        ],
    )?;
    delete_known_subtree(
        store,
        &alloc::format!("{root}/network"),
        &["mode", "dns_mode", "allow_outbound"],
    )?;
    delete_known_subtree(
        store,
        &alloc::format!("{root}/agent"),
        &[
            "enabled",
            "required_for_rich_integration",
            "fallback_console_enabled",
        ],
    )?;
    delete_known_subtree(
        store,
        &alloc::format!("{root}/lifecycle"),
        &[
            "restart_on_failure",
            "shutdown_timeout_ms",
            "boot_timeout_ms",
        ],
    )?;
    delete_known_subtree(
        store,
        &alloc::format!("{root}/metadata"),
        &["distro_family", "distro_version", "notes"],
    )?;

    delete_if_present(store, &alloc::format!("{root}/mounts"))?;
    delete_if_present(store, &alloc::format!("{root}/port_forwards"))?;
    delete_if_present(store, &root)?;
    Ok(config)
}

pub fn update_resources<S: ConfigStore>(
    store: &mut S,
    distro_name: &str,
    memory_mb: Option<u32>,
    vcpu_count: Option<u16>,
) -> Result<DistroConfig, AsldError> {
    let root = distro_root(distro_name);
    required_string(store, &alloc::format!("{root}/id"), "id")?;

    if let Some(memory_mb) = memory_mb {
        if memory_mb < 256 {
            return Err(AsldError::InvalidArgument("memory_mb"));
        }
        store.set_int(
            &alloc::format!("{root}/resources/memory_mb"),
            memory_mb as i64,
        )?;
    }
    if let Some(vcpu_count) = vcpu_count {
        if vcpu_count == 0 || vcpu_count > 64 {
            return Err(AsldError::InvalidArgument("vcpu_count"));
        }
        store.set_int(
            &alloc::format!("{root}/resources/vcpu_count"),
            vcpu_count as i64,
        )?;
    }

    load_distro(store, distro_name)
}

pub fn update_network<S: ConfigStore>(
    store: &mut S,
    distro_name: &str,
    mode: Option<&str>,
    dns_mode: Option<&str>,
    allow_outbound: Option<bool>,
) -> Result<DistroConfig, AsldError> {
    let mut cfg = load_distro(store, distro_name)?;
    if let Some(mode) = mode {
        cfg.network.mode = String::from(mode);
    }
    if let Some(dns_mode) = dns_mode {
        cfg.network.dns_mode = String::from(dns_mode);
    }
    if let Some(allow_outbound) = allow_outbound {
        cfg.network.allow_outbound = allow_outbound;
    }
    network::validate_network_policy(&cfg.network)?;

    let root = distro_root(distro_name);
    store.set_string(&alloc::format!("{root}/network/mode"), &cfg.network.mode)?;
    store.set_string(
        &alloc::format!("{root}/network/dns_mode"),
        &cfg.network.dns_mode,
    )?;
    store.set_bool(
        &alloc::format!("{root}/network/allow_outbound"),
        cfg.network.allow_outbound,
    )?;
    load_distro(store, distro_name)
}

pub fn clone_distro<S: ConfigStore>(
    store: &mut S,
    source_name: &str,
    target_name: &str,
    owner: Option<&str>,
    include_mounts: bool,
    include_ports: bool,
) -> Result<DistroConfig, AsldError> {
    if !is_valid_distro_name(target_name) {
        return Err(AsldError::InvalidName);
    }
    if owner.is_some_and(|owner| owner.is_empty()) {
        return Err(AsldError::MissingField("owner"));
    }
    if load_distro(store, target_name).is_ok() {
        return Err(AsldError::InvalidState("target distro already exists"));
    }

    let source = load_distro(store, source_name)?;
    let mut cloned = source.clone();
    cloned.id = alloc::format!("distro-{target_name}");
    cloned.name = String::from(target_name);
    cloned.owner = owner
        .map(String::from)
        .unwrap_or_else(|| source.owner.clone());
    cloned.storage = default_storage_for(target_name);
    cloned.metadata.notes = if cloned.metadata.notes.is_empty() {
        alloc::format!("cloned from {source_name}")
    } else {
        alloc::format!("{}; cloned from {}", cloned.metadata.notes, source_name)
    };
    if !include_mounts {
        cloned.mounts.clear();
    }
    if !include_ports {
        cloned.port_forwards.clear();
    }

    ensure_distro_tree(store, &cloned)?;
    load_distro(store, target_name)
}

fn write_scalar_fields<S: ConfigStore>(
    store: &mut S,
    root: &str,
    config: &DistroConfig,
) -> Result<(), AsldError> {
    store.set_int(
        &alloc::format!("{root}/schema_version"),
        config.schema_version as i64,
    )?;
    store.set_string(&alloc::format!("{root}/id"), &config.id)?;
    store.set_string(&alloc::format!("{root}/name"), &config.name)?;
    store.set_string(&alloc::format!("{root}/owner"), &config.owner)?;
    store.set_string(
        &alloc::format!("{root}/base_image_ref"),
        &config.base_image_ref,
    )?;
    store.set_string(
        &alloc::format!("{root}/kernel_profile"),
        &config.kernel_profile,
    )?;

    store.set_int(
        &alloc::format!("{root}/resources/memory_mb"),
        config.resources.memory_mb as i64,
    )?;
    store.set_int(
        &alloc::format!("{root}/resources/vcpu_count"),
        config.resources.vcpu_count as i64,
    )?;
    store.set_bool(
        &alloc::format!("{root}/resources/autostart"),
        config.resources.autostart,
    )?;

    store.set_string(
        &alloc::format!("{root}/storage/layout"),
        &config.storage.layout,
    )?;
    store.set_string(
        &alloc::format!("{root}/storage/base_image_path"),
        &config.storage.base_image_path,
    )?;
    store.set_string(
        &alloc::format!("{root}/storage/overlay_image_path"),
        &config.storage.overlay_image_path,
    )?;
    store.set_string(
        &alloc::format!("{root}/storage/state_image_path"),
        &config.storage.state_image_path,
    )?;
    store.set_bool(
        &alloc::format!("{root}/storage/state_image_enabled"),
        config.storage.state_image_enabled,
    )?;

    store.set_string(&alloc::format!("{root}/network/mode"), &config.network.mode)?;
    store.set_string(
        &alloc::format!("{root}/network/dns_mode"),
        &config.network.dns_mode,
    )?;
    store.set_bool(
        &alloc::format!("{root}/network/allow_outbound"),
        config.network.allow_outbound,
    )?;

    store.set_bool(
        &alloc::format!("{root}/agent/enabled"),
        config.agent.enabled,
    )?;
    store.set_bool(
        &alloc::format!("{root}/agent/required_for_rich_integration"),
        config.agent.required_for_rich_integration,
    )?;
    store.set_bool(
        &alloc::format!("{root}/agent/fallback_console_enabled"),
        config.agent.fallback_console_enabled,
    )?;

    store.set_bool(
        &alloc::format!("{root}/lifecycle/restart_on_failure"),
        config.lifecycle.restart_on_failure,
    )?;
    store.set_int(
        &alloc::format!("{root}/lifecycle/shutdown_timeout_ms"),
        config.lifecycle.shutdown_timeout_ms as i64,
    )?;
    store.set_int(
        &alloc::format!("{root}/lifecycle/boot_timeout_ms"),
        config.lifecycle.boot_timeout_ms as i64,
    )?;

    store.set_string(
        &alloc::format!("{root}/metadata/distro_family"),
        &config.metadata.distro_family,
    )?;
    store.set_string(
        &alloc::format!("{root}/metadata/distro_version"),
        &config.metadata.distro_version,
    )?;
    store.set_string(
        &alloc::format!("{root}/metadata/notes"),
        &config.metadata.notes,
    )?;
    Ok(())
}

fn required_string<S: ConfigStore>(
    store: &mut S,
    path: &str,
    field: &'static str,
) -> Result<String, AsldError> {
    store
        .get_string(path)?
        .ok_or(AsldError::MissingField(field))
}

fn delete_mount_subtree<S: ConfigStore>(store: &mut S, mount_root: &str) -> Result<(), AsldError> {
    let keys = [
        "host_path",
        "guest_path",
        "mode",
        "metadata_mode",
        "case_mode",
        "exec_policy",
        "watch_policy",
        "description",
    ];
    for key in keys {
        delete_if_present(store, &alloc::format!("{mount_root}/{key}"))?;
    }
    delete_if_present(store, mount_root)
}

fn delete_port_forward_subtree<S: ConfigStore>(
    store: &mut S,
    rule_root: &str,
) -> Result<(), AsldError> {
    let keys = [
        "listen_address",
        "listen_port",
        "guest_port",
        "protocol",
        "description",
    ];
    for key in keys {
        delete_if_present(store, &alloc::format!("{rule_root}/{key}"))?;
    }
    delete_if_present(store, rule_root)
}

fn delete_known_subtree<S: ConfigStore>(
    store: &mut S,
    root: &str,
    keys: &[&str],
) -> Result<(), AsldError> {
    for key in keys {
        delete_if_present(store, &alloc::format!("{root}/{key}"))?;
    }
    delete_if_present(store, root)
}

fn delete_if_present<S: ConfigStore>(store: &mut S, path: &str) -> Result<(), AsldError> {
    match store.del(path) {
        Ok(()) | Err(AsldError::NotFound) => Ok(()),
        Err(err) => Err(err),
    }
}

fn load_mounts<S: ConfigStore>(store: &mut S, root: &str) -> Result<Vec<MountSpec>, AsldError> {
    let mounts_root = alloc::format!("{root}/mounts");
    let mut mounts = Vec::new();
    for id in store.list_children(&mounts_root)? {
        let item_root = alloc::format!("{mounts_root}/{id}");
        let mount = MountSpec {
            id,
            host_path: required_string(
                store,
                &alloc::format!("{item_root}/host_path"),
                "mount.host_path",
            )?,
            guest_path: required_string(
                store,
                &alloc::format!("{item_root}/guest_path"),
                "mount.guest_path",
            )?,
            mode: required_string(store, &alloc::format!("{item_root}/mode"), "mount.mode")?,
            metadata_mode: store
                .get_string(&alloc::format!("{item_root}/metadata_mode"))?
                .unwrap_or_else(|| String::from("relaxed")),
            case_mode: store
                .get_string(&alloc::format!("{item_root}/case_mode"))?
                .unwrap_or_else(|| String::from("host-native")),
            exec_policy: store
                .get_string(&alloc::format!("{item_root}/exec_policy"))?
                .unwrap_or_else(|| String::from("inherit")),
            watch_policy: store
                .get_string(&alloc::format!("{item_root}/watch_policy"))?
                .unwrap_or_else(|| String::from("best-effort")),
            description: store
                .get_string(&alloc::format!("{item_root}/description"))?
                .unwrap_or_default(),
        };
        mounts::validate_mount(&mount)?;
        mounts.push(mount);
    }
    Ok(mounts)
}

fn load_port_forwards<S: ConfigStore>(
    store: &mut S,
    root: &str,
) -> Result<Vec<PortForwardSpec>, AsldError> {
    let forwards_root = alloc::format!("{root}/port_forwards");
    let mut forwards = Vec::new();
    for id in store.list_children(&forwards_root)? {
        let item_root = alloc::format!("{forwards_root}/{id}");
        let listen_port = required_int(
            store,
            &alloc::format!("{item_root}/listen_port"),
            "port_forward.listen_port",
        )?;
        let guest_port = required_int(
            store,
            &alloc::format!("{item_root}/guest_port"),
            "port_forward.guest_port",
        )?;
        if !(1..=u16::MAX as i64).contains(&listen_port)
            || !(1..=u16::MAX as i64).contains(&guest_port)
        {
            return Err(AsldError::InvalidArgument("port value"));
        }
        let rule = PortForwardSpec {
            id,
            listen_address: store
                .get_string(&alloc::format!("{item_root}/listen_address"))?
                .unwrap_or_else(|| String::from("127.0.0.1")),
            listen_port: listen_port as u16,
            guest_port: guest_port as u16,
            protocol: store
                .get_string(&alloc::format!("{item_root}/protocol"))?
                .unwrap_or_else(|| String::from("tcp")),
            description: store
                .get_string(&alloc::format!("{item_root}/description"))?
                .unwrap_or_default(),
        };
        network::validate_port_forward(&rule)?;
        forwards.push(rule);
    }
    Ok(forwards)
}

fn required_int<S: ConfigStore>(
    store: &mut S,
    path: &str,
    field: &'static str,
) -> Result<i64, AsldError> {
    store.get_int(path)?.ok_or(AsldError::MissingField(field))
}

fn get_string_item(result: Result<ConfItem, ConfError>) -> Result<Option<String>, AsldError> {
    match result {
        Ok(item) => match item.value {
            Some(ConfValue::String(v)) => Ok(Some(v)),
            _ => Ok(None),
        },
        Err(ConfError::Remote(msg)) if msg == "not_found" => Ok(None),
        Err(err) => Err(AsldError::from(err)),
    }
}

fn get_int_item(result: Result<ConfItem, ConfError>) -> Result<Option<i64>, AsldError> {
    match result {
        Ok(item) => match item.value {
            Some(ConfValue::Int(v)) => Ok(Some(v)),
            _ => Ok(None),
        },
        Err(ConfError::Remote(msg)) if msg == "not_found" => Ok(None),
        Err(err) => Err(AsldError::from(err)),
    }
}

fn get_bool_item(result: Result<ConfItem, ConfError>) -> Result<Option<bool>, AsldError> {
    match result {
        Ok(item) => match item.value {
            Some(ConfValue::Bool(v)) => Ok(Some(v)),
            _ => Ok(None),
        },
        Err(ConfError::Remote(msg)) if msg == "not_found" => Ok(None),
        Err(err) => Err(AsldError::from(err)),
    }
}

fn last_segment_of(item: &ConfItem) -> Option<String> {
    item.path.rsplit('/').next().map(ToString::to_string)
}

#[cfg(test)]
use alloc::collections::BTreeMap;

#[cfg(test)]
#[derive(Default)]
pub struct FakeStore {
    entries: BTreeMap<String, FakeValue>,
}

#[cfg(test)]
#[derive(Clone, Debug)]
enum FakeValue {
    Dir,
    String(String),
    Int(i64),
    Bool(bool),
}

#[cfg(test)]
impl ConfigStore for FakeStore {
    fn mkdir(&mut self, path: &str) -> Result<(), AsldError> {
        self.entries.insert(String::from(path), FakeValue::Dir);
        Ok(())
    }

    fn del(&mut self, path: &str) -> Result<(), AsldError> {
        self.entries
            .remove(path)
            .map(|_| ())
            .ok_or(AsldError::NotFound)
    }

    fn set_string(&mut self, path: &str, value: &str) -> Result<(), AsldError> {
        self.entries
            .insert(String::from(path), FakeValue::String(String::from(value)));
        Ok(())
    }

    fn set_int(&mut self, path: &str, value: i64) -> Result<(), AsldError> {
        self.entries
            .insert(String::from(path), FakeValue::Int(value));
        Ok(())
    }

    fn set_bool(&mut self, path: &str, value: bool) -> Result<(), AsldError> {
        self.entries
            .insert(String::from(path), FakeValue::Bool(value));
        Ok(())
    }

    fn get_string(&mut self, path: &str) -> Result<Option<String>, AsldError> {
        Ok(match self.entries.get(path) {
            Some(FakeValue::String(v)) => Some(v.clone()),
            _ => None,
        })
    }

    fn get_int(&mut self, path: &str) -> Result<Option<i64>, AsldError> {
        Ok(match self.entries.get(path) {
            Some(FakeValue::Int(v)) => Some(*v),
            _ => None,
        })
    }

    fn get_bool(&mut self, path: &str) -> Result<Option<bool>, AsldError> {
        Ok(match self.entries.get(path) {
            Some(FakeValue::Bool(v)) => Some(*v),
            _ => None,
        })
    }

    fn list_children(&mut self, path: &str) -> Result<Vec<String>, AsldError> {
        let mut out = Vec::new();
        let prefix = alloc::format!("{path}/");
        for key in self.entries.keys() {
            if !key.starts_with(&prefix) {
                continue;
            }
            let rest = &key[prefix.len()..];
            if let Some((child, _)) = rest.split_once('/') {
                if !out.iter().any(|c| c == child) {
                    out.push(String::from(child));
                }
            }
        }
        out.sort();
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::String;

    use crate::config::ConfigStore;
    use crate::distro::build_distro_config;
    use crate::model::{MountSpec, PortForwardSpec};

    use super::{
        add_mount, add_port_forward, clone_distro, delete_distro, distro_root, ensure_distro_tree,
        list_distros, load_distro, remove_mount, remove_port_forward, update_network,
        update_resources, FakeStore,
    };

    #[test]
    fn materializes_and_loads_distro_tree() {
        let cfg = build_distro_config("ubuntu-dev", "ubuntu-24.04-x86_64-v1", "strati").unwrap();
        let mut store = FakeStore::default();
        ensure_distro_tree(&mut store, &cfg).unwrap();

        let loaded = load_distro(&mut store, "ubuntu-dev").unwrap();
        assert_eq!(loaded.name, "ubuntu-dev");
        assert_eq!(loaded.owner, "strati");
        assert_eq!(loaded.network.mode, "nat");
    }

    #[test]
    fn roundtrips_mounts_and_port_forwards() {
        let mut cfg =
            build_distro_config("ubuntu-dev", "ubuntu-24.04-x86_64-v1", "strati").unwrap();
        cfg.mounts.push(MountSpec {
            id: String::from("workspace"),
            host_path: String::from("/Users/strati/work"),
            guest_path: String::from("/mnt/work"),
            mode: String::from("readwrite"),
            metadata_mode: String::from("relaxed"),
            case_mode: String::from("host-native"),
            exec_policy: String::from("inherit"),
            watch_policy: String::from("best-effort"),
            description: String::from("Developer workspace"),
        });
        cfg.port_forwards.push(PortForwardSpec {
            id: String::from("web"),
            listen_address: String::from("127.0.0.1"),
            listen_port: 3000,
            guest_port: 3000,
            protocol: String::from("tcp"),
            description: String::from("Local web preview"),
        });
        let mut store = FakeStore::default();

        ensure_distro_tree(&mut store, &cfg).unwrap();
        let loaded = load_distro(&mut store, "ubuntu-dev").unwrap();

        assert_eq!(loaded.mounts, cfg.mounts);
        assert_eq!(loaded.port_forwards, cfg.port_forwards);
    }

    #[test]
    fn lists_known_distros() {
        let cfg = build_distro_config("ubuntu-dev", "ubuntu-24.04-x86_64-v1", "strati").unwrap();
        let mut store = FakeStore::default();
        store.mkdir("platform/asl/distros").unwrap();
        store.mkdir(&distro_root("ubuntu-dev")).unwrap();
        ensure_distro_tree(&mut store, &cfg).unwrap();
        assert_eq!(
            list_distros(&mut store).unwrap(),
            alloc::vec![alloc::string::String::from("ubuntu-dev")]
        );
    }

    #[test]
    fn adds_and_removes_mount_without_rewriting_distro() {
        let cfg = build_distro_config("ubuntu-dev", "ubuntu-24.04-x86_64-v1", "strati").unwrap();
        let mut store = FakeStore::default();
        ensure_distro_tree(&mut store, &cfg).unwrap();
        let mount = MountSpec {
            id: String::from("workspace"),
            host_path: String::from("/Users/strati/work"),
            guest_path: String::from("/mnt/work"),
            mode: String::from("readwrite"),
            metadata_mode: String::from("relaxed"),
            case_mode: String::from("host-native"),
            exec_policy: String::from("inherit"),
            watch_policy: String::from("best-effort"),
            description: String::from("Workspace"),
        };

        add_mount(&mut store, "ubuntu-dev", &mount).unwrap();
        assert_eq!(
            load_distro(&mut store, "ubuntu-dev").unwrap().mounts,
            alloc::vec![mount.clone()]
        );

        remove_mount(&mut store, "ubuntu-dev", "workspace").unwrap();
        assert!(load_distro(&mut store, "ubuntu-dev")
            .unwrap()
            .mounts
            .is_empty());
    }

    #[test]
    fn adds_and_removes_port_forward_without_rewriting_distro() {
        let cfg = build_distro_config("ubuntu-dev", "ubuntu-24.04-x86_64-v1", "strati").unwrap();
        let mut store = FakeStore::default();
        ensure_distro_tree(&mut store, &cfg).unwrap();
        let rule = PortForwardSpec {
            id: String::from("web"),
            listen_address: String::from("127.0.0.1"),
            listen_port: 3000,
            guest_port: 3000,
            protocol: String::from("tcp"),
            description: String::from("Web"),
        };

        add_port_forward(&mut store, "ubuntu-dev", &rule).unwrap();
        assert_eq!(
            load_distro(&mut store, "ubuntu-dev").unwrap().port_forwards,
            alloc::vec![rule.clone()]
        );

        remove_port_forward(&mut store, "ubuntu-dev", "web").unwrap();
        assert!(load_distro(&mut store, "ubuntu-dev")
            .unwrap()
            .port_forwards
            .is_empty());
    }

    #[test]
    fn updates_resources_in_place() {
        let cfg = build_distro_config("ubuntu-dev", "ubuntu-24.04-x86_64-v1", "strati").unwrap();
        let mut store = FakeStore::default();
        ensure_distro_tree(&mut store, &cfg).unwrap();

        let updated = update_resources(&mut store, "ubuntu-dev", Some(4096), Some(4)).unwrap();

        assert_eq!(updated.resources.memory_mb, 4096);
        assert_eq!(updated.resources.vcpu_count, 4);
    }

    #[test]
    fn updates_network_policy_in_place() {
        let cfg = build_distro_config("ubuntu-dev", "ubuntu-24.04-x86_64-v1", "strati").unwrap();
        let mut store = FakeStore::default();
        ensure_distro_tree(&mut store, &cfg).unwrap();

        let updated = update_network(
            &mut store,
            "ubuntu-dev",
            Some("nat"),
            Some("host-broker"),
            Some(false),
        )
        .unwrap();

        assert_eq!(updated.network.mode, "nat");
        assert_eq!(updated.network.dns_mode, "host-broker");
        assert!(!updated.network.allow_outbound);
    }

    #[test]
    fn clones_distro_with_new_storage_and_without_ports_by_default() {
        let mut cfg =
            build_distro_config("ubuntu-dev", "ubuntu-24.04-x86_64-v1", "strati").unwrap();
        cfg.port_forwards.push(PortForwardSpec {
            id: String::from("web"),
            listen_address: String::from("127.0.0.1"),
            listen_port: 3000,
            guest_port: 3000,
            protocol: String::from("tcp"),
            description: String::from("Web"),
        });
        let mut store = FakeStore::default();
        ensure_distro_tree(&mut store, &cfg).unwrap();

        let cloned = clone_distro(
            &mut store,
            "ubuntu-dev",
            "ubuntu-copy",
            Some("ops"),
            true,
            false,
        )
        .unwrap();

        assert_eq!(cloned.name, "ubuntu-copy");
        assert_eq!(cloned.owner, "ops");
        assert!(cloned
            .storage
            .overlay_image_path
            .contains("/ubuntu-copy/images/overlay.img"));
        assert!(cloned.port_forwards.is_empty());
        assert!(cloned.metadata.notes.contains("cloned from ubuntu-dev"));
    }

    #[test]
    fn deletes_distro_tree() {
        let cfg = build_distro_config("ubuntu-dev", "ubuntu-24.04-x86_64-v1", "strati").unwrap();
        let mut store = FakeStore::default();
        ensure_distro_tree(&mut store, &cfg).unwrap();

        let deleted = delete_distro(&mut store, "ubuntu-dev").unwrap();

        assert_eq!(deleted.name, "ubuntu-dev");
        assert!(load_distro(&mut store, "ubuntu-dev").is_err());
    }
}
