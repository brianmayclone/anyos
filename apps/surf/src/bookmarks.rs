// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Bookmark manager for Surf (Firefox-style).
//!
//! Uses a TreeView for hierarchical folder/bookmark display.
//! Each entry in `BookmarkStore.roots` is either a folder (with children)
//! or a bookmark (with a URL).

use alloc::string::String;
use alloc::vec::Vec;
use libanyui_client as ui_lib;
use ui_lib::Widget;

use crate::config::{self, BookmarkItem};
use crate::state;
use crate::tab;
use crate::ui;

// ═══════════════════════════════════════════════════════════
// Node mapping — flat tree index ↔ nested bookmark path
// ═══════════════════════════════════════════════════════════

/// Path into the nested `BookmarkStore.roots` tree.
/// For example, `[1, 0, 2]` means `roots[1].children[0].children[2]`.
#[derive(Clone)]
struct NodeMapping {
    path: Vec<usize>,
    is_folder: bool,
}

static mut BM_WIN_ID: u32 = 0;
static mut BM_TREE_ID: u32 = 0;
static mut BM_SELECTED: i32 = -1;
static mut BM_NODE_MAP: Option<Vec<NodeMapping>> = None;

fn node_map() -> &'static mut Vec<NodeMapping> {
    unsafe { BM_NODE_MAP.get_or_insert_with(Vec::new) }
}

fn selected_idx() -> i32 {
    unsafe { BM_SELECTED }
}

fn set_selected(idx: i32) {
    unsafe {
        BM_SELECTED = idx;
    }
}

// ═══════════════════════════════════════════════════════════
// Tree population
// ═══════════════════════════════════════════════════════════

fn populate_children(
    tree: &ui_lib::TreeView,
    parent_id: u32,
    items: &[BookmarkItem],
    path_prefix: &[usize],
    map: &mut Vec<NodeMapping>,
) {
    for (i, item) in items.iter().enumerate() {
        let label = if item.is_folder {
            item.title.clone()
        } else if item.url.is_empty() {
            item.title.clone()
        } else {
            anyos_std::format!("{} — {}", item.title, item.url)
        };
        let node_id = tree.add_child(parent_id, &label);
        let mut path = Vec::from(path_prefix);
        path.push(i);
        map.push(NodeMapping {
            path: path.clone(),
            is_folder: item.is_folder,
        });
        // Set favicon icon on the tree node if available.
        if !item.icon.is_empty() && item.icon_w > 0 {
            tree.set_node_icon(node_id, &item.icon, item.icon_w, item.icon_h);
        }
        if item.is_folder {
            tree.set_expanded(node_id, true);
            populate_children(tree, node_id, &item.children, &path, map);
        }
    }
}

fn refresh_tree() {
    let tree_id = unsafe { BM_TREE_ID };
    if tree_id == 0 {
        return;
    }
    let tree = ui_lib::TreeView::from_id(tree_id);
    tree.clear();

    let map = node_map();
    map.clear();

    // Dummy root entry (index 0 in map = "All Bookmarks" root node).
    let root_id = tree.add_root("All Bookmarks");
    map.push(NodeMapping {
        path: Vec::new(),
        is_folder: true,
    });

    let st = state();
    let roots_clone: Vec<BookmarkItem> = st.bookmarks.roots.clone();
    populate_children(&tree, root_id, &roots_clone, &[], map);

    tree.set_expanded(root_id, true);
    set_selected(-1);
}

// ═══════════════════════════════════════════════════════════
// Data helpers
// ═══════════════════════════════════════════════════════════

/// Resolve a path to a mutable reference to the parent's children vec
/// and the final index. Returns None if the path is empty (root level).
fn resolve_parent_and_index<'a>(
    roots: &'a mut Vec<BookmarkItem>,
    path: &[usize],
) -> Option<(&'a mut Vec<BookmarkItem>, usize)> {
    match path.split_first() {
        None => None,
        Some((&idx, rest)) if rest.is_empty() => {
            if idx >= roots.len() {
                None
            } else {
                Some((roots, idx))
            }
        }
        Some((&idx, rest)) => {
            let item = roots.get_mut(idx)?;
            resolve_parent_and_index(&mut item.children, rest)
        }
    }
}

