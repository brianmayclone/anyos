use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use libdb_client::Database;

use crate::{ConfValue, NodeKind, RegistryEntry, Scope};

const CREATE_STATEMENTS: &[&str] = &[
    "CREATE TABLE registry (canonical_path TEXT, logical_path TEXT, scope INTEGER, owner_uid INTEGER, kind INTEGER, value_type INTEGER, value_text TEXT, value_int INTEGER, value_bool INTEGER, version INTEGER, updated_at INTEGER, writer_uid INTEGER, writer_name TEXT)",
    "CREATE TABLE audit (seq INTEGER, actor_uid INTEGER, owner_uid INTEGER, actor_name TEXT, tid INTEGER, action TEXT, scope INTEGER, logical_path TEXT, status TEXT, detail TEXT, version INTEGER, at_ms INTEGER)",
    "CREATE TABLE schemas (scope INTEGER, owner_uid INTEGER, namespace TEXT, schema_version INTEGER, applied_version INTEGER, manifest_text TEXT, updated_at INTEGER, writer_name TEXT)",
    "CREATE UNIQUE INDEX registry_canonical_path_idx ON registry (canonical_path)",
    "CREATE INDEX schemas_namespace_idx ON schemas (namespace)",
];

const ALTER_STATEMENTS: &[&str] = &[
    "ALTER TABLE audit ADD COLUMN owner_uid INTEGER",
];

#[derive(Clone)]
pub struct AuditEntry {
    pub seq: u64,
    pub actor_uid: u16,
    pub owner_uid: u16,
    pub actor_name: String,
    pub tid: u32,
    pub action: String,
    pub scope: Scope,
    pub logical_path: String,
    pub status: String,
    pub detail: String,
    pub version: u64,
    pub at_ms: u64,
}

pub fn init_tables(db: &Database) {
    for sql in CREATE_STATEMENTS {
        if let Err(err) = db.exec(sql) {
            if !err.contains("already exists") {
                anyos_std::println!("confd: CREATE TABLE error: {}", err);
            }
        }
    }
    for sql in ALTER_STATEMENTS {
        if let Err(err) = db.exec(sql) {
            if !err.contains("duplicate column") && !err.contains("already exists") {
                anyos_std::println!("confd: ALTER TABLE error: {}", err);
            }
        }
    }
    if let Err(err) = db.flush() {
        anyos_std::println!("confd: database flush after init_tables failed: {}", err);
    }
}

pub fn count_rows(db: &Database, table: &str) -> Option<u64> {
    let sql = format!("SELECT * FROM {}", table);
    let Ok(result) = db.query(&sql) else {
        return None;
    };
    Some(result.row_count() as u64)
}

pub fn load_entries(db: &Database) -> Vec<RegistryEntry> {
    let mut entries = Vec::new();
    let sql = "SELECT canonical_path, logical_path, scope, owner_uid, kind, value_type, value_text, value_int, value_bool, version, updated_at, writer_uid, writer_name FROM registry ORDER BY canonical_path";
    let Ok(result) = db.query(sql) else {
        return entries;
    };

    for row in 0..result.row_count() {
        let canonical_path = result.get_text(row, 0).unwrap_or_default();
        let logical_path = result.get_text(row, 1).unwrap_or_default();
        let scope = match result.get_int(row, 2).unwrap_or(0) {
            1 => Scope::System,
            2 => Scope::User,
            _ => continue,
        };
        let owner_uid = result.get_int(row, 3).unwrap_or(0).max(0) as u16;
        let kind = match result.get_int(row, 4).unwrap_or(0) {
            1 => NodeKind::Directory,
            2 => NodeKind::Value,
            _ => continue,
        };
        let value = match result.get_int(row, 5).unwrap_or(0) {
            1 => Some(ConfValue::String(result.get_text(row, 6).unwrap_or_default())),
            2 => Some(ConfValue::Int(result.get_int(row, 7).unwrap_or(0))),
            3 => Some(ConfValue::Bool(result.get_int(row, 8).unwrap_or(0) != 0)),
            4 => Some(ConfValue::ExternalRef(result.get_text(row, 6).unwrap_or_default())),
            _ => None,
        };
        let version = result.get_int(row, 9).unwrap_or(0).max(0) as u64;
        let updated_at = result.get_int(row, 10).unwrap_or(0).max(0) as u64;
        let writer_uid = result.get_int(row, 11).unwrap_or(0).max(0) as u16;
        let writer_name = result.get_text(row, 12).unwrap_or_default();

        entries.push(RegistryEntry {
            canonical_path,
            logical_path,
            scope,
            owner_uid,
            kind,
            value,
            version,
            updated_at,
            writer_uid,
            writer_name,
        });
    }

    entries
}

