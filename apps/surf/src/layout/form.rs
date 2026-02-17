//! Form field position collection for overlay rendering.

use alloc::vec::Vec;
use crate::dom::NodeId;
use super::{LayoutBox, FormFieldKind};

/// Position of a form field in document coordinates.
pub struct FormFieldPos {
    pub node_id: NodeId,
    pub kind: FormFieldKind,
    pub doc_x: i32,
    pub doc_y: i32,
    pub width: i32,
    pub height: i32,
}

/// Walk the layout tree and collect positions of all form fields.
pub fn collect_form_positions(root: &LayoutBox) -> Vec<FormFieldPos> {
    let mut positions = Vec::new();
    walk_form_positions(root, 0, 0, &mut positions);
    positions
}

fn walk_form_positions(bx: &LayoutBox, parent_x: i32, parent_y: i32, out: &mut Vec<FormFieldPos>) {
    let abs_x = parent_x + bx.x;
    let abs_y = parent_y + bx.y;

    if let Some(kind) = bx.form_field {
        if let Some(node_id) = bx.node_id {
            out.push(FormFieldPos {
                node_id,
                kind,
                doc_x: abs_x,
                doc_y: abs_y,
                width: bx.width,
                height: bx.height,
            });
        }
    }

    for child in &bx.children {
        walk_form_positions(child, abs_x, abs_y, out);
    }
}
