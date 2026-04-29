//! DataGrid — full-featured data grid with sorting, resizing, reordering.

use crate::control::{Control, ControlBase, ControlKind, EventResponse};
use alloc::vec::Vec;

/// Scrollbar track width in logical pixels.
const SCROLLBAR_W: u32 = 10;
/// Padding around scrollbar edges.
const SCROLLBAR_PAD: i32 = 2;
/// Minimum thumb height in pixels.
const MIN_THUMB: i32 = 20;
/// Corner radius for the rounded scrollbar thumb.
const THUMB_RADIUS: u32 = 4;

/// Text alignment within a cell.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CellAlign {
    Left = 0,
    Center = 1,
    Right = 2,
}

impl CellAlign {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Center,
            2 => Self::Right,
            _ => Self::Left,
        }
    }
}

/// Per-cell icon (ARGB pixel data).
pub struct CellIcon {
    pub pixels: Vec<u32>,
    pub width: u16,
    pub height: u16,
}

/// Sort direction for a column.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    None,
    Ascending,
    Descending,
}

/// How a column's data should be compared when sorting.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SortType {
    /// Lexicographic byte comparison (default).
    String = 0,
    /// Numeric comparison — parses leading digits, falls back to lexicographic.
    Numeric = 1,
}

impl SortType {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Numeric,
            _ => Self::String,
        }
    }
}

/// A single column definition.
#[derive(Clone)]
pub struct Column {
    pub header: Vec<u8>,
    pub width: u32,
    pub min_width: u32,
    pub align: CellAlign,
    pub sort_type: SortType,
}

/// Row selection mode.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SelectionMode {
    Single,
    Multi,
}

/// Drag interaction state machine.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DragMode {
    None,
    Resizing {
        col_index: usize,
        drag_start_x: i32,
        original_width: u32,
    },
    Reordering {
        col_index: usize,
        drag_start_x: i32,
        current_x: i32,
    },
}

/// Connector line between rows (drawn in a specific column).
pub struct ConnectorLine {
    pub start_row: usize,
    pub end_row: usize,
    pub color: u32,
    pub filled: bool,
}

pub struct DataGrid {
    pub(crate) base: ControlBase,
    columns: Vec<Column>,
    display_order: Vec<usize>,
    cell_data: Vec<Vec<u8>>,
    cell_colors: Vec<u32>,
    cell_bg_colors: Vec<u32>,
    /// Per-character text colors. Flat array of u32 ARGB values.
    char_colors: Vec<u32>,
    /// Per-cell offset into `char_colors`. One entry per cell.
    /// `u32::MAX` means no per-char colors (use cell default).
    char_color_offsets: Vec<u32>,
    cell_icons: Vec<Option<CellIcon>>,
    pub(crate) row_count: usize,
    sort_column: Option<usize>,
    sort_direction: SortDirection,
    sorted_rows: Vec<usize>,
    pub(crate) scroll_y: i32,
    scroll_x: i32,
    selection_mode: SelectionMode,
    selected_rows: Vec<u8>,
    anchor_row: Option<usize>,
    drag_mode: DragMode,
    hovered_row: Option<usize>,
    pub(crate) header_height: u32,
    pub(crate) row_height: u32,
    pub(crate) font_size: u16,
    /// Per-row minimap colors (one u32 per row, 0 = no marker). Shown in scrollbar.
    minimap_colors: Vec<u32>,
    /// Last clicked column (display index), set by handle_click.
    pub(crate) last_click_col: i32,
    /// Connector lines drawn over a column (visual only).
    connector_lines: Vec<ConnectorLine>,
    /// Column index (display) in which connector lines are drawn.
    connector_column: usize,
    /// Per-row left indent in pixels. Applied to the column specified by `indent_column`.
    /// Used for tree-like indentation in process lists etc.
    row_indents: Vec<u16>,
    /// Logical column index to which row_indents are applied (default: 0).
    indent_column: usize,
    /// True while the user is dragging the scrollbar thumb.
    dragging_scrollbar: bool,
    dragging_h_scrollbar: bool,
    /// Mouse-Y offset from thumb top when scrollbar drag started.
    scrollbar_drag_anchor: i32,
    /// Bitmask of logical columns that can be edited in-place.
    editable_columns: u32,
    editing_row: Option<usize>,
    editing_col: Option<usize>,
    edit_buffer: Vec<u8>,
    edit_cursor: usize,
    /// Per data-row editor kind used by property-grid style value columns.
    /// 0=text, 1=int, 2=bool, 3=color, 4=enum.
    row_editor_kinds: Vec<u8>,
    /// Per data-row pipe-separated option list for enum-like editors.
    row_editor_options: Vec<Vec<u8>>,
}

impl DataGrid {
    pub fn new(base: ControlBase) -> Self {
        Self {
            base,
            columns: Vec::new(),
            display_order: Vec::new(),
            cell_data: Vec::new(),
            cell_colors: Vec::new(),
            cell_bg_colors: Vec::new(),
            char_colors: Vec::new(),
            char_color_offsets: Vec::new(),
            cell_icons: Vec::new(),
            row_count: 0,
            sort_column: None,
            sort_direction: SortDirection::None,
            sorted_rows: Vec::new(),
            scroll_y: 0,
            scroll_x: 0,
            selection_mode: SelectionMode::Single,
            selected_rows: Vec::new(),
            anchor_row: None,
            drag_mode: DragMode::None,
            hovered_row: None,
            header_height: 32,
            row_height: 28,
            font_size: 0,
            minimap_colors: Vec::new(),
            last_click_col: -1,
            connector_lines: Vec::new(),
            connector_column: 2,
            row_indents: Vec::new(),
            indent_column: 0,
            dragging_scrollbar: false,
            dragging_h_scrollbar: false,
            scrollbar_drag_anchor: 0,
            editable_columns: 0,
            editing_row: None,
            editing_col: None,
            edit_buffer: Vec::new(),
            edit_cursor: 0,
            row_editor_kinds: Vec::new(),
            row_editor_options: Vec::new(),
        }
    }

    // ── Column API ─────────────────────────────────────────────────

    pub fn set_columns_from_data(&mut self, data: &[u8]) {
        self.columns.clear();
        self.display_order.clear();
        // Format: header\x1Fwidth\x1Falign[\x1Fsort_type]\x1E...
        for (i, col_data) in data.split(|&b| b == 0x1E).enumerate() {
            let parts: Vec<&[u8]> = col_data.split(|&b| b == 0x1F).collect();
            let header = parts.first().copied().unwrap_or(&[]);
            let width = parts.get(1).and_then(|s| parse_u32(s)).unwrap_or(100);
            let align = parts
                .get(2)
                .and_then(|s| s.first().map(|&b| CellAlign::from_u8(b.wrapping_sub(b'0'))))
                .unwrap_or(CellAlign::Left);
            let sort_type = parts
                .get(3)
                .and_then(|s| s.first().map(|&b| SortType::from_u8(b.wrapping_sub(b'0'))))
                .unwrap_or(SortType::String);
            self.columns.push(Column {
                header: header.to_vec(),
                width,
                min_width: 30,
                align,
                sort_type,
            });
            self.display_order.push(i);
        }
        self.base.mark_dirty();
    }

    pub fn column_count(&self) -> usize {
        self.columns.len()
    }

    pub fn set_column_width(&mut self, col_index: usize, width: u32) {
        if col_index < self.columns.len() {
            self.columns[col_index].width = width.max(self.columns[col_index].min_width);
            self.base.mark_dirty();
        }
    }

    /// Set the sort comparison type for a column.
    pub fn set_column_sort_type(&mut self, col_index: usize, sort_type: SortType) {
        if col_index < self.columns.len() {
            self.columns[col_index].sort_type = sort_type;
        }
    }

    // ── Cell data API ──────────────────────────────────────────────

