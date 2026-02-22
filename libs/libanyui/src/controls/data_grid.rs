//! DataGrid — full-featured data grid with sorting, resizing, reordering.

use alloc::vec;
use alloc::vec::Vec;
use crate::control::{Control, ControlBase, ControlKind, EventResponse};

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
        match v { 1 => Self::Center, 2 => Self::Right, _ => Self::Left }
    }
}

/// Sort direction for a column.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    None,
    Ascending,
    Descending,
}

/// A single column definition.
#[derive(Clone)]
pub struct Column {
    pub header: Vec<u8>,
    pub width: u32,
    pub min_width: u32,
    pub align: CellAlign,
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
    Resizing { col_index: usize, drag_start_x: i32, original_width: u32 },
    Reordering { col_index: usize, drag_start_x: i32, current_x: i32 },
}

pub struct DataGrid {
    pub(crate) base: ControlBase,
    columns: Vec<Column>,
    display_order: Vec<usize>,
    cell_data: Vec<Vec<u8>>,
    cell_colors: Vec<u32>,
    pub(crate) row_count: usize,
    sort_column: Option<usize>,
    sort_direction: SortDirection,
    sorted_rows: Vec<usize>,
    scroll_y: i32,
    scroll_x: i32,
    selection_mode: SelectionMode,
    selected_rows: Vec<u8>,
    drag_mode: DragMode,
    hovered_row: Option<usize>,
    pub(crate) header_height: u32,
    pub(crate) row_height: u32,
}

impl DataGrid {
    pub fn new(base: ControlBase) -> Self {
        Self {
            base,
            columns: Vec::new(),
            display_order: Vec::new(),
            cell_data: Vec::new(),
            cell_colors: Vec::new(),
            row_count: 0,
            sort_column: None,
            sort_direction: SortDirection::None,
            sorted_rows: Vec::new(),
            scroll_y: 0,
            scroll_x: 0,
            selection_mode: SelectionMode::Single,
            selected_rows: Vec::new(),
            drag_mode: DragMode::None,
            hovered_row: None,
            header_height: 32,
            row_height: 28,
        }
    }

    // ── Column API ─────────────────────────────────────────────────

    pub fn set_columns_from_data(&mut self, data: &[u8]) {
        self.columns.clear();
        self.display_order.clear();
        // Format: header\x1Fwidth\x1Falign\x1Eheader\x1Fwidth\x1Falign
        for (i, col_data) in data.split(|&b| b == 0x1E).enumerate() {
            let parts: Vec<&[u8]> = col_data.split(|&b| b == 0x1F).collect();
            let header = parts.first().copied().unwrap_or(&[]);
            let width = parts.get(1).and_then(|s| parse_u32(s)).unwrap_or(100);
            let align = parts.get(2).and_then(|s| s.first().map(|&b| CellAlign::from_u8(b.wrapping_sub(b'0')))).unwrap_or(CellAlign::Left);
            self.columns.push(Column {
                header: header.to_vec(),
                width,
                min_width: 30,
                align,
            });
            self.display_order.push(i);
        }
        self.base.dirty = true;
    }

    pub fn column_count(&self) -> usize { self.columns.len() }