/// Get the children vec that a path points into for insertion.
fn resolve_folder<'a>(
    roots: &'a mut Vec<BookmarkItem>,
    path: &[usize],
) -> &'a mut Vec<BookmarkItem> {
    match path.split_first() {
        None => roots,
        Some((&idx, rest)) => {
            if idx >= roots.len() {
                roots
            } else {
                let children = &mut roots[idx].children;
                resolve_folder(children, rest)
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════
// Add / Edit / Delete operations
// ═══════════════════════════════════════════════════════════

fn open_add_dialog(is_folder: bool) {
    let st = state();

    let title_text = if is_folder {
        "New Folder"
    } else {
        "Add Bookmark"
    };
    let win = ui_lib::Window::new(title_text, -1, -1, 440, if is_folder { 130 } else { 170 });

    let panel = ui_lib::View::new();
    panel.set_dock(ui_lib::DOCK_FILL);
    panel.set_padding(16, 16, 16, 16);
    panel.set_color(0xFF1E1E1E);
    win.add(&panel);

    let lbl_name = ui_lib::Label::new("Name:");
    lbl_name.set_position(0, 10);
    lbl_name.set_size(60, 22);
    lbl_name.set_text_color(0xFFCCCCCC);
    panel.add(&lbl_name);

    let name_field = ui_lib::TextField::new();
    name_field.set_position(65, 8);
    name_field.set_size(340, 24);
    if !is_folder {
        // Pre-fill with current page title.
        let tab = &st.tabs[st.active_tab];
        name_field.set_text(&tab.page_title);
    }
    panel.add(&name_field);

    let url_field = if !is_folder {
        let lbl_url = ui_lib::Label::new("URL:");
        lbl_url.set_position(0, 46);
        lbl_url.set_size(60, 22);
        lbl_url.set_text_color(0xFFCCCCCC);
        panel.add(&lbl_url);

        let uf = ui_lib::TextField::new();
        uf.set_position(65, 44);
        uf.set_size(340, 24);
        // Pre-fill with current page URL.
        uf.set_text(&st.tabs[st.active_tab].url_text);
        panel.add(&uf);
        Some(uf)
    } else {
        None
    };

    let btn_bar = ui_lib::View::new();
    btn_bar.set_dock(ui_lib::DOCK_BOTTOM);
    btn_bar.set_size(0, 44);
    btn_bar.set_color(0xFF252525);
    btn_bar.set_padding(8, 8, 8, 8);
    win.add(&btn_bar);

    let btn_ok = ui_lib::Button::new("Add");
    btn_ok.set_position(260, 4);
    btn_ok.set_size(80, 28);
    btn_bar.add(&btn_ok);

    let btn_cancel = ui_lib::Button::new("Cancel");
    btn_cancel.set_position(348, 4);
    btn_cancel.set_size(80, 28);
    btn_bar.add(&btn_cancel);

    let win_id = win.id();

    btn_ok.on_click(move |_| {
        let name = config::get_field_text(&name_field);
        if name.is_empty() {
            return;
        }
        let url = if let Some(ref uf) = url_field {
            config::get_field_text(uf)
        } else {
            String::new()
        };

        // Fetch favicon for bookmarks (not folders).
        let (icon, icon_w, icon_h) = if !is_folder && !url.is_empty() {
            config::fetch_favicon(&url).unwrap_or((Vec::new(), 0, 0))
        } else {
            (Vec::new(), 0, 0)
        };

        let item = BookmarkItem {
            title: name,
            url,
            is_folder,
            children: Vec::new(),
            icon,
            icon_w,
            icon_h,
        };

        // Insert into selected folder, or at root level.
        let st = state();
        let sel = selected_idx();
        let map = node_map();
        if sel > 0 && (sel as usize) < map.len() && map[sel as usize].is_folder {
            let path = map[sel as usize].path.clone();
            let folder = resolve_folder(&mut st.bookmarks.roots, &path);
            folder.push(item);
        } else {
            st.bookmarks.roots.push(item);
        }

        config::save(&st.config, &st.bookmarks);
        refresh_tree();
        ui_lib::Control::from_id(win_id).set_visible(false);
    });

    btn_cancel.on_click(move |_| {
        ui_lib::Control::from_id(win_id).set_visible(false);
    });

    win.on_close(|_| {});
}

fn open_edit_dialog() {
    let sel = selected_idx();
    if sel <= 0 {
        return;
    }
    let map = node_map();
    if (sel as usize) >= map.len() {
        return;
    }
    let path = map[sel as usize].path.clone();
    let is_folder = map[sel as usize].is_folder;

    let st = state();
    let (title, url) = {
        if let Some((parent, idx)) = resolve_parent_and_index(&mut st.bookmarks.roots, &path) {
            (parent[idx].title.clone(), parent[idx].url.clone())
        } else {
            return;
        }
    };

    let win = ui_lib::Window::new(
        "Edit Bookmark",
        -1,
        -1,
        440,
        if is_folder { 130 } else { 170 },
    );

    let panel = ui_lib::View::new();
    panel.set_dock(ui_lib::DOCK_FILL);
    panel.set_padding(16, 16, 16, 16);
    panel.set_color(0xFF1E1E1E);
    win.add(&panel);

    let lbl_name = ui_lib::Label::new("Name:");
    lbl_name.set_position(0, 10);
    lbl_name.set_size(60, 22);
    lbl_name.set_text_color(0xFFCCCCCC);
    panel.add(&lbl_name);

    let name_field = ui_lib::TextField::new();
    name_field.set_position(65, 8);
    name_field.set_size(340, 24);
    name_field.set_text(&title);
    panel.add(&name_field);

    let url_field = if !is_folder {
        let lbl_url = ui_lib::Label::new("URL:");
        lbl_url.set_position(0, 46);
        lbl_url.set_size(60, 22);
        lbl_url.set_text_color(0xFFCCCCCC);
        panel.add(&lbl_url);

        let uf = ui_lib::TextField::new();
        uf.set_position(65, 44);
        uf.set_size(340, 24);
        uf.set_text(&url);
        panel.add(&uf);
        Some(uf)
    } else {
        None
    };

    let btn_bar = ui_lib::View::new();
    btn_bar.set_dock(ui_lib::DOCK_BOTTOM);
    btn_bar.set_size(0, 44);
    btn_bar.set_color(0xFF252525);
    btn_bar.set_padding(8, 8, 8, 8);
    win.add(&btn_bar);

    let btn_ok = ui_lib::Button::new("Save");
    btn_ok.set_position(260, 4);
    btn_ok.set_size(80, 28);
    btn_bar.add(&btn_ok);

    let btn_cancel = ui_lib::Button::new("Cancel");
    btn_cancel.set_position(348, 4);
    btn_cancel.set_size(80, 28);
    btn_bar.add(&btn_cancel);

    let win_id = win.id();
    let path_clone = path.clone();

    btn_ok.on_click(move |_| {
        let new_name = config::get_field_text(&name_field);
        if new_name.is_empty() {
            return;
        }
        let new_url = if let Some(ref uf) = url_field {
            config::get_field_text(uf)
        } else {
            String::new()
        };

        let st = state();
        if let Some((parent, idx)) = resolve_parent_and_index(&mut st.bookmarks.roots, &path_clone)
        {
            parent[idx].title = new_name;
            if !parent[idx].is_folder {
                parent[idx].url = new_url;
            }
        }

        config::save(&st.config, &st.bookmarks);
        refresh_tree();
        ui_lib::Control::from_id(win_id).set_visible(false);
    });

    btn_cancel.on_click(move |_| {
        ui_lib::Control::from_id(win_id).set_visible(false);
    });

    win.on_close(|_| {});
}

fn delete_selected() {
    let sel = selected_idx();
    if sel <= 0 {
        return;
    }
    let map = node_map();
    if (sel as usize) >= map.len() {
        return;
    }
    let path = map[sel as usize].path.clone();

    let st = state();
    if let Some((parent, idx)) = resolve_parent_and_index(&mut st.bookmarks.roots, &path) {
        if idx < parent.len() {
            parent.remove(idx);
        }
    }

    config::save(&st.config, &st.bookmarks);
    refresh_tree();
}

fn open_selected() {
    let sel = selected_idx();
    if sel <= 0 {
        return;
    }
    let map = node_map();
    if (sel as usize) >= map.len() || map[sel as usize].is_folder {
        return;
    }
    let path = map[sel as usize].path.clone();

    let st = state();
    if let Some((parent, idx)) = resolve_parent_and_index(&mut st.bookmarks.roots, &path) {
        let url = parent[idx].url.clone();
        if !url.is_empty() {
            st.tabs[st.active_tab].url_text = url.clone();
            st.url_field.set_text(&url);
            tab::navigate(&url);
        }
    }
}

fn open_selected_in_new_tab() {
    let sel = selected_idx();
    if sel <= 0 {
        return;
    }
    let map = node_map();
    if (sel as usize) >= map.len() || map[sel as usize].is_folder {
        return;
    }
    let path = map[sel as usize].path.clone();

    let st = state();
    if let Some((parent, idx)) = resolve_parent_and_index(&mut st.bookmarks.roots, &path) {
        let url = parent[idx].url.clone();
        if !url.is_empty() {
            ui::add_tab();
            let st = state();
            st.tabs[st.active_tab].url_text = url.clone();
            st.url_field.set_text(&url);
            tab::navigate(&url);
        }
    }
}

// ═══════════════════════════════════════════════════════════
// Public API
// ═══════════════════════════════════════════════════════════

/// Add the current page as a bookmark (Ctrl+D).
pub fn add_current_page() {
    let st = state();
    let tab = &st.tabs[st.active_tab];
    let title = if tab.page_title.is_empty() {
        tab.url_text.clone()
    } else {
        tab.page_title.clone()
    };
    let url = tab.url_text.clone();
    if url.is_empty() {
        return;
    }

    let (icon, icon_w, icon_h) = config::fetch_favicon(&url).unwrap_or((Vec::new(), 0, 0));

    st.bookmarks.roots.push(BookmarkItem {
        title,
        url,
        is_folder: false,
        children: Vec::new(),
        icon,
        icon_w,
        icon_h,
    });
    config::save(&st.config, &st.bookmarks);

    // Refresh tree if bookmark manager is open.
    refresh_tree();
}

/// Open the bookmark manager window.
pub fn open_bookmark_manager() {
    let bm_win_id = unsafe { BM_WIN_ID };
    if bm_win_id != 0 {
        // Already open — bring to front.
        let win = ui_lib::Control::from_id(bm_win_id);
        win.set_visible(true);
        refresh_tree();
        return;
    }

    let win = ui_lib::Window::new("Bookmarks", -1, -1, 650, 480);

    // ── Toolbar ──
    let toolbar = ui_lib::View::new();
    toolbar.set_dock(ui_lib::DOCK_TOP);
    toolbar.set_size(0, 36);
    toolbar.set_color(0xFF2A2A2C);
    toolbar.set_padding(4, 4, 4, 4);
    win.add(&toolbar);

    let btn_add = ui_lib::Button::new("Add Bookmark");
    btn_add.set_position(4, 2);
    btn_add.set_size(110, 28);
    toolbar.add(&btn_add);

    let btn_folder = ui_lib::Button::new("New Folder");
    btn_folder.set_position(120, 2);
    btn_folder.set_size(90, 28);
    toolbar.add(&btn_folder);

    let btn_edit = ui_lib::Button::new("Edit");
    btn_edit.set_position(216, 2);
    btn_edit.set_size(60, 28);
    toolbar.add(&btn_edit);

    let btn_delete = ui_lib::Button::new("Delete");
    btn_delete.set_position(282, 2);
    btn_delete.set_size(60, 28);
    toolbar.add(&btn_delete);

    // ── TreeView ──
    let tree = ui_lib::TreeView::new(650, 400);
    tree.set_dock(ui_lib::DOCK_FILL);
    win.add(&tree);

    // ── Context menu ──
    let ctx = ui_lib::ContextMenu::new("Open|Open in New Tab|-|Edit|Delete|-|New Folder");
    tree.set_context_menu(&ctx);

    // ── Status bar ──
    let status = ui_lib::Label::new("");
    status.set_dock(ui_lib::DOCK_BOTTOM);
    status.set_size(0, 24);
    status.set_color(0xFF252525);
    status.set_text_color(0xFF969696);
    status.set_font_size(12);
    status.set_padding(8, 4, 0, 0);
    win.add(&status);

    let win_id = win.id();
    let tree_id = tree.id();
    let status_id = status.id();

    unsafe {
        BM_WIN_ID = win_id;
        BM_TREE_ID = tree_id;
    }

    // Populate tree.
    refresh_tree();

    // ── Callbacks ──
    tree.on_selection_changed(move |e| {
        set_selected(e.index as i32);
        // Update status bar with URL of selected bookmark.
        let map = node_map();
        let idx = e.index as usize;
        if idx < map.len() && !map[idx].is_folder {
            let path = map[idx].path.clone();
            let st = state();
            if let Some((parent, i)) = resolve_parent_and_index(&mut st.bookmarks.roots, &path) {
                let lbl = ui_lib::Control::from_id(status_id);
                lbl.set_text(&parent[i].url);
            }
        } else {
            let lbl = ui_lib::Control::from_id(status_id);
            lbl.set_text("");
        }
    });

    tree.on_enter(move |_| {
        open_selected();
    });

    ctx.on_item_click(|e| {
        match e.index {
            0 => open_selected(),            // Open
            1 => open_selected_in_new_tab(), // Open in New Tab
            // 2 = separator
            3 => open_edit_dialog(), // Edit
            4 => delete_selected(),  // Delete
            // 5 = separator
            6 => open_add_dialog(true), // New Folder
            _ => {}
        }
    });

    btn_add.on_click(|_| {
        open_add_dialog(false);
    });
    btn_folder.on_click(|_| {
        open_add_dialog(true);
    });
    btn_edit.on_click(|_| {
        open_edit_dialog();
    });
    btn_delete.on_click(|_| {
        delete_selected();
    });

    win.on_close(move |_| unsafe {
        BM_WIN_ID = 0;
        BM_TREE_ID = 0;
    });
}