    pub fn set_data_from_encoded(&mut self, data: &[u8]) {
        self.cell_data.clear();
        self.row_count = 0;
        let col_count = self.columns.len().max(1);
        for row_data in data.split(|&b| b == 0x1E) {
            let cells: Vec<&[u8]> = row_data.split(|&b| b == 0x1F).collect();
            for (ci, cell) in cells.iter().enumerate() {
                if ci >= col_count {
                    break;
                }
                self.cell_data.push(cell.to_vec());
            }
            // Pad with empty cells if row has fewer columns
            for _ in cells.len()..col_count {
                self.cell_data.push(Vec::new());
            }
            self.row_count += 1;
        }
        self.clamp_scroll();
        self.ensure_selection_bits();
        self.rebuild_sort();
        self.base.mark_dirty();
    }

    pub fn set_row_count(&mut self, count: usize) {
        let col_count = self.columns.len().max(1);
        if count > self.row_count {
            for _ in self.row_count * col_count..count * col_count {
                self.cell_data.push(Vec::new());
            }
        } else if count < self.row_count {
            self.cell_data.truncate(count * col_count);
        }
        self.row_count = count;
        self.clamp_scroll();
        self.ensure_selection_bits();
        self.rebuild_sort();
        self.base.mark_dirty();
    }

    pub fn set_cell(&mut self, row: usize, col: usize, text: &[u8]) {
        let col_count = self.columns.len().max(1);
        let idx = row * col_count + col;
        if idx < self.cell_data.len() {
            if self.cell_data[idx].as_slice() != text {
                self.cell_data[idx].clear();
                self.cell_data[idx].extend_from_slice(text);
                self.base.mark_dirty();
            }
        }
    }

