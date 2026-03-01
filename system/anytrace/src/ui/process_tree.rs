//! Process/thread tree view.

use libanyui_client as ui;
use ui::Widget;
use crate::logic::process_list::ProcessEntry;
use crate::util::format;

/// Process tree panel.
pub struct ProcessTreeView {
    pub tree: ui::TreeView,
    /// Cached TIDs in tree order, for mapping selection index → TID.
    tids: alloc::vec::Vec<u32>,
}

impl ProcessTreeView {
    /// Create the process tree view.
    pub fn new(_parent: &impl Widget) -> Self {
        let tree = ui::TreeView::new(300, 800);
        tree.set_dock(ui::DOCK_FILL);
        Self { tree, tids: alloc::vec::Vec::new() }
    }

    /// Build the label for a process entry.
    fn make_label(proc: &ProcessEntry) -> alloc::string::String {
        alloc::format!(
            "[{}] {} ({})",
            proc.tid,
            proc.name,
            format::thread_state_str(proc.state),
        )
    }

    /// Incrementally update the tree to match the current process list.
    ///
    /// Preserves the current selection (by TID) across updates.
    pub fn refresh(&mut self, processes: &[ProcessEntry]) {
        // Remember selected TID before modifying
        let selected_tid = self.selected_tid_internal();

        let new_count = processes.len();
        let old_count = self.tids.len();

        // Update existing nodes in-place
        let common = core::cmp::min(old_count, new_count);
        for i in 0..common {
            let proc = &processes[i];
            // Only update if TID or state changed
            if i >= self.tids.len() || self.tids[i] != proc.tid {
                let label = Self::make_label(proc);
                self.tree.set_node_text(i as u32, &label);
                self.tree.set_node_text_color(i as u32, format::thread_state_color(proc.state));
            } else {
                // Same TID — still update text (state/cpu_ticks may have changed)
                let label = Self::make_label(proc);
                self.tree.set_node_text(i as u32, &label);
                self.tree.set_node_text_color(i as u32, format::thread_state_color(proc.state));
            }
        }

        // Remove excess old nodes (from the end to avoid index shifts)
        if old_count > new_count {
            for i in (new_count..old_count).rev() {
                self.tree.remove_node(i as u32);
            }
        }

        // Add new nodes
        for i in old_count..new_count {
            let proc = &processes[i];
            let label = Self::make_label(proc);
            let node_id = self.tree.add_root(&label);
            self.tree.set_node_text_color(node_id, format::thread_state_color(proc.state));
        }

        // Update cached TID list
        self.tids.clear();
        self.tids.extend(processes.iter().map(|p| p.tid));

        // Restore selection by TID
        if let Some(tid) = selected_tid {
            if let Some(pos) = self.tids.iter().position(|&t| t == tid) {
                self.tree.set_selected(pos as u32);
            }
        }
    }

    /// Get the currently selected TID (internal, before refresh).
    fn selected_tid_internal(&self) -> Option<u32> {
        let sel = self.tree.selected();
        if sel != u32::MAX && (sel as usize) < self.tids.len() {
            Some(self.tids[sel as usize])
        } else {
            None
        }
    }

    /// Get the TID of the selected process (or 0 if none).
    pub fn selected_tid(&self, _processes: &[ProcessEntry]) -> u32 {
        self.selected_tid_internal().unwrap_or(0)
    }
}
