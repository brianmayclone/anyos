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
        match v { 1 => Self::Numeric, _ => Self::String }
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
    Resizing { col_index: usize, drag_start_x: i32, original_width: u32 },
    Reordering { col_index: usize, drag_start_x: i32, current_x: i32 },
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
            let align = parts.get(2).and_then(|s| s.first().map(|&b| CellAlign::from_u8(b.wrapping_sub(b'0')))).unwrap_or(CellAlign::Left);
            let sort_type = parts.get(3).and_then(|s| s.first().map(|&b| SortType::from_u8(b.wrapping_sub(b'0')))).unwrap_or(SortType::String);
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

    pub fn column_count(&self) -> usize { self.columns.len() }

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
                if ci >= col_count { break; }
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

    /// Get the display column index of the last click (-1 if none).
    pub fn last_click_col(&self) -> i32 { self.last_click_col }

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
            if self.is_row_selected(r) { return Some(r); }
        }
        None
    }

    /// Clamp scroll_y so the viewport doesn't extend past the last row.
    fn clamp_scroll(&mut self) {
        let content_h = self.row_count as i32 * self.row_height as i32;
        let viewport_h = (self.base.h as i32).saturating_sub(self.header_height as i32);
        let max_scroll = (content_h - viewport_h).max(0);
        if self.scroll_y > max_scroll {
            self.scroll_y = max_scroll;
        }
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
            _ => { self.sorted_rows.clear(); return; }
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
    fn base(&self) -> &ControlBase { &self.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.base }
    fn kind(&self) -> ControlKind { ControlKind::DataGrid }

    fn set_font_size(&mut self, size: u16) { self.font_size = size; }
    fn get_font_size(&self) -> u16 { if self.font_size > 0 { self.font_size } else { 13 } }

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
                    crate::draw::fill_rect(&clipped, x, row_y, w, self.row_height, tc.alt_row_bg);
                }

                // Cell text + icons
                let mut col_x = x - self.scroll_x;
                for disp_col in 0..col_count {
                    let logical_col = self.display_order[disp_col];
                    let col = &self.columns[logical_col];
                    let cell_idx = data_row * col_count + logical_col;

                    let cell_clip = clipped.with_clip(col_x, row_y, col.width, self.row_height);

                    // Draw per-cell background color (if set)
                    if cell_idx < self.cell_bg_colors.len() && self.cell_bg_colors[cell_idx] != 0 {
                        crate::draw::fill_rect(&cell_clip, col_x, row_y, col.width, self.row_height, self.cell_bg_colors[cell_idx]);
                    }

                    // Draw cell icon (if any)
                    let mut icon_offset: i32 = 0;
                    if cell_idx < self.cell_icons.len() {
                        if let Some(ref icon) = self.cell_icons[cell_idx] {
                            let iw = icon.width as i32;
                            let ih = icon.height as i32;
                            let ix = col_x + 4;
                            let iy = row_y + (self.row_height as i32 - ih) / 2;
                            crate::draw::blit_argb(&cell_clip, ix, iy, icon.width as u32, icon.height as u32, &icon.pixels);
                            icon_offset = iw + 4;
                        }
                    }

                    if cell_idx < self.cell_data.len() && !self.cell_data[cell_idx].is_empty() {
                        let text = &self.cell_data[cell_idx];
                        let default_color = if cell_idx < self.cell_colors.len() && self.cell_colors[cell_idx] != 0 {
                            self.cell_colors[cell_idx]
                        } else if selected {
                            0xFFFFFFFF
                        } else {
                            tc.text
                        };

                        let fs = if self.font_size > 0 { self.font_size } else { 13 };
                        let text_x = match col.align {
                            CellAlign::Left => col_x + 8 + icon_offset,
                            CellAlign::Center => {
                                let (tw, _) = crate::draw::text_size_at(text, fs);
                                col_x + icon_offset + (col.width as i32 - icon_offset - tw as i32) / 2
                            }
                            CellAlign::Right => {
                                let (tw, _) = crate::draw::text_size_at(text, fs);
                                col_x + col.width as i32 - 8 - tw as i32
                            }
                        };
                        let text_y = row_y + (self.row_height as i32 - fs as i32) / 2;

                        // Check for per-character colors
                        let has_char_colors = cell_idx < self.char_color_offsets.len()
                            && self.char_color_offsets[cell_idx] != u32::MAX;

                        if has_char_colors {
                            let base_off = self.char_color_offsets[cell_idx] as usize;
                            let text_len = text.len();
                            // Draw spans of consecutive characters with the same color
                            let mut cx = text_x;
                            let mut span_start = 0usize;
                            while span_start < text_len {
                                let cc_idx = base_off + span_start;
                                let span_color = if cc_idx < self.char_colors.len() && self.char_colors[cc_idx] != 0 {
                                    self.char_colors[cc_idx]
                                } else {
                                    default_color
                                };
                                // Extend span while same color
                                let mut span_end = span_start + 1;
                                while span_end < text_len {
                                    let next_idx = base_off + span_end;
                                    let next_color = if next_idx < self.char_colors.len() && self.char_colors[next_idx] != 0 {
                                        self.char_colors[next_idx]
                                    } else {
                                        default_color
                                    };
                                    if next_color != span_color { break; }
                                    span_end += 1;
                                }
                                let span = &text[span_start..span_end];
                                crate::draw::draw_text_sized(&cell_clip, cx, text_y, span_color, span, fs);
                                let (sw, _) = crate::draw::text_size_at(span, fs);
                                cx += sw as i32;
                                span_start = span_end;
                            }
                        } else {
                            crate::draw::draw_text_sized(&cell_clip, text_x, text_y, default_color, text, fs);
                        }
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
            // Column separator line — only draw down to content, not full control height
            let sep_h = (self.header_height + self.row_count as u32 * self.row_height).min(h);
            crate::draw::fill_rect(&clipped, col_x - 1, y, 1, sep_h, tc.separator);
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

        // ── Vertical scrollbar + minimap ──
        let content_h = self.row_count as u32 * self.row_height;
        let view_h = h.saturating_sub(self.header_height);
        if content_h > view_h && view_h > 4 {
            let has_minimap = !self.minimap_colors.is_empty();
            let bar_w = if has_minimap { 10u32 } else { 6u32 };
            let bar_x = x + w as i32 - bar_w as i32 - 2;
            let track_y = y + self.header_height as i32 + 2;
            let track_h = (view_h - 4) as i32;
            crate::draw::fill_rect(&clipped, bar_x, track_y, bar_w, track_h as u32, tc.scrollbar_track);

            // Minimap: draw colored markers for each row
            if has_minimap && self.row_count > 0 && track_h > 0 {
                let total = self.row_count as i32;
                for (row, &color) in self.minimap_colors.iter().enumerate() {
                    if color == 0 || row >= self.row_count { continue; }
                    let py = track_y + (row as i64 * track_h as i64 / total as i64) as i32;
                    let ph = ((track_h as i64 / total as i64).max(1)).min(3) as u32;
                    crate::draw::fill_rect(&clipped, bar_x, py, bar_w, ph, color);
                }

                // Viewport indicator (semi-transparent)
                let vp_y = track_y + (self.scroll_y as i64 * track_h as i64 / (self.row_count as i64 * self.row_height as i64)).max(0) as i32;
                let vp_h = (view_h as i64 * track_h as i64 / content_h as i64).max(4) as u32;
                crate::draw::fill_rect(&clipped, bar_x, vp_y, bar_w, vp_h, 0x30FFFFFF);
            }

            let thumb_h = ((view_h as u64 * track_h as u64) / content_h as u64).max(20) as i32;
            let max_scroll = (content_h - view_h) as i32;
            let scroll_frac = if max_scroll > 0 {
                (self.scroll_y as i64 * (track_h - thumb_h) as i64 / max_scroll as i64) as i32
            } else { 0 };
            let thumb_y = track_y + scroll_frac.max(0).min(track_h - thumb_h);
            crate::draw::fill_rounded_rect(&clipped, bar_x, thumb_y, bar_w, thumb_h as u32, 3, tc.scrollbar);
        }

        // ── Connector lines (drawn over a column) ──
        if !self.connector_lines.is_empty() && self.connector_column < col_count {
            let logical_col = self.display_order[self.connector_column];
            let col_w = self.columns[logical_col].width;
            // Compute column x position
            let mut conn_col_x = x - self.scroll_x;
            for dc in 0..self.connector_column {
                let lc = self.display_order[dc];
                conn_col_x += self.columns[lc].width as i32;
            }
            let conn_clip = clipped.with_clip(conn_col_x, y + self.header_height as i32, col_w, view_h as u32);
            let rh = self.row_height as i32;
            let base_y = y + self.header_height as i32 - self.scroll_y;
            let mid_x = conn_col_x + col_w as i32 / 2;

            for cl in &self.connector_lines {
                let y0 = base_y + cl.start_row as i32 * rh;
                let y1 = base_y + cl.end_row as i32 * rh + rh;
                // Filled background
                if cl.filled {
                    let fy = y0.max(y + self.header_height as i32);
                    let fy1 = y1.min(y + h as i32);
                    if fy1 > fy {
                        // Semi-transparent fill
                        let fill_color = (cl.color & 0x00FFFFFF) | 0x20000000;
                        crate::draw::fill_rect(&conn_clip, conn_col_x, fy, col_w, (fy1 - fy) as u32, fill_color);
                    }
                }
                // Top and bottom horizontal lines
                let lx0 = conn_col_x + 2;
                let lx1 = conn_col_x + col_w as i32 - 2;
                crate::draw::fill_rect(&conn_clip, lx0, y0, (lx1 - lx0) as u32, 1, cl.color);
                crate::draw::fill_rect(&conn_clip, lx0, y1 - 1, (lx1 - lx0) as u32, 1, cl.color);
                // Left and right vertical edges
                crate::draw::fill_rect(&conn_clip, lx0, y0, 1, (y1 - y0) as u32, cl.color);
                crate::draw::fill_rect(&conn_clip, lx1, y0, 1, (y1 - y0) as u32, cl.color);
            }
        }
    }

    fn is_interactive(&self) -> bool { true }

    fn handle_mouse_down(&mut self, lx: i32, ly: i32, button: u32) -> EventResponse {
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
        match self.drag_mode {
            DragMode::Resizing { col_index, drag_start_x, original_width } => {
                let delta = lx - drag_start_x;
                let logical_col = self.display_order[col_index];
                let min_w = self.columns[logical_col].min_width.max(30);
                let new_width = (original_width as i32 + delta).max(min_w as i32) as u32;
                self.columns[logical_col].width = new_width;
                self.base.mark_dirty();
                EventResponse::CHANGED
            }
            DragMode::Reordering { drag_start_x, ref mut current_x, .. } => {
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
        let content_h = self.row_count as i32 * self.row_height as i32;
        let viewport_h = self.base.h as i32 - self.header_height as i32;
        let max_scroll = (content_h - viewport_h).max(0);
        self.scroll_y = (self.scroll_y - delta * 20).max(0).min(max_scroll);
        self.base.mark_dirty();
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
        match keycode {
            KEY_ENTER => {
                if self.selected_row().is_some() {
                    return EventResponse::SUBMIT;
                }
                EventResponse::CONSUMED
            }
            KEY_UP => {
                if self.row_count == 0 { return EventResponse::CONSUMED; }
                let vis = self.selected_visual_row().unwrap_or(0);
                let new_vis = if vis > 0 { vis - 1 } else { 0 };
                self.select_visual_row(new_vis);
                EventResponse::CHANGED
            }
            KEY_DOWN => {
                if self.row_count == 0 { return EventResponse::CONSUMED; }
                let vis = self.selected_visual_row().unwrap_or(0);
                let new_vis = if vis + 1 < self.row_count { vis + 1 } else { self.row_count - 1 };
                self.select_visual_row(new_vis);
                EventResponse::CHANGED
            }
            KEY_HOME => {
                if self.row_count == 0 { return EventResponse::CONSUMED; }
                self.select_visual_row(0);
                EventResponse::CHANGED
            }
            KEY_END => {
                if self.row_count == 0 { return EventResponse::CONSUMED; }
                self.select_visual_row(self.row_count - 1);
                EventResponse::CHANGED
            }
            _ => EventResponse::IGNORED,
        }
    }

    fn handle_double_click(&mut self, _lx: i32, ly: i32, _button: u32) -> EventResponse {
        // Double-click on a data row → SUBMIT
        if ly >= self.header_height as i32 {
            if self.selected_row().is_some() {
                return EventResponse::SUBMIT;
            }
        }
        EventResponse::CONSUMED
    }

    fn accepts_focus(&self) -> bool { true }
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

/// Parse a numeric sort key from a byte slice (zero-allocation).
///
/// Returns `(is_number, integer_part, fractional_part)`. Non-numeric text
/// gets `is_number=false` and sorts after all numbers. Handles optional
/// leading whitespace, negative sign, and decimal point. Trailing suffixes
/// (e.g. "KB", "%") are ignored.
fn parse_sort_key(s: &[u8]) -> (bool, i64, i64) {
    let mut i = 0;
    // Skip leading whitespace
    while i < s.len() && s[i] == b' ' { i += 1; }
    if i >= s.len() {
        return (false, 0, 0);
    }

    let negative = s[i] == b'-';
    if negative { i += 1; }

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