    pub fn get_cell(&self, row: usize, col: usize) -> &[u8] {
        let col_count = self.columns.len().max(1);
        let idx = row * col_count + col;
        self.cell_data.get(idx).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn set_editable_columns(&mut self, mask: u32) {
        self.editable_columns = mask;
        self.cancel_edit();
        self.base.mark_dirty();
    }

    pub fn set_row_editor_kinds_from_encoded(&mut self, data: &[u8]) {
        self.row_editor_kinds.clear();
        for row_data in data.split(|&b| b == 0x1E) {
            let kind = row_data
                .first()
                .copied()
                .map(|b| b.saturating_sub(b'0'))
                .unwrap_or(0)
                .min(4);
            self.row_editor_kinds.push(kind);
        }
        self.base.mark_dirty();
    }

    pub fn set_row_editor_options_from_encoded(&mut self, data: &[u8]) {
        self.row_editor_options.clear();
        for row_data in data.split(|&b| b == 0x1E) {
            self.row_editor_options.push(row_data.to_vec());
        }
        self.base.mark_dirty();
    }

    pub fn set_cell_colors(&mut self, colors: &[u32]) {
        if self.cell_colors.as_slice() != colors {
            self.cell_colors = colors.to_vec();
            self.base.mark_dirty();
        }
    }

    pub fn set_cell_bg_colors(&mut self, colors: &[u32]) {
        if self.cell_bg_colors.as_slice() != colors {
            self.cell_bg_colors = colors.to_vec();
            self.base.mark_dirty();
        }
    }

    /// Set per-character text colors for cells.
    /// `colors`: flat array of u32 ARGB values (one per character).
    /// `offsets`: one entry per cell — index into `colors` where that cell's
    ///   per-char colors begin. Use `u32::MAX` for cells without per-char colors.
    pub fn set_char_colors(&mut self, colors: &[u32], offsets: &[u32]) {
        self.char_colors = colors.to_vec();
        self.char_color_offsets = offsets.to_vec();
        self.base.mark_dirty();
    }

    /// Set an icon (ARGB pixels) for a specific cell. The icon is drawn before the text.
    pub fn set_cell_icon(&mut self, row: usize, col: usize, pixels: &[u32], w: u16, h: u16) {
        let col_count = self.columns.len().max(1);
        let idx = row * col_count + col;
        // Extend the icons vec if needed
        if idx >= self.cell_icons.len() {
            self.cell_icons.resize_with(idx + 1, || None);
        }
        self.cell_icons[idx] = Some(CellIcon {
            pixels: pixels.to_vec(),
            width: w,
            height: h,
        });
        self.base.mark_dirty();
    }

    /// Set per-row minimap colors (shown in the scrollbar track).
    /// One color per row; 0 means no marker.
    pub fn set_minimap_colors(&mut self, colors: &[u32]) {
        self.minimap_colors = colors.to_vec();
        self.base.mark_dirty();
    }

    /// Set per-row left indent in pixels. Applied to the column set by `set_indent_column`.
    /// The indent shifts icon + text to the right by the given amount.
    /// `indents` has one entry per row (u16 pixel value, 0 = no indent).
    pub fn set_row_indents(&mut self, indents: &[u16]) {
        self.row_indents = indents.to_vec();
        self.base.mark_dirty();
    }

    /// Set which logical column receives the per-row indent (default: 0).
    pub fn set_indent_column(&mut self, col: usize) {
        self.indent_column = col;
    }

    /// Get the display column index of the last click (-1 if none).
    pub fn last_click_col(&self) -> i32 {
        self.last_click_col
    }

    fn is_col_editable(&self, logical_col: usize) -> bool {
        logical_col < 32 && (self.editable_columns & (1u32 << logical_col)) != 0
    }

    fn row_editor_kind(&self, data_row: usize) -> u8 {
        self.row_editor_kinds.get(data_row).copied().unwrap_or(0)
    }

    fn typed_editor_active(&self, data_row: usize, logical_col: usize) -> bool {
        logical_col == 1 && self.row_editor_kind(data_row) != 0
    }

    fn toggle_bool_cell(&mut self, row: usize, col: usize) -> bool {
        let current = self.get_cell(row, col);
        let next = if ascii_eq_ignore_case(current, b"true") || current == b"1" {
            b"false".as_slice()
        } else {
            b"true".as_slice()
        };
        self.set_cell(row, col, next);
        self.clear_selection();
        self.set_row_selected(row, true);
        self.base.state = row as u32;
        self.last_click_col = col as i32;
        self.base.mark_dirty();
        true
    }

    fn cycle_enum_cell(&mut self, row: usize, col: usize) -> bool {
        let Some(options) = self.row_editor_options.get(row) else {
            return false;
        };
        if options.is_empty() {
            return false;
        }
        let current = self.get_cell(row, col);
        let mut first: Option<&[u8]> = None;
        let mut next: Option<&[u8]> = None;
        let mut found_current = false;
        for option in options.split(|&b| b == b'|') {
            if option.is_empty() {
                continue;
            }
            if first.is_none() {
                first = Some(option);
            }
            if found_current {
                next = Some(option);
                break;
            }
            if option == current {
                found_current = true;
            }
        }
        let value = next.or(first).unwrap_or(current).to_vec();
        self.set_cell(row, col, &value);
        self.clear_selection();
        self.set_row_selected(row, true);
        self.base.state = row as u32;
        self.last_click_col = col as i32;
        self.base.mark_dirty();
        true
    }

    fn start_edit(&mut self, data_row: usize, display_col: usize) -> bool {
        if data_row >= self.row_count || display_col >= self.display_order.len() {
            return false;
        }
        let logical_col = self.display_order[display_col];
        if !self.is_col_editable(logical_col) {
            return false;
        }
        if self.typed_editor_active(data_row, logical_col)
            && matches!(self.row_editor_kind(data_row), 2 | 4)
        {
            return match self.row_editor_kind(data_row) {
                2 => self.toggle_bool_cell(data_row, logical_col),
                4 => self.cycle_enum_cell(data_row, logical_col),
                _ => false,
            };
        }
        self.edit_buffer = self.get_cell(data_row, logical_col).to_vec();
        self.edit_cursor = self.edit_buffer.len();
        self.editing_row = Some(data_row);
        self.editing_col = Some(logical_col);
        self.clear_selection();
        self.set_row_selected(data_row, true);
        self.base.state = data_row as u32;
        self.last_click_col = display_col as i32;
        self.base.mark_dirty();
        true
    }

    fn commit_edit(&mut self) -> bool {
        let Some(row) = self.editing_row else {
            return false;
        };
        let Some(col) = self.editing_col else {
            return false;
        };
        let value = self.edit_buffer.clone();
        self.set_cell(row, col, &value);
        self.editing_row = None;
        self.editing_col = None;
        self.edit_buffer.clear();
        self.edit_cursor = 0;
        self.base.state = row as u32;
        self.base.mark_dirty();
        true
    }

    fn cancel_edit(&mut self) {
        self.editing_row = None;
        self.editing_col = None;
        self.edit_buffer.clear();
        self.edit_cursor = 0;
    }

    /// Set connector lines (drawn over a column, typically the separator).
    pub fn set_connector_lines(&mut self, lines: Vec<ConnectorLine>) {
        self.connector_lines = lines;
        self.base.mark_dirty();
    }

    /// Set which display column connector lines are drawn in.
    pub fn set_connector_column(&mut self, col: usize) {
        self.connector_column = col;
        self.base.mark_dirty();
    }

    /// Get the first selected row index, or None.
    pub fn selected_row(&self) -> Option<usize> {
        for r in 0..self.row_count {
            if self.is_row_selected(r) {
                return Some(r);
            }
        }
        None
    }

    /// Clamp scroll offsets so the viewport doesn't extend past the content.
    fn clamp_scroll(&mut self) {
        let content_h = self.row_count as i32 * self.row_height as i32;
        let viewport_h = (self.base.h as i32).saturating_sub(self.header_height as i32);
        let max_scroll = (content_h - viewport_h).max(0);
        if self.scroll_y > max_scroll {
            self.scroll_y = max_scroll;
        }
        let max_scroll_x = (self.total_content_width() as i32 - self.base.w as i32).max(0);
        if self.scroll_x > max_scroll_x {
            self.scroll_x = max_scroll_x;
        }
        self.scroll_x = self.scroll_x.max(0);
        self.scroll_y = self.scroll_y.max(0);
    }

    // ── Scrollbar helpers ─────────────────────────────────────────

    fn total_content_width(&self) -> u32 {
        self.display_order
            .iter()
            .filter_map(|&logical| self.columns.get(logical).map(|c| c.width))
            .sum()
    }

    /// Returns (track_h, thumb_h, max_scroll) if the scrollbar is visible.
    /// All values are in logical pixels.
    fn scrollbar_metrics(&self) -> Option<(i32, i32, i32)> {
        let content_h = self.row_count as u32 * self.row_height;
        let view_h = self.base.h.saturating_sub(self.header_height);
        if content_h <= view_h || view_h <= 4 {
            return None;
        }
        let track_h = (view_h - 4) as i32;
        let thumb_h =
            ((view_h as u64 * track_h as u64) / content_h as u64).max(MIN_THUMB as u64) as i32;
        let max_scroll = (content_h - view_h) as i32;
        Some((track_h, thumb_h, max_scroll))
    }

    /// Y position of thumb top, relative to the header bottom.
    fn scrollbar_thumb_y(&self, track_h: i32, thumb_h: i32, max_scroll: i32) -> i32 {
        let frac = if max_scroll > 0 {
            (self.scroll_y as i64 * (track_h - thumb_h) as i64 / max_scroll as i64) as i32
        } else {
            0
        };
        SCROLLBAR_PAD + frac.max(0).min(track_h - thumb_h)
    }

    /// Set scroll_y from a thumb-top position.
    fn set_scroll_from_thumb(
        &mut self,
        thumb_top: i32,
        track_h: i32,
        thumb_h: i32,
        max_scroll: i32,
    ) {
        let clamped = thumb_top.max(0).min(track_h - thumb_h);
        let new_scroll = if track_h > thumb_h {
            (clamped as i64 * max_scroll as i64 / (track_h - thumb_h) as i64) as i32
        } else {
            0
        };
        self.scroll_y = new_scroll.max(0).min(max_scroll);
    }

    /// Returns (track_w, thumb_w, max_scroll) if the horizontal scrollbar is visible.
    /// All values are in logical pixels.
    fn h_scrollbar_metrics(&self) -> Option<(i32, i32, i32)> {
        let content_w = self.total_content_width();
        let view_w = self.base.w;
        if content_w <= view_w || view_w <= 4 {
            return None;
        }
        let track_w = (view_w - 4) as i32;
        let thumb_w =
            ((view_w as u64 * track_w as u64) / content_w as u64).max(MIN_THUMB as u64) as i32;
        let max_scroll = (content_w - view_w) as i32;
        Some((track_w, thumb_w, max_scroll))
    }

    fn h_scrollbar_thumb_x(&self, track_w: i32, thumb_w: i32, max_scroll: i32) -> i32 {
        let frac = if max_scroll > 0 {
            (self.scroll_x as i64 * (track_w - thumb_w) as i64 / max_scroll as i64) as i32
        } else {
            0
        };
        SCROLLBAR_PAD + frac.max(0).min(track_w - thumb_w)
    }

    fn set_scroll_x_from_thumb(
        &mut self,
        thumb_left: i32,
        track_w: i32,
        thumb_w: i32,
        max_scroll: i32,
    ) {
        let clamped = thumb_left.max(0).min(track_w - thumb_w);
        let new_scroll = if track_w > thumb_w {
            (clamped as i64 * max_scroll as i64 / (track_w - thumb_w) as i64) as i32
        } else {
            0
        };
        self.scroll_x = new_scroll.max(0).min(max_scroll);
    }

    // ── Selection ──────────────────────────────────────────────────

    pub fn set_selection_mode(&mut self, mode: SelectionMode) {
        self.selection_mode = mode;
    }

    fn ensure_selection_bits(&mut self) {
        let bytes_needed = (self.row_count + 7) / 8;
        self.selected_rows.resize(bytes_needed, 0);
    }

    pub fn is_row_selected(&self, row: usize) -> bool {
        if row >= self.row_count {
            return false;
        }
        let byte = row / 8;
        let bit = row % 8;
        byte < self.selected_rows.len() && (self.selected_rows[byte] & (1 << bit)) != 0
    }

    pub(crate) fn set_row_selected(&mut self, row: usize, selected: bool) {
        if row >= self.row_count {
            return;
        }
        self.ensure_selection_bits();
        let byte = row / 8;
        let bit = row % 8;
        if selected {
            self.selected_rows[byte] |= 1 << bit;
        } else {
            self.selected_rows[byte] &= !(1 << bit);
        }
    }

    pub(crate) fn clear_selection(&mut self) {
        self.selected_rows.fill(0);
    }

    // ── Sort ───────────────────────────────────────────────────────

    pub fn sort_by(&mut self, column: usize, direction: SortDirection) {
        self.sort_column = if direction == SortDirection::None {
            None
        } else {
            Some(column)
        };
        self.sort_direction = direction;
        self.rebuild_sort();
        self.base.mark_dirty();
    }

    fn rebuild_sort(&mut self) {
        if self.sort_direction == SortDirection::None || self.sort_column.is_none() {
            self.sorted_rows.clear();
            return;
        }
        let col_count = self.columns.len().max(1);
        let logical_col = match self.sort_column {
            Some(dc) if dc < self.display_order.len() => self.display_order[dc],
            _ => {
                self.sorted_rows.clear();
                return;
            }
        };
        let numeric = logical_col < self.columns.len()
            && self.columns[logical_col].sort_type == SortType::Numeric;
        self.sorted_rows = (0..self.row_count).collect();
        let ascending = self.sort_direction == SortDirection::Ascending;
        let data = &self.cell_data;
        self.sorted_rows.sort_by(|&a, &b| {
            let a_idx = a * col_count + logical_col;
            let b_idx = b * col_count + logical_col;
            let a_text = data.get(a_idx).map(|v| v.as_slice()).unwrap_or(&[]);
            let b_text = data.get(b_idx).map(|v| v.as_slice()).unwrap_or(&[]);
            let ord = if numeric {
                parse_sort_key(a_text).cmp(&parse_sort_key(b_text))
            } else {
                a_text.cmp(b_text)
            };
            if ascending {
                ord
            } else {
                ord.reverse()
            }
        });
    }

    // ── Hit-test helpers ───────────────────────────────────────────

    fn column_at_x(&self, lx: i32) -> Option<usize> {
        let mut col_x = -self.scroll_x;
        for (i, &logical) in self.display_order.iter().enumerate() {
            let w = self.columns[logical].width as i32;
            if lx >= col_x && lx < col_x + w {
                return Some(i);
            }
            col_x += w;
        }
        None
    }

    fn column_edge_at_x(&self, lx: i32) -> Option<(usize, i32)> {
        let mut col_x = -self.scroll_x;
        for (i, &logical) in self.display_order.iter().enumerate() {
            col_x += self.columns[logical].width as i32;
            if (lx - col_x).abs() <= 4 {
                return Some((i, col_x));
            }
        }
        None
    }

    fn row_at_y(&self, ly: i32) -> Option<usize> {
        if ly < self.header_height as i32 {
            return None;
        }
        let data_y = ly - self.header_height as i32 + self.scroll_y;
        let row = data_y / self.row_height as i32;
        if row >= 0 && (row as usize) < self.row_count {
            Some(row as usize)
        } else {
            None
        }
    }

    fn data_row(&self, vis_row: usize) -> usize {
        if self.sorted_rows.is_empty() {
            vis_row
        } else {
            self.sorted_rows[vis_row]
        }
    }

    /// Find the visual row index of the currently selected data row.
    fn selected_visual_row(&self) -> Option<usize> {
        let data_row = self.selected_row()?;
        if self.sorted_rows.is_empty() {
            Some(data_row)
        } else {
            self.sorted_rows.iter().position(|&r| r == data_row)
        }
    }

    /// Select a visual row (handles sort mapping, clears old selection, scrolls into view).
    fn select_visual_row(&mut self, vis_row: usize) {
        let data_row = self.data_row(vis_row);
        self.clear_selection();
        self.set_row_selected(data_row, true);
        self.base.state = data_row as u32;
        self.scroll_to_row(vis_row);
        self.base.mark_dirty();
    }

    /// Scroll to ensure a visual row is visible.
    pub fn scroll_to_row(&mut self, vis_row: usize) {
        let rh = self.row_height as i32;
        let row_top = vis_row as i32 * rh;
        let row_bottom = row_top + rh;
        let viewport_h = self.base.h as i32 - self.header_height as i32;
        if row_top < self.scroll_y {
            self.scroll_y = row_top;
        } else if row_bottom > self.scroll_y + viewport_h {
            self.scroll_y = row_bottom - viewport_h;
        }
    }
}

impl Control for DataGrid {
    fn base(&self) -> &ControlBase {
        &self.base
    }
    fn base_mut(&mut self) -> &mut ControlBase {
        &mut self.base
    }
    fn kind(&self) -> ControlKind {
        ControlKind::DataGrid
    }

