//! Register display using DataGrid.

use libanyui_client as ui;
use ui::Widget;
use ui::ColumnDef;
use anyos_std::debug::DebugRegs;
use crate::util::format::{hex64, fmt_u64, REG_NAMES};

/// Register view panel.
pub struct RegistersView {
    pub grid: ui::DataGrid,
    prev_regs: [u64; 19],
}

impl RegistersView {
    /// Create the registers view.
    pub fn new(_parent: &impl Widget) -> Self {
        let grid = ui::DataGrid::new(420, 600);
        grid.set_dock(ui::DOCK_FILL);
        grid.set_columns(&[
            ColumnDef::new("Register").width(80),
            ColumnDef::new("Hex").width(180),
            ColumnDef::new("Decimal").width(160),
        ]);

        Self {
            grid,
            prev_regs: [0; 19],
        }
    }

    /// Update the register display with new values.
    pub fn update(&mut self, regs: &DebugRegs) {
        let vals = regs_to_array(regs);
        let col_count = 3u32;
        let row_count = 19u32;

        self.grid.set_row_count(row_count);

        let mut colors = alloc::vec![0u32; (row_count * col_count) as usize];

        for i in 0..19 {
            let row = i as u32;
            self.grid.set_cell(row, 0, REG_NAMES[i]);
            self.grid.set_cell(row, 1, &hex64(vals[i]));
            self.grid.set_cell(row, 2, &fmt_u64(vals[i]));

            // Highlight changed registers in yellow
            if vals[i] != self.prev_regs[i] {
                let base = (row * col_count) as usize;
                colors[base + 1] = 0xFFFFEB3B;
                colors[base + 2] = 0xFFFFEB3B;
            }
        }

        self.grid.set_cell_colors(&colors);
        self.prev_regs = vals;
    }
}

/// Extract register values as an array for iteration.
fn regs_to_array(r: &DebugRegs) -> [u64; 19] {
    [
        r.rax, r.rbx, r.rcx, r.rdx, r.rsi, r.rdi, r.rbp,
        r.r8, r.r9, r.r10, r.r11, r.r12, r.r13, r.r14, r.r15,
        r.rsp, r.rip, r.rflags, r.cr3,
    ]
}
