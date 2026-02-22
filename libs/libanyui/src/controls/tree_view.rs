//! TreeView — hierarchical tree control with expand/collapse, icons, and selection.

use alloc::vec;
use alloc::vec::Vec;
use crate::control::{Control, ControlBase, ControlKind, EventResponse};

/// A single node in the tree.
pub(crate) struct TreeNode {
    pub text: Vec<u8>,
    pub parent: Option<usize>,       // None = root-level node
    pub depth: u16,                   // cached indentation depth
    pub expanded: bool,               // expanded/collapsed state
    pub has_children: bool,           // cached: true if any node has this as parent
    pub icon_pixels: Vec<u32>,        // optional ARGB icon pixels
    pub icon_w: u16,
    pub icon_h: u16,
    pub style: u32,                   // bit0=bold
    pub text_color: u32,              // 0 = use default theme color
}

pub struct TreeView {
    pub(crate) base: ControlBase,
    nodes: Vec<TreeNode>,
    pub(crate) selected_node: Option<usize>,
    hovered_node: Option<usize>,
    scroll_y: i32,
    focused: bool,
    pub(crate) indent_width: u32,   // pixels per depth level, default 20
    pub(crate) row_height: u32,     // default 24
    pub(crate) icon_size: u32,      // default 16
}

impl TreeView {
    pub fn new(base: ControlBase) -> Self {
        Self {
            base,
            nodes: Vec::new(),
            selected_node: None,
            hovered_node: None,
            scroll_y: 0,
            focused: false,
            indent_width: 20,
            row_height: 24,
            icon_size: 16,
        }
    }

    // ── Node API ──────────────────────────────────────────────────────

    /// Add a node. `parent_index` = None for root, Some(idx) for child.
    /// Returns the index of the new node.
    pub fn add_node(&mut self, parent: Option<usize>, text: &[u8]) -> usize {
        let depth = if let Some(p) = parent {
            if p < self.nodes.len() {
                self.nodes[p].has_children = true;
                self.nodes[p].depth + 1
            } else {
                0
            }
        } else {
            0
        };
        let idx = self.nodes.len();
        self.nodes.push(TreeNode {
            text: text.to_vec(),
            parent,
            depth,
            expanded: true, // default expanded
            has_children: false,
            icon_pixels: Vec::new(),
            icon_w: 0,
            icon_h: 0,
            style: 0,
            text_color: 0,
        });
        self.base.dirty = true;
        idx
    }

    /// Remove a node and all its descendants. Fixes parent indices and selection.
    pub fn remove_node(&mut self, index: usize) {
        if index >= self.nodes.len() { return; }

        let old_len = self.nodes.len();
        let mut to_remove = vec![false; old_len];
        to_remove[index] = true;

        // Mark all descendants
        loop {
            let mut changed = false;
            for i in 0..old_len {
                if to_remove[i] { continue; }
                if let Some(p) = self.nodes[i].parent {
                    if p < old_len && to_remove[p] {
                        to_remove[i] = true;
                        changed = true;
                    }
                }
            }
            if !changed { break; }
        }

        // Build old-to-new index mapping
        let mut new_indices = vec![0usize; old_len];
        let mut new_idx = 0usize;
        for i in 0..old_len {
            if !to_remove[i] {
                new_indices[i] = new_idx;
                new_idx += 1;
            }
        }

        // Remove marked nodes (reverse order to preserve indices during removal)
        for i in (0..old_len).rev() {
            if to_remove[i] {
                self.nodes.remove(i);
            }
        }

        // Fix parent indices after removal
        for node in &mut self.nodes {
            if let Some(p) = node.parent {
                node.parent = Some(new_indices[p]);
            }
        }

        // Rebuild has_children flags
        for i in 0..self.nodes.len() {
            self.nodes[i].has_children = false;
        }
        for i in 0..self.nodes.len() {
            if let Some(p) = self.nodes[i].parent {
                if p < self.nodes.len() {
                    self.nodes[p].has_children = true;
                }
            }
        }

        // Fix selection
        if let Some(sel) = self.selected_node {
            if sel < old_len && to_remove[sel] {
                self.selected_node = None;
            } else if sel < old_len {
                self.selected_node = Some(new_indices[sel]);
            } else {
                self.selected_node = None;
            }
        }

        self.base.dirty = true;
    }