    fn set_font_size(&mut self, size: u16) {
        self.font_size = size;
    }
    fn get_font_size(&self) -> u16 {
        if self.font_size > 0 {
            self.font_size
        } else {
            13
        }
    }

    fn scrollbar_hit_x(&self) -> Option<i32> {
        if self.scrollbar_metrics().is_some() {
            Some(self.base.w as i32 - SCROLLBAR_W as i32 - SCROLLBAR_PAD - 2)
        } else {
            None
        }
    }

    fn scrollbar_hit_y(&self) -> Option<i32> {
        if self.h_scrollbar_metrics().is_some() {
            Some(self.base.h as i32 - SCROLLBAR_W as i32 - SCROLLBAR_PAD - 2)
        } else {
            None
        }
    }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let b = self.base();
        let ctx = crate::control::prepare_render(b, ax, ay);
        let (x, y, w, h) = (ctx.x, ctx.y, ctx.w, ctx.h);
        let tc = crate::theme::colors();

        // Scaled dimensions
        let hdr_h = crate::theme::scale(self.header_height);
        let rh_s = crate::theme::scale(self.row_height) as i32;
        let scroll_x_s = crate::theme::scale_i32(self.scroll_x);
        let scroll_y_s = crate::theme::scale_i32(self.scroll_y);
        let cell_pad = crate::theme::scale_i32(8);
        let icon_pad = crate::theme::scale_i32(4);
        let logical_fs = if self.font_size > 0 {
            self.font_size
        } else {
            13
        };
        let fs = crate::draw::scale_font(logical_fs);

        // Clip to control bounds (physical)
        let clipped = surface.with_clip(x, y, w, h);

        // Background
        crate::draw::fill_rect(&clipped, x, y, w, h, tc.card_bg);

        if self.columns.is_empty() {
            return;
        }

        let col_count = self.columns.len();

        // ── Data rows (scrolled) ──
        let viewport_h = h.saturating_sub(hdr_h) as i32;
        if viewport_h > 0 && self.row_count > 0 {
            let vis_start = (scroll_y_s / rh_s).max(0) as usize;
            let vis_end =
                ((scroll_y_s + viewport_h) / rh_s + 2).min(self.row_count as i32) as usize;

            for vis_row in vis_start..vis_end {
                let data_row = self.data_row(vis_row);
                let row_y = y + hdr_h as i32 + (vis_row as i32) * rh_s - scroll_y_s;
                let rh_u = rh_s as u32;

                // Row background
                let selected = self.is_row_selected(data_row);
                if selected {
                    crate::draw::fill_rect(&clipped, x, row_y, w, rh_u, tc.selection);
                } else if Some(vis_row) == self.hovered_row {
                    crate::draw::fill_rect(&clipped, x, row_y, w, rh_u, tc.control_hover);
                } else if vis_row % 2 == 1 {
                    crate::draw::fill_rect(&clipped, x, row_y, w, rh_u, tc.alt_row_bg);
                }

                // Cell text + icons
                let mut col_x = x - scroll_x_s;
                for disp_col in 0..col_count {
                    let logical_col = self.display_order[disp_col];
                    let col = &self.columns[logical_col];
                    let col_w_s = crate::theme::scale(col.width);
                    let cell_idx = data_row * col_count + logical_col;

                    let cell_clip = clipped.with_clip(col_x, row_y, col_w_s, rh_u);

                    // Draw per-cell background color (if set)
                    if cell_idx < self.cell_bg_colors.len() && self.cell_bg_colors[cell_idx] != 0 {
                        crate::draw::fill_rect(
                            &cell_clip,
                            col_x,
                            row_y,
                            col_w_s,
                            rh_u,
                            self.cell_bg_colors[cell_idx],
                        );
                    }

                    // Per-row indent (applied to the configured indent column)
                    let row_indent: i32 =
                        if logical_col == self.indent_column && data_row < self.row_indents.len() {
                            crate::theme::scale_i32(self.row_indents[data_row] as i32)
                        } else {
                            0
                        };

                    // Draw cell icon (if any)
                    let mut icon_offset: i32 = row_indent;
                    if cell_idx < self.cell_icons.len() {
                        if let Some(ref icon) = self.cell_icons[cell_idx] {
                            let iw = icon.width as i32;
                            let ih = icon.height as i32;
                            let ix = col_x + icon_pad + row_indent;
                            let iy = row_y + (rh_s - ih) / 2;
                            crate::draw::blit_argb(
                                &cell_clip,
                                ix,
                                iy,
                                icon.width as u32,
                                icon.height as u32,
                                &icon.pixels,
                            );
                            icon_offset = row_indent + iw + icon_pad;
                        }
                    }

                    let editing_this_cell =
                        self.editing_row == Some(data_row) && self.editing_col == Some(logical_col);
                    let editor_kind = if logical_col == 1 {
                        self.row_editor_kind(data_row)
                    } else {
                        0
                    };
                    let text = if editing_this_cell {
                        self.edit_buffer.as_slice()
                    } else if cell_idx < self.cell_data.len() {
                        self.cell_data[cell_idx].as_slice()
                    } else {
                        &[]
                    };
                    if editing_this_cell {
                        crate::draw::fill_rect(
                            &cell_clip,
                            col_x + 1,
                            row_y + 2,
                            col_w_s.saturating_sub(2),
                            rh_u.saturating_sub(4),
                            tc.control_bg,
                        );
                        crate::draw::draw_border(
                            &cell_clip,
                            col_x + 1,
                            row_y + 2,
                            col_w_s.saturating_sub(2),
                            rh_u.saturating_sub(4),
                            tc.accent,
                        );
                    }

                    if !text.is_empty() || editing_this_cell {
                        let default_color = if cell_idx < self.cell_colors.len()
                            && self.cell_colors[cell_idx] != 0
                        {
                            self.cell_colors[cell_idx]
                        } else if editing_this_cell {
                            tc.text
                        } else if selected {
                            0xFFFFFFFF
                        } else {
                            tc.text
                        };

                        let decorator_offset = if !editing_this_cell {
                            draw_cell_editor_decorator(
                                &cell_clip,
                                col_x,
                                row_y,
                                col_w_s,
                                rh_u,
                                editor_kind,
                                text,
                                tc,
                            )
                        } else {
                            0
                        };
                        let text_x = match col.align {
                            CellAlign::Left => col_x + cell_pad + icon_offset,
                            CellAlign::Center => {
                                let (tw, _) = crate::draw::text_size_at(text, fs);
                                col_x + icon_offset + (col_w_s as i32 - icon_offset - tw as i32) / 2
                            }
                            CellAlign::Right => {
                                let (tw, _) = crate::draw::text_size_at(text, fs);
                                col_x + col_w_s as i32 - cell_pad - decorator_offset - tw as i32
                            }
                        };
                        let text_y = row_y + (rh_s - fs as i32) / 2;

                        // Check for per-character colors
                        let has_char_colors = cell_idx < self.char_color_offsets.len()
                            && self.char_color_offsets[cell_idx] != u32::MAX;

                        if has_char_colors {
                            let base_off = self.char_color_offsets[cell_idx] as usize;
                            let text_len = text.len();
                            let mut cx = text_x;
                            let mut span_start = 0usize;
                            while span_start < text_len {
                                let cc_idx = base_off + span_start;
                                let span_color = if cc_idx < self.char_colors.len()
                                    && self.char_colors[cc_idx] != 0
                                {
                                    self.char_colors[cc_idx]
                                } else {
                                    default_color
                                };
                                let mut span_end = span_start + 1;
                                while span_end < text_len {
                                    let next_idx = base_off + span_end;
                                    let next_color = if next_idx < self.char_colors.len()
                                        && self.char_colors[next_idx] != 0
                                    {
                                        self.char_colors[next_idx]
                                    } else {
                                        default_color
                                    };
                                    if next_color != span_color {
                                        break;
                                    }
                                    span_end += 1;
                                }
                                let span = &text[span_start..span_end];
                                crate::draw::draw_text_sized(
                                    &cell_clip, cx, text_y, span_color, span, fs,
                                );
                                let (sw, _) = crate::draw::text_size_at(span, fs);
                                cx += sw as i32;
                                span_start = span_end;
                            }
                        } else {
                            crate::draw::draw_text_sized(
                                &cell_clip,
                                text_x,
                                text_y,
                                default_color,
                                text,
                                fs,
                            );
                        }

                        if editing_this_cell {
                            let cursor_prefix = &text[..self.edit_cursor.min(text.len())];
                            let (prefix_w, _) = crate::draw::text_size_at(cursor_prefix, fs);
                            let caret_x = text_x + prefix_w as i32 + 1;
                            crate::draw::fill_rect(
                                &cell_clip,
                                caret_x,
                                row_y + 5,
                                1,
                                rh_u.saturating_sub(10),
                                tc.accent,
                            );
                        }
                    }

                    col_x += col_w_s as i32;
                }

                // Row separator
                crate::draw::fill_rect(&clipped, x, row_y + rh_s - 1, w, 1, tc.separator);
            }
        }

