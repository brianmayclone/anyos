use alloc::vec::Vec;
use crate::{Control, Widget, lib, events, KIND_DATA_GRID};
use crate::events::SelectionChangedEvent;

leaf_control!(DataGrid, KIND_DATA_GRID);

/// Column alignment constants.
pub const ALIGN_LEFT: u8 = 0;
pub const ALIGN_CENTER: u8 = 1;
pub const ALIGN_RIGHT: u8 = 2;

/// Selection mode constants.
pub const SELECTION_SINGLE: u32 = 0;
pub const SELECTION_MULTI: u32 = 1;

/// Sort direction constants.
pub const SORT_NONE: u32 = 0;
pub const SORT_ASCENDING: u32 = 1;
pub const SORT_DESCENDING: u32 = 2;

/// Sort type constants.
pub const SORT_STRING: u8 = 0;
pub const SORT_NUMERIC: u8 = 1;

/// Builder for column definitions.
pub struct ColumnDef {
    header: Vec<u8>,
    width: u32,
    align: u8,
    sort_type: u8,
}

impl ColumnDef {
    pub fn new(header: &str) -> Self {
        Self {
            header: header.as_bytes().to_vec(),
            width: 100,
            align: ALIGN_LEFT,
            sort_type: SORT_STRING,
        }
    }

    pub fn width(mut self, w: u32) -> Self {
        self.width = w;
        self
    }

    pub fn align(mut self, a: u8) -> Self {
        self.align = a;
        self
    }

    /// Set numeric sort mode for this column. When sorted, values are
    /// compared as numbers instead of lexicographically.
    pub fn numeric(mut self) -> Self {
        self.sort_type = SORT_NUMERIC;
        self
    }
}

impl DataGrid {
    /// Create a new empty DataGrid with the given display size.
    pub fn new(w: u32, h: u32) -> Self {
        let id = (lib().create_control)(KIND_DATA_GRID, core::ptr::null(), 0);
        (lib().set_size)(id, w, h);
        Self { ctrl: Control { id } }
    }

    /// Define columns from a slice of ColumnDef builders.
    pub fn set_columns(&self, cols: &[ColumnDef]) {
        let mut buf = Vec::new();
        for (i, col) in cols.iter().enumerate() {
            if i > 0 { buf.push(0x1E); } // record separator
            buf.extend_from_slice(&col.header);
            buf.push(0x1F); // unit separator
            write_u32_ascii(&mut buf, col.width);
            buf.push(0x1F);
            buf.push(b'0' + col.align);
            buf.push(0x1F);
            buf.push(b'0' + col.sort_type);
        }
        (lib().datagrid_set_columns)(self.ctrl.id, buf.as_ptr(), buf.len() as u32);
    }

    /// Get the number of columns.
    pub fn column_count(&self) -> u32 {
        (lib().datagrid_get_column_count)(self.ctrl.id)
    }

    /// Set the width of a specific column.
    pub fn set_column_width(&self, col_index: u32, width: u32) {
        (lib().datagrid_set_column_width)(self.ctrl.id, col_index, width);
    }

    /// Set the sort comparison type for a column.
    /// Use SORT_STRING (0) for lexicographic or SORT_NUMERIC (1) for numeric.
    pub fn set_column_sort_type(&self, col_index: u32, sort_type: u32) {
        (lib().datagrid_set_column_sort_type)(self.ctrl.id, col_index, sort_type);
    }

    /// Set all cell data at once. Each inner Vec is a row of cell strings.
    pub fn set_data(&self, rows: &[Vec<&str>]) {
        let mut buf = Vec::new();
        for (ri, row) in rows.iter().enumerate() {
            if ri > 0 { buf.push(0x1E); }
            for (ci, cell) in row.iter().enumerate() {
                if ci > 0 { buf.push(0x1F); }
                buf.extend_from_slice(cell.as_bytes());
            }
        }
        (lib().datagrid_set_data)(self.ctrl.id, buf.as_ptr(), buf.len() as u32);
    }

    /// Set a single cell's text.
    pub fn set_cell(&self, row: u32, col: u32, text: &str) {
        (lib().datagrid_set_cell)(self.ctrl.id, row, col, text.as_ptr(), text.len() as u32);
    }

    /// Get a cell's text into a buffer. Returns the number of bytes written.
    pub fn get_cell(&self, row: u32, col: u32, buf: &mut [u8]) -> u32 {
        (lib().datagrid_get_cell)(self.ctrl.id, row, col, buf.as_mut_ptr(), buf.len() as u32)
    }

    /// Set per-cell ARGB text colors. Flat array indexed as row * col_count + col.
    /// Pass 0 for default color.
    pub fn set_cell_colors(&self, colors: &[u32]) {
        (lib().datagrid_set_cell_colors)(self.ctrl.id, colors.as_ptr(), colors.len() as u32);
    }

