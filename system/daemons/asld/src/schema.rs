use libconf::{ConfError};
use libconf_schema::{
    default_bool, default_int, default_string, manifest, RegistryScope, ServiceSchema,
};

pub const ASL_DIRS: &[&str] = &["images", "distros", "profiles", "profiles/default"];
pub const ASL_DEFAULTS: &[libconf_schema::DefaultEntry<'static>] = &[
    default_string("profiles/default/network_mode", "nat"),
    default_string("profiles/default/dns_mode", "host-broker"),
    default_int("profiles/default/memory_mb", 2048),
    default_int("profiles/default/vcpu_count", 2),
    default_bool("profiles/default/agent_enabled", true),
    default_bool("profiles/default/fallback_console_enabled", true),
];
pub const ASL_MIGRATIONS: &[libconf_schema::MigrationStep<'static>] = &[];
pub const ASL_MANIFEST: libconf_schema::RegistryManifest<'static> = manifest(
    "platform/asl",
    RegistryScope::System,
    1,
    ASL_DIRS,
    ASL_DEFAULTS,
    ASL_MIGRATIONS,
);
pub const ASL_SCHEMA: ServiceSchema<'static> = ServiceSchema::new("asld", &ASL_MANIFEST);

pub fn register_manifest() -> Result<(u32, u32), ConfError> {
    ASL_SCHEMA.register()
}

#[cfg(test)]
mod tests {
    use super::{ASL_DEFAULTS, ASL_DIRS, ASL_MANIFEST};

    #[test]
    fn manifest_uses_expected_namespace() {
        assert_eq!(ASL_MANIFEST.namespace, "platform/asl");
        assert_eq!(ASL_MANIFEST.version, 1);
    }

    #[test]
    fn manifest_contains_required_roots_and_defaults() {
        assert!(ASL_DIRS.contains(&"distros"));
        assert!(ASL_DIRS.contains(&"profiles/default"));
        assert!(ASL_DEFAULTS.iter().any(|d| d.path == "profiles/default/network_mode"));
    }
}