        // ── Header (drawn over data, doesn't scroll vertically) ──
        crate::draw::fill_rect(&clipped, x, y, w, hdr_h, tc.control_bg);

        let hdr_fs = crate::draw::scale_font(13);
        let mut col_x = x - scroll_x_s;
        for disp_col in 0..col_count {
            let logical_col = self.display_order[disp_col];
            let col = &self.columns[logical_col];
            let col_w_s = crate::theme::scale(col.width);

            // Header text (clipped to column bounds)
            let text_y = y + (hdr_h as i32 - hdr_fs as i32) / 2;
            let hdr_clip = clipped.with_clip(col_x, y, col_w_s, hdr_h);
            crate::draw::draw_text_sized(
                &hdr_clip,
                col_x + cell_pad,
                text_y,
                tc.text,
                &col.header,
                hdr_fs,
            );

            // Sort indicator
            if self.sort_column == Some(disp_col) && self.sort_direction != SortDirection::None {
                let ix = col_x + col_w_s as i32 - crate::theme::scale_i32(16);
                let iy = y + (hdr_h as i32) / 2;
                if self.sort_direction == SortDirection::Ascending {
                    draw_sort_arrow_up(&clipped, ix, iy, tc.accent);
                } else {
                    draw_sort_arrow_down(&clipped, ix, iy, tc.accent);
                }
            }

            col_x += col_w_s as i32;
            // Column separator line
            let sep_h =
                (hdr_h + self.row_count as u32 * crate::theme::scale(self.row_height)).min(h);
            crate::draw::fill_rect(&clipped, col_x - 1, y, 1, sep_h, tc.separator);
        }

        // Header bottom border
        crate::draw::fill_rect(&clipped, x, y + hdr_h as i32 - 1, w, 1, tc.separator);

        // ── Reorder visual feedback ──
        if let DragMode::Reordering {
            col_index,
            current_x,
            drag_start_x,
        } = self.drag_mode
        {
            if (current_x - drag_start_x).abs() > 5 && col_index < self.display_order.len() {
                let logical = self.display_order[col_index];
                let cw = crate::theme::scale(self.columns[logical].width);
                let cx_s = crate::theme::scale_i32(current_x);
                crate::draw::fill_rect(&clipped, x + cx_s, y, cw, h, 0x40007AFF);
                crate::draw::fill_rect(&clipped, x + cx_s, y, 2, h, tc.accent);
            }
        }

        // ── Vertical scrollbar + minimap ──
        let content_h_s = self.row_count as u32 * crate::theme::scale(self.row_height);
        let view_h_s = h.saturating_sub(hdr_h);
        if content_h_s > view_h_s && view_h_s > 4 {
            let bar_w = crate::theme::scale(SCROLLBAR_W);
            let bar_x = x + w as i32 - bar_w as i32 - crate::theme::scale_i32(SCROLLBAR_PAD);
            let track_y = y + hdr_h as i32 + crate::theme::scale_i32(SCROLLBAR_PAD);
            let track_h = (view_h_s as i32 - crate::theme::scale_i32(SCROLLBAR_PAD * 2)).max(1);
            crate::draw::fill_rect(
                &clipped,
                bar_x,
                track_y,
                bar_w,
                track_h as u32,
                tc.scrollbar_track,
            );

            let has_minimap = !self.minimap_colors.is_empty();
            if has_minimap && self.row_count > 0 && track_h > 0 {
                let total = self.row_count as i32;
                for (row, &color) in self.minimap_colors.iter().enumerate() {
                    if color == 0 || row >= self.row_count {
                        continue;
                    }
                    let py = track_y + (row as i64 * track_h as i64 / total as i64) as i32;
                    let ph = ((track_h as i64 / total as i64).max(1)).min(3) as u32;
                    crate::draw::fill_rect(&clipped, bar_x, py, bar_w, ph, color);
                }
                let vp_y = track_y
                    + (scroll_y_s as i64 * track_h as i64 / (self.row_count as i64 * rh_s as i64))
                        .max(0) as i32;
                let vp_h = (view_h_s as i64 * track_h as i64 / content_h_s as i64).max(4) as u32;
                crate::draw::fill_rect(&clipped, bar_x, vp_y, bar_w, vp_h, 0x30FFFFFF);
            }

            let thumb_h = ((view_h_s as u64 * track_h as u64) / content_h_s as u64)
                .max(MIN_THUMB as u64) as i32;
            let max_scroll_s = (content_h_s as i32 - view_h_s as i32).max(0);
            let scroll_frac = if max_scroll_s > 0 {
                (scroll_y_s as i64 * (track_h - thumb_h) as i64 / max_scroll_s as i64) as i32
            } else {
                0
            };
            let thumb_y = track_y + scroll_frac.max(0).min(track_h - thumb_h);
            let thumb_r = crate::theme::scale(THUMB_RADIUS);
            crate::draw::fill_rounded_rect(
                &clipped,
                bar_x,
                thumb_y,
                bar_w,
                thumb_h as u32,
                thumb_r,
                tc.scrollbar,
            );
        }

