//! Data structures for the Settings app.

use alloc::string::String;

// ── Built-in page identifiers ───────────────────────────────────────────────

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

// ── Page entry (shown in the sidebar) ───────────────────────────────────────

pub struct PageEntry {
    pub id: BuiltinId,
    pub name: String,
    pub icon: String,
    pub category: String,
    pub order: i64,
    /// Control ID of the panel View (0 = not yet built).
    pub panel_id: u32,
}
