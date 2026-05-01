// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT

use libconf_schema::{
    default_bool, default_external_ref, default_int, default_string, manifest, RegistryScope,
    ServiceSchema,
};

const ANYMAIL_DIRS: &[&str] = &["config", "config/accounts", "state"];
const ANYMAIL_DEFAULTS: &[libconf_schema::DefaultEntry<'static>] = &[
    default_int("config/accounts_schema_version", 0),
    default_int("config/accounts_count", 0),
    default_string("config/contacts_json", ""),
    default_bool("config/check_on_startup", true),
    default_string("config/theme", "dark"),
    default_int("config/active_account", 0),
    default_external_ref("state/mail_store_ref", ""),
];
const ANYMAIL_MIGRATIONS: &[libconf_schema::MigrationStep<'static>] = &[];
const ANYMAIL_MANIFEST: libconf_schema::RegistryManifest<'static> = manifest(
    "apps/anymail",
    RegistryScope::User,
    3,
    ANYMAIL_DIRS,
    ANYMAIL_DEFAULTS,
    ANYMAIL_MIGRATIONS,
);

pub fn schema() -> ServiceSchema<'static> {
    ServiceSchema::new("anymail", &ANYMAIL_MANIFEST)
}