        // ── Horizontal scrollbar ──
        let content_w_s = crate::theme::scale(self.total_content_width());
        let view_w_s = w;
        if content_w_s > view_w_s && view_w_s > 4 {
            let bar_h = crate::theme::scale(SCROLLBAR_W);
            let bar_y = y + h as i32 - bar_h as i32 - crate::theme::scale_i32(SCROLLBAR_PAD);
            let track_x = x + crate::theme::scale_i32(SCROLLBAR_PAD);
            let track_w = (view_w_s as i32 - crate::theme::scale_i32(SCROLLBAR_PAD * 2)).max(1);
            crate::draw::fill_rect(
                &clipped,
                track_x,
                bar_y,
                track_w as u32,
                bar_h,
                tc.scrollbar_track,
            );

            let thumb_w = ((view_w_s as u64 * track_w as u64) / content_w_s as u64)
                .max(MIN_THUMB as u64) as i32;
            let max_scroll_s = (content_w_s as i32 - view_w_s as i32).max(0);
            let scroll_frac = if max_scroll_s > 0 {
                (scroll_x_s as i64 * (track_w - thumb_w) as i64 / max_scroll_s as i64) as i32
            } else {
                0
            };
            let thumb_x = track_x + scroll_frac.max(0).min(track_w - thumb_w);
            let thumb_r = crate::theme::scale(THUMB_RADIUS);
            crate::draw::fill_rounded_rect(
                &clipped,
                thumb_x,
                bar_y,
                thumb_w as u32,
                bar_h,
                thumb_r,
                tc.scrollbar,
            );
        }

        // ── Connector lines (drawn over a column) ──
        if !self.connector_lines.is_empty() && self.connector_column < col_count {
            let logical_col = self.display_order[self.connector_column];
            let col_w = crate::theme::scale(self.columns[logical_col].width);
            let mut conn_col_x = x - scroll_x_s;
            for dc in 0..self.connector_column {
                let lc = self.display_order[dc];
                conn_col_x += crate::theme::scale(self.columns[lc].width) as i32;
            }
            let conn_clip = clipped.with_clip(conn_col_x, y + hdr_h as i32, col_w, view_h_s);
            let base_y = y + hdr_h as i32 - scroll_y_s;
            let conn_pad = crate::theme::scale_i32(2);

            for cl in &self.connector_lines {
                let y0 = base_y + cl.start_row as i32 * rh_s;
                let y1 = base_y + cl.end_row as i32 * rh_s + rh_s;
                if cl.filled {
                    let fy = y0.max(y + hdr_h as i32);
                    let fy1 = y1.min(y + h as i32);
                    if fy1 > fy {
                        let fill_color = (cl.color & 0x00FFFFFF) | 0x20000000;
                        crate::draw::fill_rect(
                            &conn_clip,
                            conn_col_x,
                            fy,
                            col_w,
                            (fy1 - fy) as u32,
                            fill_color,
                        );
                    }
                }
                let lx0 = conn_col_x + conn_pad;
                let lx1 = conn_col_x + col_w as i32 - conn_pad;
                let line_w = (lx1 - lx0).max(0) as u32;
                crate::draw::fill_rect(&conn_clip, lx0, y0, line_w, 1, cl.color);
                crate::draw::fill_rect(&conn_clip, lx0, y1 - 1, line_w, 1, cl.color);
                crate::draw::fill_rect(&conn_clip, lx0, y0, 1, (y1 - y0) as u32, cl.color);
                crate::draw::fill_rect(&conn_clip, lx1, y0, 1, (y1 - y0) as u32, cl.color);
            }
        }
    }

    fn is_interactive(&self) -> bool {
        true
    }

    fn handle_mouse_down(&mut self, lx: i32, ly: i32, button: u32) -> EventResponse {
        if let Some(hit_y) = self.scrollbar_hit_y() {
            if ly >= hit_y {
                if let Some((track_w, thumb_w, max_scroll)) = self.h_scrollbar_metrics() {
                    let tx = self.h_scrollbar_thumb_x(track_w, thumb_w, max_scroll);
                    if lx >= tx && lx < tx + thumb_w {
                        self.dragging_h_scrollbar = true;
                        self.scrollbar_drag_anchor = lx - tx;
                    } else {
                        self.dragging_h_scrollbar = true;
                        self.scrollbar_drag_anchor = thumb_w / 2;
                        let new_left = lx - thumb_w / 2 - SCROLLBAR_PAD;
                        self.set_scroll_x_from_thumb(new_left, track_w, thumb_w, max_scroll);
                    }
                    self.base.mark_dirty();
                    return EventResponse::CONSUMED;
                }
            }
        }

        // Check scrollbar area (hit_test already routed scrollbar clicks to us)
        if let Some(hit_x) = self.scrollbar_hit_x() {
            if lx >= hit_x {
                if let Some((track_h, thumb_h, max_scroll)) = self.scrollbar_metrics() {
                    // ly is relative to the DataGrid top; scrollbar starts below header
                    let sb_local_y = ly - self.header_height as i32;
                    let ty = self.scrollbar_thumb_y(track_h, thumb_h, max_scroll);
                    if sb_local_y >= ty && sb_local_y < ty + thumb_h {
                        // Click on thumb — start drag
                        self.dragging_scrollbar = true;
                        self.scrollbar_drag_anchor = sb_local_y - ty;
                    } else {
                        // Click on track — jump so thumb centres on click, then drag
                        self.dragging_scrollbar = true;
                        self.scrollbar_drag_anchor = thumb_h / 2;
                        let new_top = sb_local_y - thumb_h / 2 - SCROLLBAR_PAD;
                        self.set_scroll_from_thumb(new_top, track_h, thumb_h, max_scroll);
                    }
                    self.base.mark_dirty();
                    return EventResponse::CONSUMED;
                }
            }
        }

        if ly < self.header_height as i32 {
            // Check resize handle first (4px near column edge)
            if let Some((col_idx, _edge_x)) = self.column_edge_at_x(lx) {
                let logical = self.display_order[col_idx];
                self.drag_mode = DragMode::Resizing {
                    col_index: col_idx,
                    drag_start_x: lx,
                    original_width: self.columns[logical].width,
                };
                return EventResponse::CONSUMED;
            }
            // Start potential reorder
            if let Some(col_idx) = self.column_at_x(lx) {
                self.drag_mode = DragMode::Reordering {
                    col_index: col_idx,
                    drag_start_x: lx,
                    current_x: lx,
                };
                return EventResponse::CONSUMED;
            }
        }

        // Right-click on a row: select it so context menu targets the right entry
        if button & 0x02 != 0 {
            if let Some(vis_row) = self.row_at_y(ly) {
                let data_row = self.data_row(vis_row);
                if !self.is_row_selected(data_row) {
                    self.clear_selection();
                    self.set_row_selected(data_row, true);
                    self.anchor_row = Some(data_row);
                    self.base.state = data_row as u32;
                    self.base.mark_dirty();
                }
                return EventResponse::CHANGED;
            }
        }

        EventResponse::CONSUMED
    }

