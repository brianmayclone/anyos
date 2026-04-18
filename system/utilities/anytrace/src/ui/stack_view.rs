//! Call stack view using TreeView.

use libanyui_client as ui;
use ui::Widget;
use crate::logic::unwinder::StackFrame;
use crate::util::format::hex64;

/// Call stack view panel.
pub struct StackView {
    pub tree: ui::TreeView,
}

impl StackView {
    /// Create the call stack view.
    pub fn new(_parent: &impl Widget) -> Self {
        let tree = ui::TreeView::new(600, 300);
        tree.set_dock(ui::DOCK_FILL);
        Self { tree }
    }

    /// Update the call stack display.
    pub fn update(&self, frames: &[StackFrame]) {
        self.tree.clear();
        for frame in frames {
            let label = if let Some(ref sym) = frame.symbol {
                alloc::format!("#{} {} + {:#x} ({})", frame.index, sym, frame.offset, hex64(frame.rip))
            } else {
                alloc::format!("#{} {} (RBP={})", frame.index, hex64(frame.rip), hex64(frame.rbp))
            };
            self.tree.add_root(&label);
        }
    }
}
