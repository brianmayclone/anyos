//! Persistence helpers for the AMI v1 state table.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use libdb_client::Database;

use crate::{AmiValue, StateEntry};

const CREATE_STATEMENTS: &[&str] = &[
    "CREATE TABLE state (key TEXT, type INTEGER, value_text TEXT, value_int INTEGER, value_bool INTEGER, version INTEGER, updated_at INTEGER, owner TEXT)",
    "CREATE UNIQUE INDEX state_key_idx ON state (key)",
];

pub fn init_tables(db: &Database) {
    for sql in CREATE_STATEMENTS {
        let _ = db.exec(sql);
    }
}

pub fn load_entries(db: &Database) -> Vec<StateEntry> {
    let mut entries = Vec::new();
    let sql = "SELECT key, type, value_text, value_int, value_bool, version, updated_at, owner FROM state ORDER BY key";
    let Ok(result) = db.query(sql) else {
        return entries;
    };

    for row in 0..result.row_count() {
        let key = result.get_text(row, 0).unwrap_or_default();
        let value_type = result.get_int(row, 1).unwrap_or(0);
        let value = match value_type {
            1 => AmiValue::String(result.get_text(row, 2).unwrap_or_default()),
            2 => AmiValue::Int(result.get_int(row, 3).unwrap_or(0)),
            3 => AmiValue::Bool(result.get_int(row, 4).unwrap_or(0) != 0),
            _ => continue,
        };
        let version = result.get_int(row, 5).unwrap_or(0).max(0) as u64;
        let updated_at = result.get_int(row, 6).unwrap_or(0).max(0) as u64;
        let owner = result.get_text(row, 7).unwrap_or_default();
        entries.push(StateEntry {
            key,
            value,
            version,
            updated_at,
            owner,
        });
    }

    entries
}

pub fn persist_entry(db: &Database, entry: &StateEntry) -> Result<(), String> {
    delete_entry(db, &entry.key)?;

    let (type_code, value_text, value_int, value_bool) = match &entry.value {
        AmiValue::String(s) => (
            1,
            format!("'{}'", escape_sql(s)),
            String::from("0"),
            String::from("0"),
        ),
        AmiValue::Int(v) => (2, String::from("''"), format!("{}", *v), String::from("0")),
        AmiValue::Bool(v) => (
            3,
            String::from("''"),
            String::from("0"),
            if *v {
                String::from("1")
            } else {
                String::from("0")
            },
        ),
    };

    let sql = format!(
        "INSERT INTO state (key, type, value_text, value_int, value_bool, version, updated_at, owner) VALUES ('{}', {}, {}, {}, {}, {}, {}, '{}')",
        escape_sql(&entry.key),
        type_code,
        value_text,
        value_int,
        value_bool,
        entry.version as i64,
        entry.updated_at as i64,
        escape_sql(&entry.owner),
    );
    db.exec(&sql)?;
    let _ = db.flush();
    Ok(())
}

pub fn delete_entry(db: &Database, key: &str) -> Result<(), String> {
    let sql = format!("DELETE FROM state WHERE key = '{}'", escape_sql(key));
    db.exec(&sql)?;
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
