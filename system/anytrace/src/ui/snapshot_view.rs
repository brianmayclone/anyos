//! Snapshot list view using DataGrid.

use libanyui_client as ui;
use ui::Widget;
use ui::ColumnDef;
use crate::logic::snapshots::Snapshot;
use crate::util::format::{hex64, fmt_u64};

/// Snapshot view panel.
pub struct SnapshotView {
    pub grid: ui::DataGrid,
}

impl SnapshotView {
    /// Create the snapshot view.
    pub fn new(_parent: &impl Widget) -> Self {
        let grid = ui::DataGrid::new(600, 300);
        grid.set_dock(ui::DOCK_FILL);
        grid.set_columns(&[
            ColumnDef::new("#").width(40),
            ColumnDef::new("Timestamp").width(100),
            ColumnDef::new("TID").width(60),
            ColumnDef::new("RIP").width(180),
            ColumnDef::new("Label").width(200),
        ]);
        Self { grid }
    }

    /// Refresh the snapshot list.
    pub fn update(&self, snapshots: &[Snapshot]) {
        self.grid.set_row_count(snapshots.len() as u32);
        for (i, snap) in snapshots.iter().enumerate() {
            let row = i as u32;
            self.grid.set_cell(row, 0, &fmt_u64(snap.index as u64));
            self.grid.set_cell(row, 1, &fmt_u64(snap.timestamp));
            self.grid.set_cell(row, 2, &fmt_u64(snap.tid as u64));
            self.grid.set_cell(row, 3, &hex64(snap.regs.rip));
            self.grid.set_cell(row, 4, &snap.label);
        }
    }
}
