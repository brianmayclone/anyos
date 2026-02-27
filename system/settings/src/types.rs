//! Data structures for the modular Settings app.

use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::json::Value;

// ── Built-in module identifiers ─────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
pub enum BuiltinId {
    Dashboard,
    General,
    Display,
    Apps,
    Devices,
    Network,
    Update,
}

// ── External module descriptor (loaded from JSON) ───────────────────────────

#[derive(Clone, Copy, PartialEq)]
pub enum ConfigFormat {
    KeyValue,
    Pipe,
    Crontab,
    Json,
}

pub struct ColumnSpec {
    pub key: String,
    pub label: String,
    pub placeholder: String,
    pub width: u32,
}

pub enum FieldType {
    Text,
    Number { min: i64, max: i64 },
    Bool,
    Choice { options: Vec<String> },
    Path { browse_folder: bool },
    Table { columns: Vec<ColumnSpec> },
}

pub struct FieldDef {
    pub key: String,
    pub label: String,
    pub field_type: FieldType,
    pub default_str: String,
}

pub struct ModuleDescriptor {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub category: String,
    pub description: String,
    pub config_path: String,
    pub resolved_path: String,
    pub on_apply_ipc: String,
    pub order: i64,
    pub config_format: ConfigFormat,
    pub fields: Vec<FieldDef>,
}

// ── Unified module entry (built-in or external) ─────────────────────────────

pub enum ModuleKind {
    Builtin(BuiltinId),
    External(ModuleDescriptor),
}

pub struct ModuleEntry {
    pub kind: ModuleKind,
    pub name: String,
    pub icon: String,
    pub category: String,
    pub order: i64,
    /// Control ID of the panel View for this module (0 = not yet built).
    pub panel_id: u32,
    /// Control IDs for each field (external modules only).
    pub field_ctrl_ids: Vec<u32>,
    /// Current config values (external modules only).
    pub values: Value,
    /// Whether values have been modified since last save.
    pub dirty: bool,
}