    fn handle_mouse_move(&mut self, lx: i32, ly: i32) -> EventResponse {
        if self.dragging_h_scrollbar {
            if let Some((track_w, thumb_w, max_scroll)) = self.h_scrollbar_metrics() {
                let new_left = lx - self.scrollbar_drag_anchor - SCROLLBAR_PAD;
                self.set_scroll_x_from_thumb(new_left, track_w, thumb_w, max_scroll);
                self.base.mark_dirty();
                return EventResponse::CHANGED;
            }
        }
        if self.dragging_scrollbar {
            if let Some((track_h, thumb_h, max_scroll)) = self.scrollbar_metrics() {
                let sb_local_y = ly - self.header_height as i32;
                let new_top = sb_local_y - self.scrollbar_drag_anchor - SCROLLBAR_PAD;
                self.set_scroll_from_thumb(new_top, track_h, thumb_h, max_scroll);
                self.base.mark_dirty();
                return EventResponse::CHANGED;
            }
        }
        match self.drag_mode {
            DragMode::Resizing {
                col_index,
                drag_start_x,
                original_width,
            } => {
                let delta = lx - drag_start_x;
                let logical_col = self.display_order[col_index];
                let min_w = self.columns[logical_col].min_width.max(30);
                let new_width = (original_width as i32 + delta).max(min_w as i32) as u32;
                self.columns[logical_col].width = new_width;
                self.base.mark_dirty();
                EventResponse::CHANGED
            }
            DragMode::Reordering {
                drag_start_x,
                ref mut current_x,
                ..
            } => {
                if (lx - drag_start_x).abs() > 5 {
                    *current_x = lx;
                    self.base.mark_dirty();
                }
                EventResponse::CONSUMED
            }
            DragMode::None => {
                if ly >= self.header_height as i32 {
                    let new_hover = self.row_at_y(ly);
                    if new_hover != self.hovered_row {
                        self.hovered_row = new_hover;
                        self.base.mark_dirty();
                    }
                } else if self.hovered_row.is_some() {
                    self.hovered_row = None;
                    self.base.mark_dirty();
                }
                EventResponse::IGNORED
            }
        }
    }

    fn handle_mouse_up(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        if self.dragging_h_scrollbar {
            self.dragging_h_scrollbar = false;
            return EventResponse::CONSUMED;
        }
        if self.dragging_scrollbar {
            self.dragging_scrollbar = false;
            return EventResponse::CONSUMED;
        }
        let mode = core::mem::replace(&mut self.drag_mode, DragMode::None);
        match mode {
            DragMode::Reordering {
                col_index,
                drag_start_x,
                current_x,
            } => {
                if (current_x - drag_start_x).abs() > 5 {
                    if let Some(target_col) = self.column_at_x(current_x) {
                        if target_col != col_index {
                            let val = self.display_order.remove(col_index);
                            self.display_order.insert(target_col, val);
                        }
                    }
                }
                self.base.mark_dirty();
                EventResponse::CHANGED
            }
            DragMode::Resizing { .. } => {
                self.base.mark_dirty();
                EventResponse::CHANGED
            }
            DragMode::None => EventResponse::CONSUMED,
        }
    }

    fn handle_click(&mut self, lx: i32, ly: i32, _button: u32) -> EventResponse {
        if ly < self.header_height as i32 {
            // Header click -> sort toggle (only if not dragging)
            if let Some(disp_col) = self.column_at_x(lx) {
                if self.sort_column == Some(disp_col) {
                    self.sort_direction = match self.sort_direction {
                        SortDirection::Ascending => SortDirection::Descending,
                        SortDirection::Descending => SortDirection::None,
                        SortDirection::None => SortDirection::Ascending,
                    };
                } else {
                    self.sort_column = Some(disp_col);
                    self.sort_direction = SortDirection::Ascending;
                }
                self.rebuild_sort();
                self.base.mark_dirty();
            }
            EventResponse::CHANGED
        } else {
            // Track clicked column
            self.last_click_col = self.column_at_x(lx).map(|c| c as i32).unwrap_or(-1);

            // Row selection
            if let Some(vis_row) = self.row_at_y(ly) {
                let data_row = self.data_row(vis_row);
                let mods = crate::state().last_modifiers;
                let ctrl = mods & 2 != 0;
                let shift = mods & 1 != 0;

                match self.selection_mode {
                    SelectionMode::Single => {
                        self.clear_selection();
                        self.set_row_selected(data_row, true);
                        self.anchor_row = Some(data_row);
                        self.base.state = data_row as u32;
                    }
                    SelectionMode::Multi => {
                        if ctrl {
                            // Ctrl+Click: toggle individual row
                            let was = self.is_row_selected(data_row);
                            self.set_row_selected(data_row, !was);
                            if !was {
                                self.anchor_row = Some(data_row);
                            }
                        } else if shift {
                            // Shift+Click: range select from anchor
                            let anchor = self.anchor_row.unwrap_or(0);
                            let lo = anchor.min(data_row);
                            let hi = anchor.max(data_row);
                            self.clear_selection();
                            for r in lo..=hi {
                                self.set_row_selected(r, true);
                            }
                        } else {
                            // Plain click: select only this row
                            self.clear_selection();
                            self.set_row_selected(data_row, true);
                            self.anchor_row = Some(data_row);
                        }
                        self.base.state = data_row as u32;
                    }
                }
                self.base.mark_dirty();
            }
            EventResponse::CHANGED
        }
    }

    fn handle_scroll(&mut self, delta: i32) -> EventResponse {
        let mods = crate::state().last_modifiers;
        if mods & 1 != 0 {
            let max_scroll_x = (self.total_content_width() as i32 - self.base.w as i32).max(0);
            let prev = self.scroll_x;
            self.scroll_x = (self.scroll_x - delta * 40).max(0).min(max_scroll_x);
            if self.scroll_x != prev {
                self.base.mark_dirty();
            }
            return EventResponse::CONSUMED;
        }
        let content_h = self.row_count as i32 * self.row_height as i32;
        let viewport_h = self.base.h as i32 - self.header_height as i32;
        let max_scroll = (content_h - viewport_h).max(0);
        let prev = self.scroll_y;
        self.scroll_y = (self.scroll_y - delta * 20).max(0).min(max_scroll);
        if self.scroll_y != prev {
            self.base.mark_dirty();
        }
        EventResponse::CONSUMED
    }

    fn handle_mouse_leave(&mut self) {
        if self.hovered_row.is_some() {
            self.hovered_row = None;
            self.base.mark_dirty();
        }
    }

    fn handle_key_down(&mut self, keycode: u32, _char_code: u32, _modifiers: u32) -> EventResponse {
        use crate::control::*;
        if self.editing_row.is_some() {
            match keycode {
                KEY_ENTER | KEY_TAB => {
                    if self.commit_edit() {
                        return EventResponse::SUBMIT;
                    }
                    return EventResponse::CONSUMED;
                }
                KEY_ESCAPE => {
                    self.cancel_edit();
                    self.base.mark_dirty();
                    return EventResponse::CONSUMED;
                }
                KEY_BACKSPACE => {
                    if self.edit_cursor > 0 {
                        self.edit_cursor -= 1;
                        self.edit_buffer.remove(self.edit_cursor);
                        self.base.mark_dirty();
                        return EventResponse::CHANGED;
                    }
                    return EventResponse::CONSUMED;
                }
                KEY_DELETE => {
                    if self.edit_cursor < self.edit_buffer.len() {
                        self.edit_buffer.remove(self.edit_cursor);
                        self.base.mark_dirty();
                        return EventResponse::CHANGED;
                    }
                    return EventResponse::CONSUMED;
                }
                KEY_LEFT => {
                    if self.edit_cursor > 0 {
                        self.edit_cursor -= 1;
                        self.base.mark_dirty();
                    }
                    return EventResponse::CONSUMED;
                }
                KEY_RIGHT => {
                    if self.edit_cursor < self.edit_buffer.len() {
                        self.edit_cursor += 1;
                        self.base.mark_dirty();
                    }
                    return EventResponse::CONSUMED;
                }
                KEY_HOME => {
                    self.edit_cursor = 0;
                    self.base.mark_dirty();
                    return EventResponse::CONSUMED;
                }
                KEY_END => {
                    self.edit_cursor = self.edit_buffer.len();
                    self.base.mark_dirty();
                    return EventResponse::CONSUMED;
                }
                _ => {}
            }
            if (32..=126).contains(&_char_code) {
                if self
                    .editing_row
                    .map(|row| self.row_editor_kind(row) == 3 && !is_argb_editor_char(_char_code))
                    .unwrap_or(false)
                {
                    return EventResponse::CONSUMED;
                }
                self.edit_buffer.insert(
                    self.edit_cursor.min(self.edit_buffer.len()),
                    _char_code as u8,
                );
                self.edit_cursor = (self.edit_cursor + 1).min(self.edit_buffer.len());
                self.base.mark_dirty();
                return EventResponse::CHANGED;
            }
            return EventResponse::CONSUMED;
        }
        match keycode {
            KEY_ENTER => {
                if self.selected_row().is_some() {
                    let display_col = if self.last_click_col >= 0 {
                        self.last_click_col as usize
                    } else {
                        0
                    };
                    if let Some(row) = self.selected_row() {
                        if self.start_edit(row, display_col) {
                            return if self.typed_editor_active(row, display_col) {
                                EventResponse::SUBMIT
                            } else {
                                EventResponse::CONSUMED
                            };
                        }
                    }
                    return EventResponse::SUBMIT;
                }
                EventResponse::CONSUMED
            }
            KEY_SPACE => {
                if let Some(row) = self.selected_row() {
                    let display_col = if self.last_click_col >= 0 {
                        self.last_click_col as usize
                    } else {
                        0
                    };
                    if display_col < self.display_order.len() {
                        let logical_col = self.display_order[display_col];
                        if logical_col == 1 && self.row_editor_kind(row) == 2 {
                            self.toggle_bool_cell(row, logical_col);
                            return EventResponse::SUBMIT;
                        }
                    }
                }
                EventResponse::IGNORED
            }
            KEY_UP => {
                if self.row_count == 0 {
                    return EventResponse::CONSUMED;
                }
                let vis = self.selected_visual_row().unwrap_or(0);
                let new_vis = if vis > 0 { vis - 1 } else { 0 };
                self.select_visual_row(new_vis);
                EventResponse::CHANGED
            }
            KEY_DOWN => {
                if self.row_count == 0 {
                    return EventResponse::CONSUMED;
                }
                let vis = self.selected_visual_row().unwrap_or(0);
                let new_vis = if vis + 1 < self.row_count {
                    vis + 1
                } else {
                    self.row_count - 1
                };
                self.select_visual_row(new_vis);
                EventResponse::CHANGED
            }
            KEY_HOME => {
                if self.row_count == 0 {
                    return EventResponse::CONSUMED;
                }
                self.select_visual_row(0);
                EventResponse::CHANGED
            }
            KEY_END => {
                if self.row_count == 0 {
                    return EventResponse::CONSUMED;
                }
                self.select_visual_row(self.row_count - 1);
                EventResponse::CHANGED
            }
            _ => EventResponse::IGNORED,
        }
    }

