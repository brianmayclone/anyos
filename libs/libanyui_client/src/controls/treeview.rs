use crate::{Control, Widget, lib, KIND_TREE_VIEW};
use crate::events;
use crate::events::SelectionChangedEvent;

leaf_control!(TreeView, KIND_TREE_VIEW);

/// Node style constants.
pub const STYLE_NORMAL: u32 = 0;
pub const STYLE_BOLD: u32 = 1;

impl TreeView {
    /// Create a new empty TreeView with the given display size.
    pub fn new(w: u32, h: u32) -> Self {
        let id = (lib().create_control)(KIND_TREE_VIEW, core::ptr::null(), 0);
        (lib().set_size)(id, w, h);
        Self { ctrl: Control { id } }
    }

    /// Add a root-level node. Returns the node index.
    pub fn add_root(&self, text: &str) -> u32 {
        (lib().treeview_add_node)(self.ctrl.id, u32::MAX, text.as_ptr(), text.len() as u32)
    }

    /// Add a child node under the given parent. Returns the node index.
    pub fn add_child(&self, parent: u32, text: &str) -> u32 {
        (lib().treeview_add_node)(self.ctrl.id, parent, text.as_ptr(), text.len() as u32)
    }

    /// Remove a node and all its descendants.
    pub fn remove_node(&self, index: u32) {
        (lib().treeview_remove_node)(self.ctrl.id, index);
    }

    /// Set the text of a node.
    pub fn set_node_text(&self, index: u32, text: &str) {
        (lib().treeview_set_node_text)(self.ctrl.id, index, text.as_ptr(), text.len() as u32);
    }

    /// Set the icon of a node from ARGB pixel data.
    pub fn set_node_icon(&self, index: u32, pixels: &[u32], w: u32, h: u32) {
        (lib().treeview_set_node_icon)(self.ctrl.id, index, pixels.as_ptr(), w, h);
    }

    /// Set the icon of a node from an ICO file.
    pub fn set_node_icon_from_file(&self, index: u32, path: &str, size: u32) {
        if let Some(icon) = crate::Icon::load(path, size) {
            (lib().treeview_set_node_icon)(self.ctrl.id, index, icon.pixels.as_ptr(), icon.width, icon.height);
        }
    }

    /// Set the style of a node (STYLE_NORMAL=0, STYLE_BOLD=1).
    pub fn set_node_style(&self, index: u32, style: u32) {
        (lib().treeview_set_node_style)(self.ctrl.id, index, style);
    }

    /// Set the text color of a node (0 = use theme default).
    pub fn set_node_text_color(&self, index: u32, color: u32) {
        (lib().treeview_set_node_text_color)(self.ctrl.id, index, color);
    }

    /// Set whether a node is expanded.
    pub fn set_expanded(&self, index: u32, expanded: bool) {
        (lib().treeview_set_expanded)(self.ctrl.id, index, expanded as u32);
    }

    /// Check if a node is expanded.
    pub fn is_expanded(&self, index: u32) -> bool {
        (lib().treeview_get_expanded)(self.ctrl.id, index) != 0
    }

    /// Get the selected node index, or u32::MAX if none selected.
    pub fn selected(&self) -> u32 {
        (lib().treeview_get_selected)(self.ctrl.id)
    }

    /// Set the selected node (u32::MAX to deselect).
    pub fn set_selected(&self, index: u32) {
        (lib().treeview_set_selected)(self.ctrl.id, index);
    }

    /// Clear all nodes.
    pub fn clear(&self) {
        (lib().treeview_clear)(self.ctrl.id);
    }

    /// Get the total number of nodes.
    pub fn node_count(&self) -> u32 {
        (lib().treeview_get_node_count)(self.ctrl.id)
    }

    /// Set indent width (pixels per depth level).
    pub fn set_indent_width(&self, width: u32) {
        (lib().treeview_set_indent_width)(self.ctrl.id, width);
    }

    /// Set row height in pixels.
    pub fn set_row_height(&self, height: u32) {
        (lib().treeview_set_row_height)(self.ctrl.id, height);
    }

    /// Register a callback for when the selection changes.
    pub fn on_selection_changed(&self, mut f: impl FnMut(&SelectionChangedEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| {
            let index = Control::from_id(id).get_state();
            f(&SelectionChangedEvent { id, index });
        });
        (lib().on_change_fn)(self.ctrl.id, thunk, ud);
    }

    /// Register a callback for when a node is clicked/toggled.
    pub fn on_node_clicked(&self, mut f: impl FnMut(&crate::events::ClickEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| {
            f(&crate::events::ClickEvent { id });
        });
        (lib().on_click_fn)(self.ctrl.id, thunk, ud);
    }
}