    /// Get node text.
    pub fn node_text(&self, index: usize) -> &[u8] {
        if index < self.nodes.len() {
            &self.nodes[index].text
        } else {
            &[]
        }
    }

    /// Set node text.
    pub fn set_node_text(&mut self, index: usize, text: &[u8]) {
        if index < self.nodes.len() {
            self.nodes[index].text.clear();
            self.nodes[index].text.extend_from_slice(text);
            self.base.dirty = true;
        }
    }

    /// Set node icon pixels (ARGB).
    pub fn set_node_icon(&mut self, index: usize, pixels: &[u32], w: u16, h: u16) {
        if index < self.nodes.len() {
            self.nodes[index].icon_pixels = pixels.to_vec();
            self.nodes[index].icon_w = w;
            self.nodes[index].icon_h = h;
            self.base.dirty = true;
        }
    }

    /// Set node style (bit0=bold).
    pub fn set_node_style(&mut self, index: usize, style: u32) {
        if index < self.nodes.len() {
            self.nodes[index].style = style;
            self.base.dirty = true;
        }
    }

    /// Set node text color (0 = theme default).
    pub fn set_node_text_color(&mut self, index: usize, color: u32) {
        if index < self.nodes.len() {
            self.nodes[index].text_color = color;
            self.base.dirty = true;
        }
    }

    /// Set expanded/collapsed state for a node.
    pub fn set_expanded(&mut self, index: usize, expanded: bool) {
        if index < self.nodes.len() {
            self.nodes[index].expanded = expanded;
            self.base.dirty = true;
        }
    }

    /// Check if a node is expanded.
    pub fn is_expanded(&self, index: usize) -> bool {
        if index < self.nodes.len() {
            self.nodes[index].expanded
        } else {
            false
        }
    }

    /// Get selected node index.
    pub fn selected(&self) -> Option<usize> {
        self.selected_node
    }

    /// Set selected node.
    pub fn set_selected(&mut self, index: Option<usize>) {
        if self.selected_node != index {
            self.selected_node = index;
            self.base.dirty = true;
        }
    }

    /// Clear all nodes.
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.selected_node = None;
        self.hovered_node = None;
        self.scroll_y = 0;
        self.base.dirty = true;
    }

    /// Get the total number of nodes.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    // ── Internal helpers ──────────────────────────────────────────────

    /// Check if all ancestors of a node are expanded.
    fn is_ancestor_chain_expanded(&self, index: usize) -> bool {
        let mut current = self.nodes[index].parent;
        while let Some(parent_idx) = current {
            if parent_idx >= self.nodes.len() { return false; }
            if !self.nodes[parent_idx].expanded { return false; }
            current = self.nodes[parent_idx].parent;
        }
        true
    }

    /// Get indices of all visible nodes (ancestors all expanded).
    fn visible_nodes(&self) -> Vec<usize> {
        let mut result = Vec::new();
        for (i, _node) in self.nodes.iter().enumerate() {
            if self.is_ancestor_chain_expanded(i) {
                result.push(i);
            }
        }
        result
    }

    /// Total content height based on visible nodes.
    fn content_height(&self) -> u32 {
        self.visible_nodes().len() as u32 * self.row_height
    }

    /// Clamp scroll_y to valid range.
    fn clamp_scroll(&mut self) {
        let visible_h = self.base.h.saturating_sub(2) as i32; // -2 for border
        let content_h = self.content_height() as i32;
        let max_scroll = (content_h - visible_h).max(0);
        self.scroll_y = self.scroll_y.max(0).min(max_scroll);
    }

    /// Ensure the selected node is visible by scrolling.
    fn ensure_selected_visible(&mut self) {
        if let Some(sel) = self.selected_node {
            let vis = self.visible_nodes();
            if let Some(vis_idx) = vis.iter().position(|&i| i == sel) {
                let row_y = vis_idx as i32 * self.row_height as i32;
                let visible_h = self.base.h.saturating_sub(2) as i32;
                if row_y < self.scroll_y {
                    self.scroll_y = row_y;
                } else if row_y + self.row_height as i32 > self.scroll_y + visible_h {
                    self.scroll_y = row_y + self.row_height as i32 - visible_h;
                }
                self.clamp_scroll();
            }
        }
    }
}

