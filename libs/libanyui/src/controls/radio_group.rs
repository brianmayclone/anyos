//! RadioGroup — container that manages mutual exclusion of RadioButton children.
//!
//! When a RadioButton inside a RadioGroup is clicked, it posts a deferred
//! deselection request. The event loop drains these after handle_click returns,
//! calling `drain_deselects()` which deselects all sibling RadioButtons.
//!
//! RadioButtons NOT inside a RadioGroup do not get automatic deselection.

use alloc::boxed::Box;
use alloc::vec::Vec;
use crate::control::{Control, ControlBase, ControlKind, ControlId, ChildLayout, find_idx};

pub struct RadioGroup {
    pub(crate) base: ControlBase,
}

impl RadioGroup {
    pub fn new(base: ControlBase) -> Self {
        Self { base }
    }
}

// ── Deferred deselection queue ──────────────────────────────────────

/// Pending radio deselection requests: (group_id, selected_id).
/// Written by RadioButton::handle_click, drained by event loop.
static mut PENDING: [(u32, u32); 4] = [(0, 0); 4];
static mut PENDING_COUNT: usize = 0;

/// Post a deselection request (called from RadioButton::handle_click).
pub(crate) fn post_deselect(group_id: ControlId, selected_id: ControlId) {
    unsafe {
        if PENDING_COUNT < PENDING.len() {
            PENDING[PENDING_COUNT] = (group_id, selected_id);
            PENDING_COUNT += 1;
        }
    }
}

/// Drain pending deselection requests. Called from event loop after handle_click.
/// Deselects all RadioButton children in the group except the selected one.
/// Returns the list of group IDs that were affected (for firing change events).
pub(crate) fn drain_deselects(controls: &mut [Box<dyn Control>]) -> Vec<ControlId> {
    let count = unsafe { PENDING_COUNT };
    if count == 0 { return Vec::new(); }

    let mut affected_groups = Vec::new();
    for i in 0..count {
        let (group_id, selected_id) = unsafe { PENDING[i] };
        if let Some(gi) = find_idx(controls, group_id) {
            // Update RadioGroup state to the index of the selected child
            let children: Vec<ControlId> = controls[gi].base().children.to_vec();
            let mut sel_index = 0u32;
            for (ci_idx, &child_id) in children.iter().enumerate() {
                if child_id == selected_id {
                    sel_index = ci_idx as u32;
                }
                if child_id != selected_id {
                    if let Some(ci) = find_idx(controls, child_id) {
                        if controls[ci].kind() == ControlKind::RadioButton
                            && controls[ci].base().state != 0
                        {
                            controls[ci].base_mut().state = 0;
                            controls[ci].base_mut().mark_dirty();
                        }
                    }
                }
            }
            // Set group state to selected index
            controls[gi].base_mut().state = sel_index;
            controls[gi].base_mut().mark_dirty();
            affected_groups.push(group_id);
        }
    }
    unsafe { PENDING_COUNT = 0; }
    affected_groups
}

// ── Control implementation ──────────────────────────────────────────

impl Control for RadioGroup {
    fn base(&self) -> &ControlBase { &self.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.base }
    fn kind(&self) -> ControlKind { ControlKind::RadioGroup }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        // Transparent container — only renders background if color is set
        if self.base.color != 0 {
            let b = self.base();
            let p = crate::draw::scale_bounds(ax, ay, b.x, b.y, b.w, b.h);
            crate::draw::fill_rect(surface, p.x, p.y, p.w, p.h, self.base.color);
        }
    }

    fn layout_children(&self, controls: &[Box<dyn Control>]) -> Option<Vec<ChildLayout>> {
        // Stack children vertically with spacing
        let pad = &self.base.padding;
        let mut cursor_y = pad.top;
        let mut result = Vec::new();

        for &child_id in &self.base.children {
            let ci = match find_idx(controls, child_id) {
                Some(i) => i,
                None => continue,
            };
            if !controls[ci].base().visible {
                continue;
            }

            let m = controls[ci].base().margin;
            result.push(ChildLayout {
                id: child_id,
                x: pad.left + m.left,
                y: cursor_y + m.top,
                w: None,
                h: None,
            });
            cursor_y += controls[ci].base().h as i32 + m.top + m.bottom;
        }
        Some(result)
    }
}
