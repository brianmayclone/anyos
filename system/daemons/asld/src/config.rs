use alloc::string::{String, ToString};
use alloc::vec::Vec;

use libconf::{ConfClient, ConfError, ConfItem, ConfValue, RegistryScope};

use crate::errors::AsldError;
use crate::model::{AgentPolicy, DistroConfig, DistroMetadata, LifecyclePolicy, NetworkPolicy, Resources, StorageSpec};
use crate::{mounts, network};

const DISTROS_ROOT: &str = "platform/asl/distros";

pub trait ConfigStore {
    fn mkdir(&mut self, path: &str) -> Result<(), AsldError>;
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

    fn set_string(&mut self, path: &str, value: &str) -> Result<(), AsldError> {
        self.client.set(RegistryScope::System, path, ConfValue::String(String::from(value)))?;
        Ok(())
    }

    fn set_int(&mut self, path: &str, value: i64) -> Result<(), AsldError> {
        self.client.set(RegistryScope::System, path, ConfValue::Int(value))?;
        Ok(())
    }

    fn set_bool(&mut self, path: &str, value: bool) -> Result<(), AsldError> {
        self.client.set(RegistryScope::System, path, ConfValue::Bool(value))?;
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
        Ok(items
            .iter()
            .filter_map(last_segment_of)
            .collect())
    }
}

pub fn distro_root(name: &str) -> String {
    alloc::format!("{DISTROS_ROOT}/{name}")
}

pub fn list_distros<S: ConfigStore>(store: &mut S) -> Result<Vec<String>, AsldError> {
    store.list_children(DISTROS_ROOT)
}

pub fn ensure_distro_tree<S: ConfigStore>(store: &mut S, config: &DistroConfig) -> Result<(), AsldError> {
    let root = distro_root(&config.name);
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
        store.set_string(&alloc::format!("{mount_root}/guest_path"), &mount.guest_path)?;
        store.set_string(&alloc::format!("{mount_root}/mode"), &mount.mode)?;
        store.set_string(&alloc::format!("{mount_root}/metadata_mode"), &mount.metadata_mode)?;
        store.set_string(&alloc::format!("{mount_root}/case_mode"), &mount.case_mode)?;
        store.set_string(&alloc::format!("{mount_root}/exec_policy"), &mount.exec_policy)?;
        store.set_string(&alloc::format!("{mount_root}/watch_policy"), &mount.watch_policy)?;
        store.set_string(&alloc::format!("{mount_root}/description"), &mount.description)?;
    }

    for rule in &config.port_forwards {
        network::validate_port_forward(rule)?;
        let rule_root = alloc::format!("{root}/port_forwards/{}", rule.id);
        store.mkdir(&rule_root)?;
        store.set_string(&alloc::format!("{rule_root}/listen_address"), &rule.listen_address)?;
        store.set_int(&alloc::format!("{rule_root}/listen_port"), rule.listen_port as i64)?;
        store.set_int(&alloc::format!("{rule_root}/guest_port"), rule.guest_port as i64)?;
        store.set_string(&alloc::format!("{rule_root}/protocol"), &rule.protocol)?;
        store.set_string(&alloc::format!("{rule_root}/description"), &rule.description)?;
    }

    Ok(())
}

pub fn load_distro<S: ConfigStore>(store: &mut S, name: &str) -> Result<DistroConfig, AsldError> {
    let root = distro_root(name);
    let id = required_string(store, &alloc::format!("{root}/id"), "id")?;
    let owner = required_string(store, &alloc::format!("{root}/owner"), "owner")?;
    let base_image_ref = required_string(store, &alloc::format!("{root}/base_image_ref"), "base_image_ref")?;
    let kernel_profile = required_string(store, &alloc::format!("{root}/kernel_profile"), "kernel_profile")?;

    let resources = Resources {
        memory_mb: store.get_int(&alloc::format!("{root}/resources/memory_mb"))?.unwrap_or(2048).max(256) as u32,
        vcpu_count: store.get_int(&alloc::format!("{root}/resources/vcpu_count"))?.unwrap_or(2).max(1) as u16,
        autostart: store.get_bool(&alloc::format!("{root}/resources/autostart"))?.unwrap_or(false),
    };
    let storage = StorageSpec {
        layout: required_string(store, &alloc::format!("{root}/storage/layout"), "storage.layout")?,
        base_image_path: required_string(store, &alloc::format!("{root}/storage/base_image_path"), "storage.base_image_path")?,
        overlay_image_path: required_string(store, &alloc::format!("{root}/storage/overlay_image_path"), "storage.overlay_image_path")?,
        state_image_path: store.get_string(&alloc::format!("{root}/storage/state_image_path"))?.unwrap_or_default(),
        state_image_enabled: store.get_bool(&alloc::format!("{root}/storage/state_image_enabled"))?.unwrap_or(false),
    };
    let network = NetworkPolicy {
        mode: store.get_string(&alloc::format!("{root}/network/mode"))?.unwrap_or_else(|| String::from("nat")),
        dns_mode: store.get_string(&alloc::format!("{root}/network/dns_mode"))?.unwrap_or_else(|| String::from("host-broker")),
        allow_outbound: store.get_bool(&alloc::format!("{root}/network/allow_outbound"))?.unwrap_or(true),
    };
    let agent = AgentPolicy {
        enabled: store.get_bool(&alloc::format!("{root}/agent/enabled"))?.unwrap_or(true),
        required_for_rich_integration: store.get_bool(&alloc::format!("{root}/agent/required_for_rich_integration"))?.unwrap_or(true),
        fallback_console_enabled: store.get_bool(&alloc::format!("{root}/agent/fallback_console_enabled"))?.unwrap_or(true),
    };
    let lifecycle = LifecyclePolicy {
        restart_on_failure: store.get_bool(&alloc::format!("{root}/lifecycle/restart_on_failure"))?.unwrap_or(true),
        shutdown_timeout_ms: store.get_int(&alloc::format!("{root}/lifecycle/shutdown_timeout_ms"))?.unwrap_or(10_000) as u32,
        boot_timeout_ms: store.get_int(&alloc::format!("{root}/lifecycle/boot_timeout_ms"))?.unwrap_or(30_000) as u32,
    };
    let metadata = DistroMetadata {
        distro_family: store.get_string(&alloc::format!("{root}/metadata/distro_family"))?.unwrap_or_default(),
        distro_version: store.get_string(&alloc::format!("{root}/metadata/distro_version"))?.unwrap_or_default(),
        notes: store.get_string(&alloc::format!("{root}/metadata/notes"))?.unwrap_or_default(),
    };

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
        mounts: Vec::new(),
        port_forwards: Vec::new(),
        agent,
        lifecycle,
        metadata,
    })
}