impl Control for TreeView {
    fn base(&self) -> &ControlBase { &self.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.base }
    fn kind(&self) -> ControlKind { ControlKind::TreeView }

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

        // Border
        crate::draw::draw_border(&clipped, x, y, w, h, tc.card_border);

        if self.nodes.is_empty() { return; }

        let vis = self.visible_nodes();
        let rh = self.row_height as i32;
        let inner_y = y + 1; // inside border
        let inner_h = h.saturating_sub(2) as i32;
        let scrollbar_w = if self.content_height() > h.saturating_sub(2) { 8i32 } else { 0 };

        for (vis_idx, &node_idx) in vis.iter().enumerate() {
            let row_y = inner_y + (vis_idx as i32) * rh - self.scroll_y;

            // Skip rows outside the visible viewport
            if row_y + rh <= inner_y || row_y >= inner_y + inner_h {
                continue;
            }

            let node = &self.nodes[node_idx];
            let is_selected = self.selected_node == Some(node_idx);
            let is_hovered = self.hovered_node == Some(node_idx);

            // Row highlight
            if is_selected {
                crate::draw::fill_rect(&clipped, x + 1, row_y, (w - 2).saturating_sub(scrollbar_w as u32), self.row_height, tc.selection);
            } else if is_hovered {
                crate::draw::fill_rect(&clipped, x + 1, row_y, (w - 2).saturating_sub(scrollbar_w as u32), self.row_height, tc.control_hover);
            }

            let mut x_offset = x + 4 + (node.depth as i32) * self.indent_width as i32;

            // Disclosure triangle (if node has children)
            if node.has_children {
                let tri_x = x_offset + 2;
                let tri_cy = row_y + rh / 2;
                if node.expanded {
                    // ▼ pointing down (6 rows)
                    for row in 0..6i32 {
                        let half = 5 - row;
                        crate::draw::fill_rect(
                            &clipped,
                            tri_x - half,
                            tri_cy - 3 + row,
                            (half * 2 + 1) as u32,
                            1,
                            tc.text_secondary,
                        );
                    }
                } else {
                    // ▶ pointing right (6 rows)
                    for row in 0..6i32 {
                        let half = if row < 3 { row } else { 5 - row };
                        crate::draw::fill_rect(
                            &clipped,
                            tri_x,
                            tri_cy - 3 + row,
                            (half + 1) as u32 * 2,
                            1,
                            tc.text_secondary,
                        );
                    }
                }
            }

            x_offset += 16; // past disclosure triangle area

            // Icon
            if !node.icon_pixels.is_empty() && node.icon_w > 0 && node.icon_h > 0 {
                let icon_y = row_y + (rh - node.icon_h as i32) / 2;
                crate::draw::blit_buffer(
                    &clipped,
                    x_offset,
                    icon_y,
                    node.icon_w as u32,
                    node.icon_h as u32,
                    &node.icon_pixels,
                );
                x_offset += self.icon_size as i32 + 4;
            }

            // Text
            if !node.text.is_empty() {
                let text_color = if node.text_color != 0 {
                    node.text_color
                } else if is_selected {
                    tc.toggle_thumb // white on selection
                } else {
                    tc.text
                };

                let text_y = row_y + (rh - 13) / 2;
                let font_id: u16 = if node.style & 1 != 0 { 1 } else { 0 };
                crate::draw::draw_text_ex(&clipped, x_offset, text_y, text_color, &node.text, font_id, 13);
            }
        }

