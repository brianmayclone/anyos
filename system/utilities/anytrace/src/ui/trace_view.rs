//! Execution trace view using DataGrid.

use libanyui_client as ui;
use ui::Widget;
use ui::ColumnDef;
use crate::logic::traces::TraceEntry;
use crate::util::format::{hex64, fmt_u64};

/// Trace view panel.
pub struct TraceView {
    pub grid: ui::DataGrid,
}

impl TraceView {
    /// Create the trace view.
    pub fn new(_parent: &impl Widget) -> Self {
        let grid = ui::DataGrid::new(600, 300);
        grid.set_dock(ui::DOCK_FILL);
        grid.set_columns(&[
            ColumnDef::new("#").width(60),
            ColumnDef::new("TID").width(50),
            ColumnDef::new("RIP").width(180),
            ColumnDef::new("Instruction").width(300),
        ]);
        Self { grid }
    }

    /// Refresh the trace list.
    pub fn update(&self, entries: &[TraceEntry]) {
        self.grid.set_row_count(entries.len() as u32);
        for (i, entry) in entries.iter().enumerate() {
            let row = i as u32;
            self.grid.set_cell(row, 0, &fmt_u64(entry.seq as u64));
            self.grid.set_cell(row, 1, &fmt_u64(entry.tid as u64));
            self.grid.set_cell(row, 2, &hex64(entry.rip));
            let instr = alloc::format!("{} {}", entry.mnemonic, entry.operands);
            self.grid.set_cell(row, 3, &instr);
        }
    }
}