pub fn load_next_audit_seq(db: &Database) -> u64 {
    let Ok(result) = db.query("SELECT seq FROM audit ORDER BY seq DESC LIMIT 1") else {
        return 1;
    };
    if result.row_count() == 0 {
        1
    } else {
        result.get_int(0, 0).unwrap_or(0).max(0) as u64 + 1
    }
}

pub fn persist_entry(db: &Database, entry: &RegistryEntry) -> Result<(), String> {
    delete_entry(db, &entry.canonical_path)?;

    let (value_type, value_text, value_int, value_bool) = match &entry.value {
        Some(ConfValue::String(s)) => (1, format!("'{}'", escape_sql(s)), String::from("0"), String::from("0")),
        Some(ConfValue::Int(v)) => (2, String::from("''"), format!("{}", *v), String::from("0")),
        Some(ConfValue::Bool(v)) => (
            3,
            String::from("''"),
            String::from("0"),
            if *v { String::from("1") } else { String::from("0") },
        ),
        Some(ConfValue::ExternalRef(path)) => (
            4,
            format!("'{}'", escape_sql(path)),
            String::from("0"),
            String::from("0"),
        ),
        None => (0, String::from("''"), String::from("0"), String::from("0")),
    };

    let sql = format!(
        "INSERT INTO registry (canonical_path, logical_path, scope, owner_uid, kind, value_type, value_text, value_int, value_bool, version, updated_at, writer_uid, writer_name) VALUES ('{}', '{}', {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, '{}')",
        escape_sql(&entry.canonical_path),
        escape_sql(&entry.logical_path),
        entry.scope as i64,
        entry.owner_uid as i64,
        entry.kind as i64,
        value_type,
        value_text,
        value_int,
        value_bool,
        entry.version as i64,
        entry.updated_at as i64,
        entry.writer_uid as i64,
        escape_sql(&entry.writer_name),
    );
    db.exec(&sql)?;
    let _ = db.flush();
    Ok(())
}

pub fn delete_entry(db: &Database, canonical_path: &str) -> Result<(), String> {
    let sql = format!(
        "DELETE FROM registry WHERE canonical_path = '{}'",
        escape_sql(canonical_path)
    );
    db.exec(&sql)?;
    let _ = db.flush();
    Ok(())
}

pub fn append_audit(
    db: &Database,
    seq: u64,
    actor_uid: u16,
    owner_uid: u16,
    actor_name: &str,
    tid: u32,
    action: &str,
    scope: Scope,
    logical_path: &str,
    status: &str,
    detail: &str,
    version: u64,
    at_ms: u64,
) {
    let sql = format!(
        "INSERT INTO audit (seq, actor_uid, owner_uid, actor_name, tid, action, scope, logical_path, status, detail, version, at_ms) VALUES ({}, {}, {}, '{}', {}, '{}', {}, '{}', '{}', '{}', {}, {})",
        seq as i64,
        actor_uid as i64,
        owner_uid as i64,
        escape_sql(actor_name),
        tid as i64,
        escape_sql(action),
        scope as i64,
        escape_sql(logical_path),
        escape_sql(status),
        escape_sql(detail),
        version as i64,
        at_ms as i64,
    );
    let _ = db.exec(&sql);
    let _ = db.flush();
}