        // ── Scrollbar ──
        let content_h = vis.len() as u32 * self.row_height;
        let view_h = h.saturating_sub(2);
        if content_h > view_h && view_h > 4 {
            let bar_w = 6u32;
            let bar_x = x + w as i32 - bar_w as i32 - 2;
            let track_y = y + 2;
            let track_h = (view_h - 4) as i32;

            // Track
            crate::draw::fill_rect(&clipped, bar_x, track_y, bar_w, track_h as u32, tc.scrollbar_track);

            // Thumb
            let thumb_h = ((view_h as u64 * track_h as u64) / content_h as u64).max(20) as i32;
            let max_scroll = (content_h - view_h) as i32;
            let scroll_frac = if max_scroll > 0 {
                (self.scroll_y as i64 * (track_h - thumb_h) as i64 / max_scroll as i64) as i32
            } else {
                0
            };
            let thumb_y = track_y + scroll_frac.max(0).min(track_h - thumb_h);
            crate::draw::fill_rounded_rect(&clipped, bar_x, thumb_y, bar_w, thumb_h as u32, 3, tc.scrollbar);
        }

        // Focus ring
        if self.focused {
            crate::draw::draw_border(&clipped, x, y, w, h, tc.accent);
        }
    }

    fn is_interactive(&self) -> bool { true }
    fn accepts_focus(&self) -> bool { true }

    fn handle_click(&mut self, lx: i32, ly: i32, _button: u32) -> EventResponse {
        let vis = self.visible_nodes();
        let rh = self.row_height as i32;
        let vis_idx = (ly - 1 + self.scroll_y) / rh; // -1 for top border

        if vis_idx < 0 || vis_idx as usize >= vis.len() {
            return EventResponse::CONSUMED;
        }

        let node_idx = vis[vis_idx as usize];
        let node_depth = self.nodes[node_idx].depth;
        let has_children = self.nodes[node_idx].has_children;

        // Check if click is on the disclosure triangle area
        let triangle_x = 4 + node_depth as i32 * self.indent_width as i32;
        if lx >= triangle_x && lx < triangle_x + 16 && has_children {
            // Toggle expand/collapse
            self.nodes[node_idx].expanded = !self.nodes[node_idx].expanded;
            self.clamp_scroll();
            self.base.dirty = true;
            return EventResponse::CHANGED;
        }

        // Select the node
        self.selected_node = Some(node_idx);
        self.base.state = node_idx as u32;
        self.base.dirty = true;
        EventResponse::CHANGED
    }

    fn handle_key_down(&mut self, keycode: u32, char_code: u32) -> EventResponse {
        let vis = self.visible_nodes();
        if vis.is_empty() { return EventResponse::IGNORED; }

        match keycode {
            // Up arrow
            0x48 => {
                if let Some(sel) = self.selected_node {
                    if let Some(pos) = vis.iter().position(|&i| i == sel) {
                        if pos > 0 {
                            self.selected_node = Some(vis[pos - 1]);
                            self.base.state = vis[pos - 1] as u32;
                            self.ensure_selected_visible();
                            self.base.dirty = true;
                            return EventResponse::CHANGED;
                        }
                    }
                } else {
                    // No selection: select first visible
                    self.selected_node = Some(vis[0]);
                    self.base.state = vis[0] as u32;
                    self.ensure_selected_visible();
                    self.base.dirty = true;
                    return EventResponse::CHANGED;
                }
                EventResponse::CONSUMED
            }
            // Down arrow
            0x50 => {
                if let Some(sel) = self.selected_node {
                    if let Some(pos) = vis.iter().position(|&i| i == sel) {
                        if pos + 1 < vis.len() {
                            self.selected_node = Some(vis[pos + 1]);
                            self.base.state = vis[pos + 1] as u32;
                            self.ensure_selected_visible();
                            self.base.dirty = true;
                            return EventResponse::CHANGED;
                        }
                    }
                } else {
                    // No selection: select first visible
                    self.selected_node = Some(vis[0]);
                    self.base.state = vis[0] as u32;
                    self.ensure_selected_visible();
                    self.base.dirty = true;
                    return EventResponse::CHANGED;
                }
                EventResponse::CONSUMED
            }
            // Left arrow: collapse current node (or move to parent)
            0x4B => {
                if let Some(sel) = self.selected_node {
                    if sel < self.nodes.len() {
                        if self.nodes[sel].has_children && self.nodes[sel].expanded {
                            // Collapse
                            self.nodes[sel].expanded = false;
                            self.clamp_scroll();
                            self.base.dirty = true;
                            return EventResponse::CHANGED;
                        } else if let Some(parent_idx) = self.nodes[sel].parent {
                            // Move selection to parent
                            self.selected_node = Some(parent_idx);
                            self.base.state = parent_idx as u32;
                            self.ensure_selected_visible();
                            self.base.dirty = true;
                            return EventResponse::CHANGED;
                        }
                    }
                }
                EventResponse::CONSUMED
            }
            // Right arrow: expand current node (or move to first child)
            0x4D => {
                if let Some(sel) = self.selected_node {
                    if sel < self.nodes.len() {
                        if self.nodes[sel].has_children && !self.nodes[sel].expanded {
                            // Expand
                            self.nodes[sel].expanded = true;
                            self.base.dirty = true;
                            return EventResponse::CHANGED;
                        } else if self.nodes[sel].has_children && self.nodes[sel].expanded {
                            // Move to first child
                            let vis_after = self.visible_nodes();
                            if let Some(pos) = vis_after.iter().position(|&i| i == sel) {
                                if pos + 1 < vis_after.len() {
                                    let next = vis_after[pos + 1];
                                    // Only move if it is actually a child
                                    if self.nodes[next].parent == Some(sel) {
                                        self.selected_node = Some(next);
                                        self.base.state = next as u32;
                                        self.ensure_selected_visible();
                                        self.base.dirty = true;
                                        return EventResponse::CHANGED;
                                    }
                                }
                            }
                        }
                    }
                }
                EventResponse::CONSUMED
            }
            // Enter
            0x1C => {
                EventResponse::CLICK
            }
            _ => {
                // Enter via char_code
                if char_code == 0x0A || char_code == 0x0D {
                    return EventResponse::CLICK;
                }
                EventResponse::IGNORED
            }
        }
    }

    fn handle_scroll(&mut self, delta: i32) -> EventResponse {
        let content_h = self.content_height() as i32;
        let visible_h = self.base.h.saturating_sub(2) as i32;
        let max_scroll = (content_h - visible_h).max(0);
        self.scroll_y = (self.scroll_y - delta * 20).max(0).min(max_scroll);
        self.base.dirty = true;
        EventResponse::CONSUMED
    }

    fn handle_mouse_move(&mut self, _lx: i32, ly: i32) -> EventResponse {
        let vis = self.visible_nodes();
        let rh = self.row_height as i32;
        let vis_idx = (ly - 1 + self.scroll_y) / rh;

        let new_hover = if vis_idx >= 0 && (vis_idx as usize) < vis.len() {
            Some(vis[vis_idx as usize])
        } else {
            None
        };

        if new_hover != self.hovered_node {
            self.hovered_node = new_hover;
            self.base.dirty = true;
        }
        EventResponse::IGNORED
    }

    fn handle_mouse_leave(&mut self) {
        if self.hovered_node.is_some() {
            self.hovered_node = None;
            self.base.dirty = true;
        }
    }

    fn handle_focus(&mut self) {
        self.focused = true;
        self.base.dirty = true;
    }

    fn handle_blur(&mut self) {
        self.focused = false;
        self.base.dirty = true;
    }
}