    fn handle_double_click(&mut self, lx: i32, ly: i32, _button: u32) -> EventResponse {
        // Double-click on a data row → SUBMIT
        if ly >= self.header_height as i32 {
            if let (Some(vis_row), Some(display_col)) = (self.row_at_y(ly), self.column_at_x(lx)) {
                let data_row = self.data_row(vis_row);
                if self.start_edit(data_row, display_col) {
                    let logical_col = self.display_order[display_col];
                    return if self.typed_editor_active(data_row, logical_col) {
                        EventResponse::SUBMIT
                    } else {
                        EventResponse::CONSUMED
                    };
                }
            }
            if self.selected_row().is_some() {
                return EventResponse::SUBMIT;
            }
        }
        EventResponse::CONSUMED
    }

    fn accepts_focus(&self) -> bool {
        true
    }
}

// ── Helpers ────────────────────────────────────────────────────────

fn draw_sort_arrow_up(s: &crate::draw::Surface, x: i32, y: i32, color: u32) {
    crate::draw::fill_rect(s, x + 2, y - 3, 1, 1, color);
    crate::draw::fill_rect(s, x + 1, y - 2, 3, 1, color);
    crate::draw::fill_rect(s, x, y - 1, 5, 1, color);
}

fn draw_sort_arrow_down(s: &crate::draw::Surface, x: i32, y: i32, color: u32) {
    crate::draw::fill_rect(s, x, y - 3, 5, 1, color);
    crate::draw::fill_rect(s, x + 1, y - 2, 3, 1, color);
    crate::draw::fill_rect(s, x + 2, y - 1, 1, 1, color);
}

fn draw_cell_editor_decorator(
    surface: &crate::draw::Surface,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    kind: u8,
    text: &[u8],
    tc: &'static crate::theme::ThemeColors,
) -> i32 {
    match kind {
        2 => {
            let size = crate::theme::scale_i32(14);
            let bx = x + w as i32 - crate::theme::scale_i32(22);
            let by = y + (h as i32 - size) / 2;
            crate::draw::draw_border(surface, bx, by, size as u32, size as u32, tc.separator);
            if ascii_eq_ignore_case(text, b"true") || text == b"1" {
                crate::draw::fill_rect(surface, bx + 3, by + size / 2, 3, 2, tc.accent);
                crate::draw::fill_rect(surface, bx + 5, by + size / 2 - 2, 2, 4, tc.accent);
                crate::draw::fill_rect(surface, bx + 7, by + size / 2 - 4, 5, 2, tc.accent);
            }
            22
        }
        3 => {
            let color = parse_argb_color(text).unwrap_or(tc.control_bg);
            let size = crate::theme::scale_i32(14);
            let sx = x + w as i32 - crate::theme::scale_i32(24);
            let sy = y + (h as i32 - size) / 2;
            crate::draw::fill_rect(surface, sx, sy, size as u32, size as u32, color);
            crate::draw::draw_border(surface, sx, sy, size as u32, size as u32, tc.separator);
            24
        }
        4 => {
            let cx = x + w as i32 - crate::theme::scale_i32(15);
            let cy = y + h as i32 / 2;
            draw_sort_arrow_down(surface, cx, cy, tc.text_secondary);
            18
        }
        _ => 0,
    }
}

fn parse_argb_color(bytes: &[u8]) -> Option<u32> {
    if bytes.len() != 9 || bytes[0] != b'#' {
        return None;
    }
    let mut value = 0u32;
    for &byte in &bytes[1..] {
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            _ => return None,
        };
        value = (value << 4) | digit as u32;
    }
    Some(value)
}

fn ascii_eq_ignore_case(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b.iter())
            .all(|(&l, &r)| l.to_ascii_lowercase() == r.to_ascii_lowercase())
}

fn is_argb_editor_char(char_code: u32) -> bool {
    char_code == b'#' as u32
        || (b'0' as u32..=b'9' as u32).contains(&char_code)
        || (b'a' as u32..=b'f' as u32).contains(&char_code)
        || (b'A' as u32..=b'F' as u32).contains(&char_code)
}

fn parse_u32(s: &[u8]) -> Option<u32> {
    let mut val = 0u32;
    if s.is_empty() {
        return None;
    }
    for &b in s {
        if b < b'0' || b > b'9' {
            return None;
        }
        val = val * 10 + (b - b'0') as u32;
    }
    Some(val)
}

/// Parse a numeric sort key from a byte slice (zero-allocation).
///
/// Returns `(is_number, integer_part, fractional_part)`. Non-numeric text
/// gets `is_number=false` and sorts after all numbers. Handles optional
/// leading whitespace, negative sign, and decimal point. Trailing suffixes
/// (e.g. "KB", "%") are ignored.
fn parse_sort_key(s: &[u8]) -> (bool, i64, i64) {
    let mut i = 0;
    // Skip leading whitespace
    while i < s.len() && s[i] == b' ' {
        i += 1;
    }
    if i >= s.len() {
        return (false, 0, 0);
    }

    let negative = s[i] == b'-';
    if negative {
        i += 1;
    }

    if i >= s.len() || s[i] < b'0' || s[i] > b'9' {
        return (false, 0, 0);
    }

    // Integer part
    let mut int_part: i64 = 0;
    while i < s.len() && s[i] >= b'0' && s[i] <= b'9' {
        int_part = int_part * 10 + (s[i] - b'0') as i64;
        i += 1;
    }

    // Fractional part (fixed-point, 6 decimal places)
    let mut frac_part: i64 = 0;
    if i < s.len() && s[i] == b'.' {
        i += 1;
        let mut scale = 100_000i64;
        while i < s.len() && s[i] >= b'0' && s[i] <= b'9' && scale > 0 {
            frac_part += (s[i] - b'0') as i64 * scale;
            scale /= 10;
            i += 1;
        }
    }

    if negative {
        int_part = -int_part;
        frac_part = -frac_part;
    }

    (true, int_part, frac_part)
}