    /// Set per-cell ARGB background colors. Flat array indexed as row * col_count + col.
    /// Pass 0 for default (no custom background).
    pub fn set_cell_bg_colors(&self, colors: &[u32]) {
        (lib().datagrid_set_cell_bg_colors)(self.ctrl.id, colors.as_ptr(), colors.len() as u32);
    }

    /// Set the number of rows (adding empty rows or truncating).
    pub fn set_row_count(&self, count: u32) {
        (lib().datagrid_set_row_count)(self.ctrl.id, count);
    }

    /// Get the number of rows.
    pub fn row_count(&self) -> u32 {
        (lib().datagrid_get_row_count)(self.ctrl.id)
    }

    /// Set selection mode: SELECTION_SINGLE (0) or SELECTION_MULTI (1).
    pub fn set_selection_mode(&self, mode: u32) {
        (lib().datagrid_set_selection_mode)(self.ctrl.id, mode);
    }

    /// Get the currently selected row index (single selection).
    /// Returns u32::MAX if nothing is selected.
    pub fn selected_row(&self) -> u32 {
        (lib().datagrid_get_selected_row)(self.ctrl.id)
    }

    /// Set the selected row (single selection mode).
    pub fn set_selected_row(&self, row: u32) {
        (lib().datagrid_set_selected_row)(self.ctrl.id, row);
    }

    /// Check if a row is selected (multi-selection mode).
    pub fn is_row_selected(&self, row: u32) -> bool {
        (lib().datagrid_is_row_selected)(self.ctrl.id, row) != 0
    }

    /// Sort by a column. Direction: SORT_NONE, SORT_ASCENDING, SORT_DESCENDING.
    pub fn sort(&self, column: u32, direction: u32) {
        (lib().datagrid_sort)(self.ctrl.id, column, direction);
    }

    /// Set the row height in pixels (minimum 16).
    pub fn set_row_height(&self, height: u32) {
        (lib().datagrid_set_row_height)(self.ctrl.id, height);
    }

    /// Set the header height in pixels (minimum 16).
    pub fn set_header_height(&self, height: u32) {
        (lib().datagrid_set_header_height)(self.ctrl.id, height);
    }

    /// Register a callback for when the selection changes.
    pub fn on_selection_changed(&self, mut f: impl FnMut(&SelectionChangedEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| {
            let index = Control::from_id(id).get_state();
            f(&SelectionChangedEvent { id, index });
        });
        (lib().on_change_fn)(self.ctrl.id, thunk, ud);
    }

    /// Register a callback for submit (Enter key or double-click on a row).
    pub fn on_submit(&self, mut f: impl FnMut(&SelectionChangedEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| {
            let index = Control::from_id(id).get_state();
            f(&SelectionChangedEvent { id, index });
        });
        (lib().on_submit_fn)(self.ctrl.id, thunk, ud);
    }

    /// Set per-character text colors for cells.
    /// `char_colors`: flat array of ARGB colors (one per character, 0 = use cell default).
    /// `offsets`: one entry per cell — start index into `char_colors` (u32::MAX = no per-char colors).
    pub fn set_char_colors(&self, char_colors: &[u32], offsets: &[u32]) {
        (lib().datagrid_set_char_colors)(
            self.ctrl.id,
            char_colors.as_ptr(),
            char_colors.len() as u32,
            offsets.as_ptr(),
            offsets.len() as u32,
        );
    }

    /// Set an icon (ARGB pixels) for a specific cell.
    pub fn set_cell_icon(&self, row: u32, col: u32, pixels: &[u32], w: u32, h: u32) {
        (lib().datagrid_set_cell_icon)(self.ctrl.id, row, col, pixels.as_ptr(), w, h);
    }

    /// Set all cell data from a pre-encoded byte buffer.
    /// Rows separated by 0x1E (record separator), columns by 0x1F (unit separator).
    pub fn set_data_raw(&self, data: &[u8]) {
        (lib().datagrid_set_data)(self.ctrl.id, data.as_ptr(), data.len() as u32);
    }

    /// Get the current scroll offset (first visible row).
    pub fn scroll_offset(&self) -> u32 {
        (lib().datagrid_get_scroll_offset)(self.ctrl.id)
    }

    /// Set the scroll offset (first visible row).
    pub fn set_scroll_offset(&self, offset: u32) {
        (lib().datagrid_set_scroll_offset)(self.ctrl.id, offset);
    }
}

fn write_u32_ascii(buf: &mut Vec<u8>, val: u32) {
    if val == 0 {
        buf.push(b'0');
        return;
    }
    let start = buf.len();
    let mut v = val;
    while v > 0 {
        buf.push(b'0' + (v % 10) as u8);
        v /= 10;
    }
    buf[start..].reverse();
}