    pub fn set_column_width(&mut self, col_index: usize, width: u32) {
        if col_index < self.columns.len() {
            self.columns[col_index].width = width.max(self.columns[col_index].min_width);
            self.base.dirty = true;
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
                if ci >= col_count { break; }
                self.cell_data.push(cell.to_vec());
            }
            // Pad with empty cells if row has fewer columns
            for _ in cells.len()..col_count {
                self.cell_data.push(Vec::new());
            }
            self.row_count += 1;
        }
        self.ensure_selection_bits();
        self.rebuild_sort();
        self.base.dirty = true;
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
        self.ensure_selection_bits();
        self.rebuild_sort();
        self.base.dirty = true;
    }

    pub fn set_cell(&mut self, row: usize, col: usize, text: &[u8]) {
        let col_count = self.columns.len().max(1);
        let idx = row * col_count + col;
        if idx < self.cell_data.len() {
            self.cell_data[idx] = text.to_vec();
            self.base.dirty = true;
        }
    }

    pub fn get_cell(&self, row: usize, col: usize) -> &[u8] {
        let col_count = self.columns.len().max(1);
        let idx = row * col_count + col;
        self.cell_data.get(idx).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn set_cell_colors(&mut self, colors: &[u32]) {
        self.cell_colors = colors.to_vec();
        self.base.dirty = true;
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
        if row >= self.row_count { return false; }
        let byte = row / 8;
        let bit = row % 8;
        byte < self.selected_rows.len() && (self.selected_rows[byte] & (1 << bit)) != 0
    }

    pub(crate) fn set_row_selected(&mut self, row: usize, selected: bool) {
        if row >= self.row_count { return; }
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
        self.sort_column = if direction == SortDirection::None { None } else { Some(column) };
        self.sort_direction = direction;
        self.rebuild_sort();
        self.base.dirty = true;
    }

    fn rebuild_sort(&mut self) {
        if self.sort_direction == SortDirection::None || self.sort_column.is_none() {
            self.sorted_rows.clear();
            return;
        }
        let col_count = self.columns.len().max(1);
        let logical_col = match self.sort_column {
            Some(dc) if dc < self.display_order.len() => self.display_order[dc],
            _ => { self.sorted_rows.clear(); return; }
        };
        self.sorted_rows = (0..self.row_count).collect();
        let ascending = self.sort_direction == SortDirection::Ascending;
        let data = &self.cell_data;
        self.sorted_rows.sort_by(|&a, &b| {
            let a_idx = a * col_count + logical_col;
            let b_idx = b * col_count + logical_col;
            let a_text = data.get(a_idx).map(|v| v.as_slice()).unwrap_or(&[]);
            let b_text = data.get(b_idx).map(|v| v.as_slice()).unwrap_or(&[]);
            let ord = a_text.cmp(b_text);
            if ascending { ord } else { ord.reverse() }
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
        if ly < self.header_height as i32 { return None; }
        let data_y = ly - self.header_height as i32 + self.scroll_y;
        let row = data_y / self.row_height as i32;
        if row >= 0 && (row as usize) < self.row_count {
            Some(row as usize)
        } else {
            None
        }
    }

    fn data_row(&self, vis_row: usize) -> usize {
        if self.sorted_rows.is_empty() { vis_row } else { self.sorted_rows[vis_row] }
    }

    fn total_columns_width(&self) -> u32 {
        self.display_order.iter().map(|&i| self.columns[i].width).sum()
    }
}

impl Control for DataGrid {
    fn base(&self) -> &ControlBase { &self.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.base }
    fn kind(&self) -> ControlKind { ControlKind::DataGrid }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let x = ax + self.base.x;
        let y = ay + self.base.y;
        let w = self.base.w;
        let h = self.base.h;
        let tc = crate::theme::colors();

        // Clip to control bounds
        let clipped = surface.with_clip(x, y, w, h);

        // Background
        crate::draw::fill_rect(&clipped, x, y, w, h, tc.card_bg);

        if self.columns.is_empty() { return; }

        let col_count = self.columns.len();

        // ── Data rows (scrolled) ──
        let viewport_h = h.saturating_sub(self.header_height) as i32;
        if viewport_h > 0 && self.row_count > 0 {
            let rh = self.row_height as i32;
            let vis_start = (self.scroll_y / rh).max(0) as usize;
            let vis_end = ((self.scroll_y + viewport_h) / rh + 2).min(self.row_count as i32) as usize;

            for vis_row in vis_start..vis_end {
                let data_row = self.data_row(vis_row);
                let row_y = y + self.header_height as i32 + (vis_row as i32) * rh - self.scroll_y;

                // Row background
                let selected = self.is_row_selected(data_row);
                if selected {
                    crate::draw::fill_rect(&clipped, x, row_y, w, self.row_height, tc.selection);
                } else if Some(vis_row) == self.hovered_row {
                    crate::draw::fill_rect(&clipped, x, row_y, w, self.row_height, tc.control_hover);
                } else if vis_row % 2 == 1 {
                    crate::draw::fill_rect(&clipped, x, row_y, w, self.row_height, 0xFF232323);
                }

                // Cell text
                let mut col_x = x - self.scroll_x;
                for disp_col in 0..col_count {
                    let logical_col = self.display_order[disp_col];
                    let col = &self.columns[logical_col];
                    let cell_idx = data_row * col_count + logical_col;

                    if cell_idx < self.cell_data.len() && !self.cell_data[cell_idx].is_empty() {
                        let text = &self.cell_data[cell_idx];
                        let color = if cell_idx < self.cell_colors.len() && self.cell_colors[cell_idx] != 0 {
                            self.cell_colors[cell_idx]
                        } else if selected {
                            0xFFFFFFFF
                        } else {
                            tc.text
                        };

                        let text_x = match col.align {
                            CellAlign::Left => col_x + 8,
                            CellAlign::Center => {
                                let (tw, _) = crate::draw::text_size(text);
                                col_x + (col.width as i32 - tw as i32) / 2
                            }
                            CellAlign::Right => {
                                let (tw, _) = crate::draw::text_size(text);
                                col_x + col.width as i32 - 8 - tw as i32
                            }
                        };
                        let text_y = row_y + (self.row_height as i32 - 13) / 2;
                        // Clip text to column bounds to prevent overflow into adjacent columns
                        let cell_clip = clipped.with_clip(col_x, row_y, col.width, self.row_height);
                        crate::draw::draw_text(&cell_clip, text_x, text_y, color, text);
                    }

                    col_x += col.width as i32;
                }

                // Row separator
                crate::draw::fill_rect(&clipped, x, row_y + rh - 1, w, 1, tc.separator);
            }
        }

        // ── Header (drawn over data, doesn't scroll vertically) ──
        crate::draw::fill_rect(&clipped, x, y, w, self.header_height, tc.control_bg);

        let mut col_x = x - self.scroll_x;
        for disp_col in 0..col_count {
            let logical_col = self.display_order[disp_col];
            let col = &self.columns[logical_col];

            // Header text (clipped to column bounds)
            let text_y = y + (self.header_height as i32 - 13) / 2;
            let hdr_clip = clipped.with_clip(col_x, y, col.width, self.header_height);
            crate::draw::draw_text(&hdr_clip, col_x + 8, text_y, tc.text, &col.header);

            // Sort indicator
            if self.sort_column == Some(disp_col) && self.sort_direction != SortDirection::None {
                let ix = col_x + col.width as i32 - 16;
                let iy = y + (self.header_height as i32) / 2;
                if self.sort_direction == SortDirection::Ascending {
                    draw_sort_arrow_up(&clipped, ix, iy, tc.accent);
                } else {
                    draw_sort_arrow_down(&clipped, ix, iy, tc.accent);
                }
            }

            col_x += col.width as i32;
            // Column separator line
            crate::draw::fill_rect(&clipped, col_x - 1, y, 1, h, tc.separator);
        }

        // Header bottom border
        crate::draw::fill_rect(&clipped, x, y + self.header_height as i32 - 1, w, 1, tc.separator);

        // ── Reorder visual feedback ──
        if let DragMode::Reordering { col_index, current_x, drag_start_x } = self.drag_mode {
            if (current_x - drag_start_x).abs() > 5 && col_index < self.display_order.len() {
                let logical = self.display_order[col_index];
                let cw = self.columns[logical].width;
                crate::draw::fill_rect(&clipped, x + current_x, y, cw, h, 0x40007AFF);
                crate::draw::fill_rect(&clipped, x + current_x, y, 2, h, tc.accent);
            }
        }

        // ── Vertical scrollbar ──
        let content_h = self.row_count as u32 * self.row_height;
        let view_h = h.saturating_sub(self.header_height);
        if content_h > view_h && view_h > 4 {
            let bar_w = 6u32;
            let bar_x = x + w as i32 - bar_w as i32 - 2;
            let track_y = y + self.header_height as i32 + 2;
            let track_h = (view_h - 4) as i32;
            crate::draw::fill_rect(&clipped, bar_x, track_y, bar_w, track_h as u32, tc.scrollbar_track);
            let thumb_h = ((view_h as u64 * track_h as u64) / content_h as u64).max(20) as i32;
            let max_scroll = (content_h - view_h) as i32;
            let scroll_frac = if max_scroll > 0 {
                (self.scroll_y as i64 * (track_h - thumb_h) as i64 / max_scroll as i64) as i32
            } else { 0 };
            let thumb_y = track_y + scroll_frac.max(0).min(track_h - thumb_h);
            crate::draw::fill_rounded_rect(&clipped, bar_x, thumb_y, bar_w, thumb_h as u32, 3, tc.scrollbar);
        }
    }

    fn is_interactive(&self) -> bool { true }

    fn handle_mouse_down(&mut self, lx: i32, ly: i32, _button: u32) -> EventResponse {
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
        EventResponse::CONSUMED
    }

    fn handle_mouse_move(&mut self, lx: i32, ly: i32) -> EventResponse {
        match self.drag_mode {
            DragMode::Resizing { col_index, drag_start_x, original_width } => {
                let delta = lx - drag_start_x;
                let logical_col = self.display_order[col_index];
                let min_w = self.columns[logical_col].min_width.max(30);
                let new_width = (original_width as i32 + delta).max(min_w as i32) as u32;
                self.columns[logical_col].width = new_width;
                self.base.dirty = true;
                EventResponse::CHANGED
            }
            DragMode::Reordering { drag_start_x, ref mut current_x, .. } => {
                if (lx - drag_start_x).abs() > 5 {
                    *current_x = lx;
                    self.base.dirty = true;
                }
                EventResponse::CONSUMED
            }
            DragMode::None => {
                if ly >= self.header_height as i32 {
                    let new_hover = self.row_at_y(ly);
                    if new_hover != self.hovered_row {
                        self.hovered_row = new_hover;
                        self.base.dirty = true;
                    }
                } else if self.hovered_row.is_some() {
                    self.hovered_row = None;
                    self.base.dirty = true;
                }
                EventResponse::IGNORED
            }
        }
    }

    fn handle_mouse_up(&mut self, lx: i32, _ly: i32, _button: u32) -> EventResponse {
        let mode = core::mem::replace(&mut self.drag_mode, DragMode::None);
        match mode {
            DragMode::Reordering { col_index, drag_start_x, current_x } => {
                if (current_x - drag_start_x).abs() > 5 {
                    if let Some(target_col) = self.column_at_x(current_x) {
                        if target_col != col_index {
                            let val = self.display_order.remove(col_index);
                            self.display_order.insert(target_col, val);
                        }
                    }
                }
                self.base.dirty = true;
                EventResponse::CHANGED
            }
            DragMode::Resizing { .. } => {
                self.base.dirty = true;
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
                self.base.dirty = true;
            }
            EventResponse::CHANGED
        } else {
            // Row selection
            if let Some(vis_row) = self.row_at_y(ly) {
                let data_row = self.data_row(vis_row);
                match self.selection_mode {
                    SelectionMode::Single => {
                        self.clear_selection();
                        self.set_row_selected(data_row, true);
                        self.base.state = data_row as u32;
                    }
                    SelectionMode::Multi => {
                        let was = self.is_row_selected(data_row);
                        self.set_row_selected(data_row, !was);
                        self.base.state = data_row as u32;
                    }
                }
                self.base.dirty = true;
            }
            EventResponse::CHANGED
        }
    }

    fn handle_scroll(&mut self, delta: i32) -> EventResponse {
        let content_h = self.row_count as i32 * self.row_height as i32;
        let viewport_h = self.base.h as i32 - self.header_height as i32;
        let max_scroll = (content_h - viewport_h).max(0);
        self.scroll_y = (self.scroll_y - delta * 20).max(0).min(max_scroll);
        self.base.dirty = true;
        EventResponse::CONSUMED
    }

    fn handle_mouse_leave(&mut self) {
        if self.hovered_row.is_some() {
            self.hovered_row = None;
            self.base.dirty = true;
        }
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

fn parse_u32(s: &[u8]) -> Option<u32> {
    let mut val = 0u32;
    if s.is_empty() { return None; }
    for &b in s {
        if b < b'0' || b > b'9' { return None; }
        val = val * 10 + (b - b'0') as u32;
    }
    Some(val)
}
