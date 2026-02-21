//! Layout engine — Windows Forms-inspired Dock layout with Padding and Margin.
//!
//! Called once per frame before rendering. Processes the control tree top-down,
//! positioning children based on their Dock style, Padding, and Margin.
//!
//! # Dock Algorithm
//! 1. Calculate parent's client area (bounds minus padding)
//! 2. Process docked children in insertion order:
//!    - Top: full width at top of remaining area
//!    - Bottom: full width at bottom of remaining area
//!    - Left: full height at left of remaining area
//!    - Right: full height at right of remaining area
//!    - Fill: all remaining area
//! 3. Children with Dock::None keep their manual (x, y) positions
//! 4. Recurse into all children

use alloc::boxed::Box;
use alloc::vec::Vec;
use crate::control::{Control, ControlId, DockStyle, find_idx};

/// Perform layout for a control and all its descendants.
pub fn perform_layout(controls: &mut Vec<Box<dyn Control>>, id: ControlId) {
    let idx = match find_idx(controls, id) {
        Some(i) => i,
        None => return,
    };

    // Collect info we need before mutating
    let children: Vec<ControlId> = controls[idx].base().children.to_vec();
    if children.is_empty() {
        return;
    }

    // Check if this control has a custom layout (StackPanel, FlowPanel, etc.)
    // layout_children takes &[Box<dyn Control>] (immutable) — no borrow conflict.
    let custom_layouts = controls[idx].layout_children(controls);

    if let Some(layouts) = custom_layouts {
        // Apply custom layout changes
        for cl in layouts {
            if let Some(ci) = find_idx(controls, cl.id) {
                controls[ci].set_position(cl.x, cl.y);
                if let Some(w) = cl.w {
                    if let Some(h) = cl.h {
                        controls[ci].set_size(w, h);
                    } else {
                        let old_h = controls[ci].base().h;
                        controls[ci].set_size(w, old_h);
                    }
                } else if let Some(h) = cl.h {
                    let old_w = controls[ci].base().w;
                    controls[ci].set_size(old_w, h);
                }
            }
        }
    } else {
        // Standard Dock layout
        let pad = controls[idx].base().padding;
        let pw = controls[idx].base().w;
        let ph = controls[idx].base().h;

        // Client area (inside padding)
        let mut area_left = pad.left;
        let mut area_top = pad.top;
        let mut area_right = pw as i32 - pad.right;
        let mut area_bottom = ph as i32 - pad.bottom;

        // Process docked children in order
        for &child_id in &children {
            let ci = match find_idx(controls, child_id) {
                Some(i) => i,
                None => continue,
            };

            if !controls[ci].base().visible {
                continue;
            }

            let dock = controls[ci].base().dock;
            let margin = controls[ci].base().margin;

            match dock {
                DockStyle::Top => {
                    let ch = controls[ci].base().h;
                    let x = area_left + margin.left;
                    let y = area_top + margin.top;
                    let w = (area_right - area_left - margin.left - margin.right).max(0) as u32;
                    controls[ci].set_position(x, y);
                    controls[ci].set_size(w, ch);
                    area_top += ch as i32 + margin.top + margin.bottom;
                }
                DockStyle::Bottom => {
                    let ch = controls[ci].base().h;
                    let x = area_left + margin.left;
                    let y = area_bottom - ch as i32 - margin.bottom;
                    let w = (area_right - area_left - margin.left - margin.right).max(0) as u32;
                    controls[ci].set_position(x, y);
                    controls[ci].set_size(w, ch);
                    area_bottom -= ch as i32 + margin.top + margin.bottom;
                }
                DockStyle::Left => {
                    let cw = controls[ci].base().w;
                    let x = area_left + margin.left;
                    let y = area_top + margin.top;
                    let h = (area_bottom - area_top - margin.top - margin.bottom).max(0) as u32;
                    controls[ci].set_position(x, y);
                    controls[ci].set_size(cw, h);
                    area_left += cw as i32 + margin.left + margin.right;
                }
                DockStyle::Right => {
                    let cw = controls[ci].base().w;
                    let x = area_right - cw as i32 - margin.right;
                    let y = area_top + margin.top;
                    let h = (area_bottom - area_top - margin.top - margin.bottom).max(0) as u32;
                    controls[ci].set_position(x, y);
                    controls[ci].set_size(cw, h);
                    area_right -= cw as i32 + margin.left + margin.right;
                }
                DockStyle::Fill => {
                    let x = area_left + margin.left;
                    let y = area_top + margin.top;
                    let w = (area_right - area_left - margin.left - margin.right).max(0) as u32;
                    let h = (area_bottom - area_top - margin.top - margin.bottom).max(0) as u32;
                    controls[ci].set_position(x, y);
                    controls[ci].set_size(w, h);
                }
                DockStyle::None => {
                    // Manual positioning — leave x/y as-is
                }
            }
        }
    }

    // Recurse into children
    for &child_id in &children {
        perform_layout(controls, child_id);
    }
}