fn write_scalar_fields<S: ConfigStore>(store: &mut S, root: &str, config: &DistroConfig) -> Result<(), AsldError> {
    store.set_int(&alloc::format!("{root}/schema_version"), config.schema_version as i64)?;
    store.set_string(&alloc::format!("{root}/id"), &config.id)?;
    store.set_string(&alloc::format!("{root}/name"), &config.name)?;
    store.set_string(&alloc::format!("{root}/owner"), &config.owner)?;
    store.set_string(&alloc::format!("{root}/base_image_ref"), &config.base_image_ref)?;
    store.set_string(&alloc::format!("{root}/kernel_profile"), &config.kernel_profile)?;

    store.set_int(&alloc::format!("{root}/resources/memory_mb"), config.resources.memory_mb as i64)?;
    store.set_int(&alloc::format!("{root}/resources/vcpu_count"), config.resources.vcpu_count as i64)?;
    store.set_bool(&alloc::format!("{root}/resources/autostart"), config.resources.autostart)?;

    store.set_string(&alloc::format!("{root}/storage/layout"), &config.storage.layout)?;
    store.set_string(&alloc::format!("{root}/storage/base_image_path"), &config.storage.base_image_path)?;
    store.set_string(&alloc::format!("{root}/storage/overlay_image_path"), &config.storage.overlay_image_path)?;
    store.set_string(&alloc::format!("{root}/storage/state_image_path"), &config.storage.state_image_path)?;
    store.set_bool(&alloc::format!("{root}/storage/state_image_enabled"), config.storage.state_image_enabled)?;

    store.set_string(&alloc::format!("{root}/network/mode"), &config.network.mode)?;
    store.set_string(&alloc::format!("{root}/network/dns_mode"), &config.network.dns_mode)?;
    store.set_bool(&alloc::format!("{root}/network/allow_outbound"), config.network.allow_outbound)?;

    store.set_bool(&alloc::format!("{root}/agent/enabled"), config.agent.enabled)?;
    store.set_bool(&alloc::format!("{root}/agent/required_for_rich_integration"), config.agent.required_for_rich_integration)?;
    store.set_bool(&alloc::format!("{root}/agent/fallback_console_enabled"), config.agent.fallback_console_enabled)?;

    store.set_bool(&alloc::format!("{root}/lifecycle/restart_on_failure"), config.lifecycle.restart_on_failure)?;
    store.set_int(&alloc::format!("{root}/lifecycle/shutdown_timeout_ms"), config.lifecycle.shutdown_timeout_ms as i64)?;
    store.set_int(&alloc::format!("{root}/lifecycle/boot_timeout_ms"), config.lifecycle.boot_timeout_ms as i64)?;

    store.set_string(&alloc::format!("{root}/metadata/distro_family"), &config.metadata.distro_family)?;
    store.set_string(&alloc::format!("{root}/metadata/distro_version"), &config.metadata.distro_version)?;
    store.set_string(&alloc::format!("{root}/metadata/notes"), &config.metadata.notes)?;
    Ok(())
}

fn required_string<S: ConfigStore>(store: &mut S, path: &str, field: &'static str) -> Result<String, AsldError> {
    store.get_string(path)?.ok_or(AsldError::MissingField(field))
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

    fn set_string(&mut self, path: &str, value: &str) -> Result<(), AsldError> {
        self.entries.insert(String::from(path), FakeValue::String(String::from(value)));
        Ok(())
    }

    fn set_int(&mut self, path: &str, value: i64) -> Result<(), AsldError> {
        self.entries.insert(String::from(path), FakeValue::Int(value));
        Ok(())
    }

    fn set_bool(&mut self, path: &str, value: bool) -> Result<(), AsldError> {
        self.entries.insert(String::from(path), FakeValue::Bool(value));
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
    use crate::distro::build_distro_config;

    use super::{distro_root, ensure_distro_tree, list_distros, load_distro, FakeStore};

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
    fn lists_known_distros() {
        let cfg = build_distro_config("ubuntu-dev", "ubuntu-24.04-x86_64-v1", "strati").unwrap();
        let mut store = FakeStore::default();
        store.mkdir("platform/asl/distros").unwrap();
        store.mkdir(&distro_root("ubuntu-dev")).unwrap();
        ensure_distro_tree(&mut store, &cfg).unwrap();
        assert_eq!(list_distros(&mut store).unwrap(), alloc::vec![alloc::string::String::from("ubuntu-dev")]);
    }
}
