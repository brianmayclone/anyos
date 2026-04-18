#![no_std]

extern crate alloc;

use alloc::format;
use alloc::string::String;

pub use libconf::{
    ConfClient, ConfError, ConfItem, ConfValue, ConfValueRef, DefaultEntry, MigrationEntry,
    MigrationStep, NodeKind, RegistryManifest, RegistryScope,
};

pub const fn default_string(path: &'static str, value: &'static str) -> DefaultEntry<'static> {
    DefaultEntry {
        path,
        value: ConfValueRef::String(value),
    }
}

pub const fn default_int(path: &'static str, value: i64) -> DefaultEntry<'static> {
    DefaultEntry {
        path,
        value: ConfValueRef::Int(value),
    }
}

pub const fn default_bool(path: &'static str, value: bool) -> DefaultEntry<'static> {
    DefaultEntry {
        path,
        value: ConfValueRef::Bool(value),
    }
}

pub const fn migration_string(
    version: u32,
    path: &'static str,
    value: &'static str,
) -> MigrationStep<'static> {
    MigrationStep::Set(MigrationEntry {
        version,
        path,
        value: ConfValueRef::String(value),
    })
}

pub const fn migration_int(version: u32, path: &'static str, value: i64) -> MigrationStep<'static> {
    MigrationStep::Set(MigrationEntry {
        version,
        path,
        value: ConfValueRef::Int(value),
    })
}

pub const fn migration_bool(version: u32, path: &'static str, value: bool) -> MigrationStep<'static> {
    MigrationStep::Set(MigrationEntry {
        version,
        path,
        value: ConfValueRef::Bool(value),
    })
}

pub const fn migration_delete(version: u32, path: &'static str) -> MigrationStep<'static> {
    MigrationStep::Delete { version, path }
}

pub const fn migration_rename(
    version: u32,
    from: &'static str,
    to: &'static str,
) -> MigrationStep<'static> {
    MigrationStep::Rename { version, from, to }
}

pub const fn migration_copy(
    version: u32,
    from: &'static str,
    to: &'static str,
) -> MigrationStep<'static> {
    MigrationStep::Copy { version, from, to }
}

pub const fn manifest(
    namespace: &'static str,
    scope: RegistryScope,
    version: u32,
    directories: &'static [&'static str],
    defaults: &'static [DefaultEntry<'static>],
    migrations: &'static [MigrationStep<'static>],
) -> RegistryManifest<'static> {
    RegistryManifest {
        namespace,
        scope,
        version,
        directories,
        defaults,
        migrations,
    }
}

pub struct ServiceSchema<'a> {
    client_name: &'a str,
    manifest: &'a RegistryManifest<'a>,
}

impl<'a> ServiceSchema<'a> {
    pub const fn new(client_name: &'a str, manifest: &'a RegistryManifest<'a>) -> Self {
        Self {
            client_name,
            manifest,
        }
    }

    pub fn register(&self) -> Result<(u32, u32), ConfError> {
        let mut client = ConfClient::connect(self.client_name)?;
        client.register_manifest(self.manifest)
    }

    pub fn read_i64(&self, rel_path: &str) -> Option<i64> {
        match self.read_item(rel_path)?.value {
            Some(ConfValue::Int(v)) => Some(v),
            _ => None,
        }
    }

    pub fn read_bool(&self, rel_path: &str) -> Option<bool> {
        match self.read_item(rel_path)?.value {
            Some(ConfValue::Bool(v)) => Some(v),
            _ => None,
        }
    }

    pub fn read_string(&self, rel_path: &str) -> Option<String> {
        match self.read_item(rel_path)?.value {
            Some(ConfValue::String(v)) => Some(v),
            _ => None,
        }
    }

    pub fn write_i64(&self, rel_path: &str, value: i64) -> Result<ConfItem, ConfError> {
        self.write_value(rel_path, ConfValue::Int(value))
    }

    pub fn write_bool(&self, rel_path: &str, value: bool) -> Result<ConfItem, ConfError> {
        self.write_value(rel_path, ConfValue::Bool(value))
    }

    pub fn write_string(&self, rel_path: &str, value: &str) -> Result<ConfItem, ConfError> {
        self.write_value(rel_path, ConfValue::String(String::from(value)))
    }

    pub fn full_path(&self, rel_path: &str) -> String {
        join_namespace(self.manifest.namespace, rel_path)
    }

    fn read_item(&self, rel_path: &str) -> Option<ConfItem> {
        let mut client = ConfClient::connect(self.client_name).ok()?;
        let full_path = self.full_path(rel_path);
        client.get(self.manifest.scope, &full_path).ok()
    }

    fn write_value(&self, rel_path: &str, value: ConfValue) -> Result<ConfItem, ConfError> {
        let mut client = ConfClient::connect(self.client_name)?;
        let full_path = self.full_path(rel_path);
        client.set(self.manifest.scope, &full_path, value)
    }
}

fn join_namespace(namespace: &str, rel_path: &str) -> String {
    if rel_path.is_empty() {
        return String::from(namespace);
    }
    format!("{}/{}", namespace, rel_path)
}