pub fn query_audit(
    db: &Database,
    scope: Scope,
    owner_uid: u16,
    logical_prefix: &str,
    limit: u32,
) -> Vec<AuditEntry> {
    let mut entries = Vec::new();
    let safe_limit = limit.max(1).min(500);
    let prefix_sql = if logical_prefix.is_empty() {
        String::new()
    } else {
        format!(
            " AND (logical_path = '{}' OR logical_path LIKE '{}/%')",
            escape_sql(logical_prefix),
            escape_sql(logical_prefix),
        )
    };
    let sql = format!(
        "SELECT seq, actor_uid, owner_uid, actor_name, tid, action, scope, logical_path, status, detail, version, at_ms FROM audit WHERE scope = {} AND owner_uid = {}{} ORDER BY seq DESC LIMIT {}",
        scope as i64,
        owner_uid as i64,
        prefix_sql,
        safe_limit as i64,
    );
    let Ok(result) = db.query(&sql) else {
        return entries;
    };

    for row in 0..result.row_count() {
        let scope = match result.get_int(row, 6).unwrap_or(0) {
            1 => Scope::System,
            2 => Scope::User,
            _ => continue,
        };
        entries.push(AuditEntry {
            seq: result.get_int(row, 0).unwrap_or(0).max(0) as u64,
            actor_uid: result.get_int(row, 1).unwrap_or(0).max(0) as u16,
            owner_uid: result.get_int(row, 2).unwrap_or(0).max(0) as u16,
            actor_name: result.get_text(row, 3).unwrap_or_default(),
            tid: result.get_int(row, 4).unwrap_or(0).max(0) as u32,
            action: result.get_text(row, 5).unwrap_or_default(),
            scope,
            logical_path: result.get_text(row, 7).unwrap_or_default(),
            status: result.get_text(row, 8).unwrap_or_default(),
            detail: result.get_text(row, 9).unwrap_or_default(),
            version: result.get_int(row, 10).unwrap_or(0).max(0) as u64,
            at_ms: result.get_int(row, 11).unwrap_or(0).max(0) as u64,
        });
    }

    entries
}

pub fn load_schema_versions(
    db: &Database,
    scope: Scope,
    owner_uid: u16,
    namespace: &str,
) -> (u32, u32) {
    let sql = format!(
        "SELECT schema_version, applied_version FROM schemas WHERE scope = {} AND owner_uid = {} AND namespace = '{}'",
        scope as i64,
        owner_uid as i64,
        escape_sql(namespace),
    );
    let Ok(result) = db.query(&sql) else {
        return (0, 0);
    };
    if result.row_count() == 0 {
        return (0, 0);
    }
    let schema_version = result.get_int(0, 0).unwrap_or(0).max(0) as u32;
    let applied_version = result.get_int(0, 1).unwrap_or(0).max(0) as u32;
    (schema_version, applied_version)
}

pub fn persist_schema(
    db: &Database,
    scope: Scope,
    owner_uid: u16,
    namespace: &str,
    schema_version: u32,
    applied_version: u32,
    manifest_text: &str,
    updated_at: u64,
    writer_name: &str,
) -> Result<(), String> {
    let delete_sql = format!(
        "DELETE FROM schemas WHERE scope = {} AND owner_uid = {} AND namespace = '{}'",
        scope as i64,
        owner_uid as i64,
        escape_sql(namespace),
    );
    db.exec(&delete_sql)?;

    let insert_sql = format!(
        "INSERT INTO schemas (scope, owner_uid, namespace, schema_version, applied_version, manifest_text, updated_at, writer_name) VALUES ({}, {}, '{}', {}, {}, '{}', {}, '{}')",
        scope as i64,
        owner_uid as i64,
        escape_sql(namespace),
        schema_version as i64,
        applied_version as i64,
        escape_sql(manifest_text),
        updated_at as i64,
        escape_sql(writer_name),
    );
    db.exec(&insert_sql)?;
    let _ = db.flush();
    Ok(())
}

fn escape_sql(s: &str) -> String {
    if !s.contains('\'') {
        return String::from(s);
    }
    let mut out = String::with_capacity(s.len() + 4);
    for ch in s.chars() {
        if ch == '\'' {
            out.push('\'');
        }
        out.push(ch);
    }
    out
}
