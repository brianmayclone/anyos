//! anyMail — Thunderbird-like email client for anyOS.
//!
//! Supports IMAP4rev1, POP3, and SMTP with TLS/STARTTLS.
//! Layout: Toolbar | Filter Bar | Folder Tree | Message Grid | Preview | Status Bar

#![no_std]
#![no_main]

mod mail;
mod protocol;
mod storage;
mod sync_worker;

use anyos_std::format;
use anyos_std::String;
use anyos_std::Vec;
use anyos_std::vec;

use libanyui_client as anyui;
use libanyui_client::IconType;
use libanyui_client::Widget;

use crate::mail::message::*;
use crate::mail::rfc2822::EmailAddress;
use crate::storage::account::*;
use crate::storage::contacts::{AddressBook, Contact};
use crate::storage::maildir;
use crate::protocol::imap::{ImapClient, ImapFolder, SpecialUse};
use crate::protocol::smtp::SmtpClient;
use crate::sync_worker::SyncPhase;

anyos_std::entry!(main);

// ═══════════════════════════════════════════════════════════════════════════
// Constants
// ═══════════════════════════════════════════════════════════════════════════

const CHECK_INTERVAL_MS: u32 = 300_000; // 5 minutes
const NUM_GRID_COLS: usize = 5;

// Colors
const COL_BG: u32         = 0xFF1E1E2E;
const COL_TOOLBAR: u32    = 0xFF252536;
const COL_STATUS: u32     = 0xFF007ACC;
const COL_SIDEBAR: u32    = 0xFF252536;
const COL_GRID_BG: u32    = 0xFF1E1E2E;
const COL_PREVIEW_BG: u32 = 0xFF1E1E2E;
const COL_UNREAD: u32     = 0xFFFFFFFF;
const COL_READ: u32       = 0xFF888888;
const COL_FLAGGED: u32    = 0xFFFFD700;
const COL_DELETED: u32    = 0xFF555555;
const COL_JUNK: u32       = 0xFF996633;
const COL_WHITE: u32      = 0xFFFFFFFF;
const COL_DIM: u32        = 0xFFAAAAAA;

// ═══════════════════════════════════════════════════════════════════════════
// Folder info stored per tree node
// ═══════════════════════════════════════════════════════════════════════════

struct FolderInfo {
    account_idx: usize,
    folder_name: String,
    special_use: SpecialUse,
    node_id: u32,
}

// ═══════════════════════════════════════════════════════════════════════════
// App State
// ═══════════════════════════════════════════════════════════════════════════

struct AppState {
    // Paths
    base_dir: String,
    config_path: String,

    // Config
    config: MailConfig,
    address_book: AddressBook,

    // UI handles — main window
    win: anyui::Window,
    toolbar: anyui::Toolbar,
    filter_bar: anyui::View,
    search_field: anyui::SearchField,
    filter_unread: anyui::Button,
    filter_starred: anyui::Button,
    filter_attach: anyui::Button,
    folder_tree: anyui::TreeView,
    msg_grid: anyui::DataGrid,
    preview_header: anyui::Label,
    preview_body: anyui::TextEditor,
    status_label: anyui::Label,

    // UI handles — buttons
    btn_new: anyui::IconButton,
    btn_reply: anyui::IconButton,
    btn_reply_all: anyui::IconButton,
    btn_forward: anyui::IconButton,
    btn_delete: anyui::IconButton,
    btn_junk: anyui::IconButton,
    btn_archive: anyui::IconButton,
    btn_getmail: anyui::IconButton,

    // Folder tree data
    folders: Vec<FolderInfo>,

    // Current message list (filtered)
    messages: Vec<MessageSummary>,
    all_messages: Vec<MessageSummary>,

    // Current selection
    current_account: usize,
    current_folder: String,
    selected_msg_idx: Option<usize>,
    current_full_msg: Option<FullMessage>,

    // Filters
    filter_text: String,
    filter_unread_on: bool,
    filter_starred_on: bool,
    filter_attach_on: bool,

    // Timer
    check_timer_id: u32,
    sync_poll_timer_id: u32,

    // Async sync / lazy loading
    folder_total_count: u32,
    folder_loaded_count: u32,
    has_more_messages: bool,
    loading_more: bool,
}

anyos_std::global_app_state!(AppState);

// ═══════════════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════════════

fn home_dir() -> String {
    let mut buf = [0u8; 256];
    let len = anyos_std::env::get("HOME", &mut buf);
    if len == u32::MAX || len == 0 {
        return String::from("/tmp");
    }
    String::from(core::str::from_utf8(&buf[..len as usize]).unwrap_or("/tmp"))
}

fn now_string() -> String {
    let mut buf = [0u8; 8];
    anyos_std::sys::time(&mut buf);
    let year = buf[0] as u16 | ((buf[1] as u16) << 8);
    let month = buf[2];
    let day = buf[3];
    let hour = buf[4];
    let min = buf[5];
    format!("{:04}-{:02}-{:02} {:02}:{:02}", year, month, day, hour, min)
}

fn set_status(text: &str) {
    app().status_label.set_text(text);
}

// ═══════════════════════════════════════════════════════════════════════════
// Entry Point
// ═══════════════════════════════════════════════════════════════════════════

fn main() {
    if !anyui::init() { return; }
    anyos_std::i18n::init();

    let home = home_dir();
    let base_dir = format!("{}/.anymail", home);
    let config_path = format!("{}/accounts.json", base_dir);
    let contacts_path = format!("{}/contacts.json", base_dir);

    // Ensure base directory exists
    anyos_std::fs::mkdir(&base_dir);

    // Load configuration
    let config = MailConfig::load(&config_path);
    let address_book = AddressBook::load(&contacts_path);

    // ── Window ─────────────────────────────────────────────────────────
    let t = anyos_std::i18n::t;
    let win = anyui::Window::new("anyMail", -1, -1, 1100, 700);

    // ── Toolbar (DOCK_TOP) ─────────────────────────────────────────────
    let toolbar = anyui::Toolbar::new();
    toolbar.set_dock(anyui::DOCK_TOP);
    toolbar.set_size(1100, 46);
    toolbar.set_color(COL_TOOLBAR);
    toolbar.set_padding(4, 4, 4, 4);

    // Compose/Reply buttons with icons (34x34 with tooltips, no inline text)
    let btn_new = toolbar.add_icon_button("");
    btn_new.set_size(34, 34);
    btn_new.set_icon(anyui::ICON_NEW_FILE);
    btn_new.set_tooltip(t("New"));

    let btn_reply = toolbar.add_icon_button("");
    btn_reply.set_size(34, 34);
    btn_reply.set_system_icon("arrow-back-up", anyui::IconType::Outline, 0xFFCCCCCC, 24);
    btn_reply.set_tooltip(t("Reply"));

    let btn_reply_all = toolbar.add_icon_button("");
    btn_reply_all.set_size(34, 34);
    btn_reply_all.set_system_icon("corner-up-left", anyui::IconType::Outline, 0xFFCCCCCC, 24);
    btn_reply_all.set_tooltip(t("Reply All"));

    let btn_forward = toolbar.add_icon_button("");
    btn_forward.set_size(34, 34);
    btn_forward.set_system_icon("mail-forward", anyui::IconType::Outline, 0xFFCCCCCC, 24);
    btn_forward.set_tooltip(t("Forward"));

    toolbar.add_separator();

    // Message management buttons
    let btn_archive = toolbar.add_icon_button("");
    btn_archive.set_size(34, 34);
    btn_archive.set_system_icon("archive", anyui::IconType::Outline, 0xFFCCCCCC, 24);
    btn_archive.set_tooltip(t("Archive"));

    let btn_junk = toolbar.add_icon_button("");
    btn_junk.set_size(34, 34);
    btn_junk.set_system_icon("mail-x", anyui::IconType::Outline, 0xFFCCCCCC, 24);
    btn_junk.set_tooltip(t("Junk"));

    let btn_delete = toolbar.add_icon_button("");
    btn_delete.set_size(34, 34);
    btn_delete.set_system_icon("trash", anyui::IconType::Outline, 0xFFCCCCCC, 24);
    btn_delete.set_tooltip(t("Delete"));

    toolbar.add_separator();

    // Account/Sync buttons
    let btn_getmail = toolbar.add_icon_button("");
    btn_getmail.set_size(34, 34);
    btn_getmail.set_system_icon("refresh", anyui::IconType::Outline, 0xFFCCCCCC, 24);
    btn_getmail.set_tooltip(t("Get Mail"));

    let btn_accounts = toolbar.add_icon_button("");
    btn_accounts.set_size(34, 34);
    btn_accounts.set_system_icon("settings", anyui::IconType::Outline, 0xFFCCCCCC, 24);
    btn_accounts.set_tooltip(t("Accounts"));

    let btn_contacts = toolbar.add_icon_button("");
    btn_contacts.set_size(34, 34);
    btn_contacts.set_system_icon("address-book", anyui::IconType::Outline, 0xFFCCCCCC, 24);
    btn_contacts.set_tooltip(t("Contacts"));

    win.add(&toolbar);

    // ── Filter Bar (DOCK_TOP) ──────────────────────────────────────────
    let filter_bar = anyui::View::new();
    filter_bar.set_dock(anyui::DOCK_TOP);
    filter_bar.set_size(1100, 30);
    filter_bar.set_color(0xFF2D2D3D);
    filter_bar.set_padding(4, 2, 4, 2);

    let search_field = anyui::SearchField::new();
    search_field.set_placeholder(t("Search messages..."));
    search_field.set_position(4, 3);
    search_field.set_size(300, 24);
    filter_bar.add(&search_field);

    let filter_unread = anyui::Button::new(t("Unread"));
    filter_unread.set_position(316, 3);
    filter_unread.set_size(60, 24);
    filter_bar.add(&filter_unread);

    let filter_starred = anyui::Button::new(t("Starred"));
    filter_starred.set_position(382, 3);
    filter_starred.set_size(65, 24);
    filter_bar.add(&filter_starred);

    let filter_attach = anyui::Button::new(t("Attach"));
    filter_attach.set_position(453, 3);
    filter_attach.set_size(60, 24);
    filter_bar.add(&filter_attach);

    win.add(&filter_bar);

    // ── Status Bar (DOCK_BOTTOM) ───────────────────────────────────────
    let status_bar = anyui::View::new();
    status_bar.set_dock(anyui::DOCK_BOTTOM);
    status_bar.set_size(1100, 24);
    status_bar.set_color(COL_STATUS);
    status_bar.set_padding(8, 2, 8, 2);

    let status_label = anyui::Label::new(t("Ready"));
    status_label.set_dock(anyui::DOCK_FILL);
    status_label.set_text_color(COL_WHITE);
    status_label.set_font_size(12);
    status_bar.add(&status_label);

    win.add(&status_bar);

    // ── Main Split (DOCK_FILL) ─────────────────────────────────────────
    // Horizontal: folder tree (20%) | content (80%)
    let main_split = anyui::SplitView::new();
    main_split.set_dock(anyui::DOCK_FILL);
    main_split.set_split_ratio(20);

    // Left panel: Folder tree
    let folder_tree = anyui::TreeView::new(200, 600);
    folder_tree.set_color(COL_SIDEBAR);
    folder_tree.set_text_color(COL_WHITE);
    folder_tree.set_row_height(22);
    main_split.add(&folder_tree);

    // Right panel: vertical split (message grid 40% | preview 60%)
    let content_split = anyui::SplitView::new();
    content_split.set_orientation(anyui::ORIENTATION_VERTICAL);
    content_split.set_split_ratio(40);

    // Message grid (top of right)
    let msg_grid = anyui::DataGrid::new(800, 300);
    msg_grid.set_columns(&[
        anyui::ColumnDef::new("").width(30),            // Flags (star/unread)
        anyui::ColumnDef::new(t("From")).width(180),
        anyui::ColumnDef::new(t("Subject")).width(350),
        anyui::ColumnDef::new(t("Date")).width(130),
        anyui::ColumnDef::new(t("Size")).width(70).align(anyui::ALIGN_RIGHT),
    ]);
    msg_grid.set_row_height(22);
    msg_grid.set_header_height(24);
    msg_grid.set_color(COL_GRID_BG);
    msg_grid.set_text_color(COL_WHITE);
    content_split.add(&msg_grid);

    // Preview pane (bottom of right)
    let preview_panel = anyui::View::new();

    // Preview header (from/to/subject/date)
    let preview_header = anyui::Label::new("");
    preview_header.set_dock(anyui::DOCK_TOP);
    preview_header.set_size(800, 60);
    preview_header.set_color(0xFF2A2A3A);
    preview_header.set_text_color(COL_DIM);
    preview_header.set_font_size(12);
    preview_header.set_padding(8, 4, 8, 4);
    preview_panel.add(&preview_header);

    // Preview body
    let preview_body = anyui::TextEditor::new(800, 350);
    preview_body.set_dock(anyui::DOCK_FILL);
    preview_body.set_color(COL_PREVIEW_BG);
    preview_body.set_text_color(COL_WHITE);
    preview_body.set_show_line_numbers(false);
    preview_panel.add(&preview_body);

    content_split.add(&preview_panel);
    main_split.add(&content_split);

    win.add(&main_split);

    // ── Context menu for message grid ──────────────────────────────────
    let ctx_items = format!(
        "{}|{}|{}|{}|{}|{}|{}|{}",
        t("Mark as Read"), t("Mark as Unread"), t("Toggle Star"), t("Mark as Junk"),
        t("Move to Trash"), t("Move to Archive"), t("Copy to Folder..."), t("Delete Permanently")
    );
    let ctx_menu = anyui::ContextMenu::new(&ctx_items);
    msg_grid.set_context_menu(&ctx_menu);

    // ── Initialize state ───────────────────────────────────────────────
    let has_accounts = !config.accounts.is_empty();

    unsafe {
        APP = Some(AppState {
            base_dir: base_dir.clone(),
            config_path: config_path.clone(),
            config,
            address_book,
            win: win.clone(),
            toolbar: toolbar.clone(),
            filter_bar: filter_bar.clone(),
            search_field: search_field.clone(),
            filter_unread: filter_unread.clone(),
            filter_starred: filter_starred.clone(),
            filter_attach: filter_attach.clone(),
            folder_tree: folder_tree.clone(),
            msg_grid: msg_grid.clone(),
            preview_header: preview_header.clone(),
            preview_body: preview_body.clone(),
            status_label: status_label.clone(),
            btn_new: btn_new.clone(),
            btn_reply: btn_reply.clone(),
            btn_reply_all: btn_reply_all.clone(),
            btn_forward: btn_forward.clone(),
            btn_delete: btn_delete.clone(),
            btn_junk: btn_junk.clone(),
            btn_archive: btn_archive.clone(),
            btn_getmail: btn_getmail.clone(),
            folders: Vec::new(),
            messages: Vec::new(),
            all_messages: Vec::new(),
            current_account: 0,
            current_folder: String::from("INBOX"),
            selected_msg_idx: None,
            current_full_msg: None,
            filter_text: String::new(),
            filter_unread_on: false,
            filter_starred_on: false,
            filter_attach_on: false,
            check_timer_id: 0,
            sync_poll_timer_id: 0,
            folder_total_count: 0,
            folder_loaded_count: 0,
            has_more_messages: false,
            loading_more: false,
        });
    }

    // ── Menu bar ─────────────────────────────────────────────────────────
    let mut mb = anyui::MenuBarBuilder::new()
        .menu("File")
            .item(1, "Check Mail", 0)
            .separator()
            .item(2, "Quit", 0)
        .end_menu()
        .menu("Message")
            .item(10, "Compose", 0)
            .item(11, "Reply", 0)
            .item(12, "Forward", 0)
            .separator()
            .item(13, "Delete", 0)
        .end_menu()
        .menu("Tools")
            .item(20, "Account Settings", 0)
            .item(21, "Address Book", 0)
        .end_menu();
    let menu_data = mb.build();
    let menu_bar = anyui::MenuBar::set(win.id(), menu_data);
    menu_bar.on_item(|e| {
        match e.item_id {
            1  => on_get_mail(),
            2  => anyui::quit(),
            10 => open_compose(ComposeMode::New),
            11 => open_compose(ComposeMode::Reply),
            12 => open_compose(ComposeMode::Forward),
            13 => on_delete(),
            20 => open_account_setup(),
            21 => open_contacts(),
            _  => {}
        }
    });

    // ── Populate folder tree ───────────────────────────────────────────
    populate_folder_tree();

    // ── Wire up events ─────────────────────────────────────────────────
    btn_new.on_click(|_| { open_compose(ComposeMode::New); });
    btn_reply.on_click(|_| { open_compose(ComposeMode::Reply); });
    btn_reply_all.on_click(|_| { open_compose(ComposeMode::ReplyAll); });
    btn_forward.on_click(|_| { open_compose(ComposeMode::Forward); });
    btn_delete.on_click(|_| { on_delete(); });
    btn_junk.on_click(|_| { on_mark_junk(); });
    btn_archive.on_click(|_| { on_archive(); });
    btn_getmail.on_click(|_| { on_get_mail(); });
    btn_accounts.on_click(|_| { open_account_setup(); });
    btn_contacts.on_click(|_| { open_contacts(); });

    search_field.on_text_changed(|e| {
        let mut buf = [0u8; 256];
        let len = app().search_field.get_text(&mut buf);
        app().filter_text = String::from(core::str::from_utf8(&buf[..len as usize]).unwrap_or(""));
        apply_filters();
    });

    filter_unread.on_click(|_| {
        app().filter_unread_on = !app().filter_unread_on;
        apply_filters();
    });

    filter_starred.on_click(|_| {
        app().filter_starred_on = !app().filter_starred_on;
        apply_filters();
    });

    filter_attach.on_click(|_| {
        app().filter_attach_on = !app().filter_attach_on;
        apply_filters();
    });

    folder_tree.on_selection_changed(|e| {
        on_folder_selected(e.index);
    });

    msg_grid.on_selection_changed(|_| {
        let row = app().msg_grid.selected_row();
        on_message_selected(row);
    });

    msg_grid.on_submit(|_| {
        // Double-click: open full message in compose (for drafts) or viewer
        if let Some(idx) = app().selected_msg_idx {
            if idx < app().messages.len() && app().messages[idx].is_draft() {
                open_compose(ComposeMode::Draft);
            }
        }
    });

    ctx_menu.on_item_click(|e| {
        on_context_action(e.index);
    });

    win.on_key_down(|e| {
        if e.ctrl() {
            let c = e.char_code;
            if c == b'n' as u32 || c == b'N' as u32 {
                open_compose(ComposeMode::New);
            } else if c == b'r' as u32 || c == b'R' as u32 {
                if e.shift() {
                    open_compose(ComposeMode::ReplyAll);
                } else {
                    open_compose(ComposeMode::Reply);
                }
            } else if c == b'f' as u32 || c == b'F' as u32 {
                app().search_field.focus();
            } else if c == b'm' as u32 || c == b'M' as u32 {
                on_get_mail();
            }
        } else {
            let k = e.keycode;
            if k == anyui::KEY_DELETE {
                on_delete();
            } else if k == anyui::KEY_F5 {
                on_get_mail();
            } else if k == anyui::KEY_F2 {
                open_account_setup();
            }
        }
    });

    win.on_close(|_| { anyui::quit(); });

    // ── Start poll timer for async sync (500ms) ────────────────────────
    let sync_poll_timer = anyui::set_timer(500, || {
        poll_sync_state();
    });
    app().sync_poll_timer_id = sync_poll_timer;

    // ── Start periodic mail check timer ──────────────────────────────
    if has_accounts {
        let timer_id = anyui::set_timer(CHECK_INTERVAL_MS, || {
            on_get_mail();
        });
        app().check_timer_id = timer_id;

        // Initial check if configured (now non-blocking)
        if app().config.check_on_startup {
            on_get_mail();
        }
    }

    set_status(t("Ready"));

    anyui::run();
}

// ═══════════════════════════════════════════════════════════════════════════
// Folder Tree
// ═══════════════════════════════════════════════════════════════════════════

fn populate_folder_tree() {
    let a = app();
    a.folder_tree.clear();
    a.folders.clear();

    if a.config.accounts.is_empty() {
        let root = a.folder_tree.add_root(anyos_std::i18n::t("No accounts configured"));
        a.folder_tree.set_node_style(root, 0);
        return;
    }

    for (acct_idx, account) in a.config.accounts.iter().enumerate() {
        let acct_label = if account.display_name.is_empty() {
            account.email.clone()
        } else {
            account.display_name.clone()
        };
        let root = a.folder_tree.add_root(&acct_label);
        a.folder_tree.set_node_style(root, 1); // bold
        a.folder_tree.set_expanded(root, true);

        // Ensure local dirs
        maildir::ensure_dirs(&a.base_dir, &account.id);

        // Default folders
        let default_folders = ["INBOX", "Sent", "Drafts", "Trash", "Spam"];
        let special_uses = [
            SpecialUse::Inbox, SpecialUse::Sent, SpecialUse::Drafts,
            SpecialUse::Trash, SpecialUse::Spam,
        ];

        for (i, folder_name) in default_folders.iter().enumerate() {
            let node_id = a.folder_tree.add_child(root, folder_name);
            a.folders.push(FolderInfo {
                account_idx: acct_idx,
                folder_name: String::from(*folder_name),
                special_use: special_uses[i],
                node_id,
            });
        }
    }
}

fn on_folder_selected(node_idx: u32) {
    // Find the folder info matching this tree node
    let a = app();
    for fi in &a.folders {
        if fi.node_id == node_idx {
            let acct_idx = fi.account_idx;
            let folder_name = fi.folder_name.clone();
            a.current_account = acct_idx;
            a.current_folder = folder_name.clone();
            a.has_more_messages = false;
            a.loading_more = false;
            // Load from local cache immediately
            load_folder_messages();
            // Then trigger background fetch for fresh data
            trigger_folder_sync(acct_idx, &folder_name);
            return;
        }
    }
}

fn load_folder_messages() {
    let a = app();
    if a.current_account >= a.config.accounts.len() { return; }

    let account = &a.config.accounts[a.current_account];
    let idx_path = maildir::index_path(&a.base_dir, &account.id, &a.current_folder);
    a.all_messages = maildir::load_index(&idx_path);
    a.selected_msg_idx = None;
    a.current_full_msg = None;

    // Clear preview
    a.preview_header.set_text("");
    a.preview_body.set_text("");

    apply_filters();

    let total = a.all_messages.len();
    let unread = a.all_messages.iter().filter(|m| !m.is_seen()).count();
    let t = anyos_std::i18n::t;
    set_status(&format!("{} - {} {}, {} {}", a.current_folder, total, t("messages"), unread, t("unread")));
}

// ═══════════════════════════════════════════════════════════════════════════
// Filtering & Grid Display
// ═══════════════════════════════════════════════════════════════════════════

fn apply_filters() {
    let a = app();
    let query = a.filter_text.clone();
    let query_lower = to_lower(&query);
    let show_unread = a.filter_unread_on;
    let show_starred = a.filter_starred_on;
    let show_attach = a.filter_attach_on;

    a.messages = a.all_messages.iter().filter(|m| {
        // Text search
        if !query_lower.is_empty() {
            let from_match = to_lower(m.from.display_short()).contains(&query_lower);
            let subj_match = to_lower(&m.subject).contains(&query_lower);
            let prev_match = to_lower(&m.preview).contains(&query_lower);
            if !from_match && !subj_match && !prev_match {
                return false;
            }
        }
        // Filter toggles
        if show_unread && m.is_seen() { return false; }
        if show_starred && !m.is_flagged() { return false; }
        if show_attach && !m.has_attachment() { return false; }
        true
    }).cloned().collect();

    refresh_grid();
}

fn refresh_grid() {
    let a = app();
    let count = a.messages.len();
    a.msg_grid.set_row_count(count as u32);

    if count == 0 {
        return;
    }

    // Build cell data and colors
    let mut text_colors = vec![COL_READ; count * NUM_GRID_COLS];
    let mut bg_colors = vec![0u32; count * NUM_GRID_COLS];

    for (row, msg) in a.messages.iter().enumerate() {
        // Flags column
        let flags_str = {
            let mut s = String::new();
            if msg.is_flagged() { s.push('*'); }
            if !msg.is_seen() { s.push('!'); }
            if msg.has_attachment() { s.push('@'); }
            s
        };
        a.msg_grid.set_cell(row as u32, 0, &flags_str);

        // From
        a.msg_grid.set_cell(row as u32, 1, msg.from.display_short());

        // Subject
        a.msg_grid.set_cell(row as u32, 2, &msg.subject);

        // Date
        let date_short = crate::mail::rfc2822::format_date_short(&msg.date);
        a.msg_grid.set_cell(row as u32, 3, &date_short);

        // Size
        let size_str = crate::mail::rfc2822::format_size(msg.size);
        a.msg_grid.set_cell(row as u32, 4, &size_str);

        // Determine text color based on message state
        let color = if msg.is_deleted() {
            COL_DELETED
        } else if msg.is_junk() {
            COL_JUNK
        } else if msg.is_flagged() {
            COL_FLAGGED
        } else if !msg.is_seen() {
            COL_UNREAD
        } else {
            COL_READ
        };

        for col in 0..NUM_GRID_COLS {
            text_colors[row * NUM_GRID_COLS + col] = color;
        }
    }

    a.msg_grid.set_cell_colors(&text_colors);
    a.msg_grid.set_cell_bg_colors(&bg_colors);
}

// ═══════════════════════════════════════════════════════════════════════════
// Message Selection & Preview
// ═══════════════════════════════════════════════════════════════════════════

fn on_message_selected(row: u32) {
    let a = app();
    let idx = row as usize;
    if idx >= a.messages.len() {
        a.selected_msg_idx = None;
        a.preview_header.set_text("");
        a.preview_body.set_text("");
        return;
    }

    a.selected_msg_idx = Some(idx);
    let msg = &a.messages[idx];

    // Show preview header
    let t = anyos_std::i18n::t;
    let header = format!(
        "{}: {}\n{}: {}\n{}: {}\n{}: {}",
        t("From"), msg.from.to_header_string(),
        t("To"), format_to_list(&msg.to),
        t("Subject"), msg.subject,
        t("Date"), crate::mail::rfc2822::format_date_short(&msg.date)
    );
    a.preview_header.set_text(&header);

    // Try to load full message for body preview
    if a.current_account < a.config.accounts.len() {
        let account = &a.config.accounts[a.current_account];
        let msg_path = maildir::message_path(&a.base_dir, &account.id, &a.current_folder, msg.uid);
        if let Some(raw) = maildir::load_message(&msg_path) {
            let full = crate::mail::mime::parse_message(&raw);
            // Show text body in preview
            if !full.text_body.is_empty() {
                a.preview_body.set_text(&full.text_body);
            } else if !full.html_body.is_empty() {
                // Strip HTML tags for text preview
                a.preview_body.set_text(&strip_html(&full.html_body));
            } else {
                a.preview_body.set_text(anyos_std::i18n::t("(No message body)"));
            }

            // Show attachment count
            if !full.attachments.is_empty() {
                let att_info = format!("{} {}", full.attachments.len(), t("attachment(s)"));
                let current_header = format!("{}\n{}: {}", header, t("Attachments"), att_info);
                a.preview_header.set_text(&current_header);
            }

            a.current_full_msg = Some(full);
        } else {
            a.preview_body.set_text(&msg.preview);
            a.current_full_msg = None;
        }
    }

    // Mark as read if not already
    if !msg.is_seen() {
        mark_message_seen(idx);
    }
}

fn mark_message_seen(idx: usize) {
    let a = app();
    if idx >= a.messages.len() { return; }

    a.messages[idx].flags |= FLAG_SEEN;

    // Also update in all_messages
    let uid = a.messages[idx].uid;
    for m in &mut a.all_messages {
        if m.uid == uid {
            m.flags |= FLAG_SEEN;
            break;
        }
    }

    // Save index
    save_current_index();
    refresh_grid();
}

fn save_current_index() {
    let a = app();
    if a.current_account >= a.config.accounts.len() { return; }
    let account = &a.config.accounts[a.current_account];
    let idx_path = maildir::index_path(&a.base_dir, &account.id, &a.current_folder);
    maildir::save_index(&idx_path, &a.all_messages);
}

// ═══════════════════════════════════════════════════════════════════════════
// Actions
// ═══════════════════════════════════════════════════════════════════════════

fn on_delete() {
    let a = app();
    if let Some(idx) = a.selected_msg_idx {
        if idx < a.messages.len() {
            let uid = a.messages[idx].uid;

            // If in Trash already, permanently delete
            if a.current_folder == "Trash" {
                a.all_messages.retain(|m| m.uid != uid);
                if a.current_account < a.config.accounts.len() {
                    let account = &a.config.accounts[a.current_account];
                    let msg_path = maildir::message_path(&a.base_dir, &account.id, "Trash", uid);
                    maildir::delete_message(&msg_path);
                }
            } else {
                // Move to Trash: mark deleted and copy to Trash folder
                for m in &mut a.all_messages {
                    if m.uid == uid {
                        m.flags |= FLAG_DELETED;
                        // Copy summary to Trash index
                        if a.current_account < a.config.accounts.len() {
                            let account = &a.config.accounts[a.current_account];
                            let trash_path = maildir::index_path(&a.base_dir, &account.id, "Trash");
                            let mut trash_msgs = maildir::load_index(&trash_path);
                            trash_msgs.push(m.clone());
                            maildir::save_index(&trash_path, &trash_msgs);
                        }
                        break;
                    }
                }
                a.all_messages.retain(|m| m.uid != uid);
            }

            save_current_index();
            a.selected_msg_idx = None;
            a.current_full_msg = None;
            a.preview_header.set_text("");
            a.preview_body.set_text("");
            apply_filters();
            set_status(anyos_std::i18n::t("Message deleted"));
        }
    }
}

fn on_mark_junk() {
    let a = app();
    if let Some(idx) = a.selected_msg_idx {
        if idx < a.messages.len() {
            let uid = a.messages[idx].uid;
            for m in &mut a.all_messages {
                if m.uid == uid {
                    m.flags ^= FLAG_JUNK;
                    break;
                }
            }
            save_current_index();
            apply_filters();
        }
    }
}

fn on_archive() {
    // Move selected message to Archive (or a custom folder)
    // For now, just mark it as seen and remove from current folder
    let a = app();
    if let Some(idx) = a.selected_msg_idx {
        if idx < a.messages.len() {
            let uid = a.messages[idx].uid;
            a.all_messages.retain(|m| m.uid != uid);
            save_current_index();
            a.selected_msg_idx = None;
            a.current_full_msg = None;
            a.preview_header.set_text("");
            a.preview_body.set_text("");
            apply_filters();
            set_status(anyos_std::i18n::t("Message archived"));
        }
    }
}

fn on_context_action(index: u32) {
    match index {
        0 => { // Mark as Read
            if let Some(idx) = app().selected_msg_idx {
                mark_message_flag(idx, FLAG_SEEN, true);
            }
        }
        1 => { // Mark as Unread
            if let Some(idx) = app().selected_msg_idx {
                mark_message_flag(idx, FLAG_SEEN, false);
            }
        }
        2 => { // Toggle Star
            if let Some(idx) = app().selected_msg_idx {
                let flagged = idx < app().messages.len() && app().messages[idx].is_flagged();
                mark_message_flag(idx, FLAG_FLAGGED, !flagged);
            }
        }
        3 => on_mark_junk(),
        4 => on_delete(),  // Move to Trash
        5 => on_archive(), // Move to Archive
        7 => on_delete(),  // Delete Permanently
        _ => {}
    }
}

fn mark_message_flag(idx: usize, flag: u32, set: bool) {
    let a = app();
    if idx >= a.messages.len() { return; }
    let uid = a.messages[idx].uid;

    if set {
        a.messages[idx].flags |= flag;
    } else {
        a.messages[idx].flags &= !flag;
    }

    for m in &mut a.all_messages {
        if m.uid == uid {
            if set { m.flags |= flag; } else { m.flags &= !flag; }
            break;
        }
    }

    save_current_index();
    refresh_grid();
}

/// Spawn a detached background thread (don't wait on drop).
fn spawn_detached(entry: fn(), stack_size: usize, name: &str) {
    if let Ok(handle) = anyos_std::process::Thread::spawn_with_stack(entry, stack_size, name) {
        core::mem::forget(handle); // Don't block on drop; worker manages its own lifecycle
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Get Mail (Sync)
// ═══════════════════════════════════════════════════════════════════════════

fn on_get_mail() {
    let a = app();
    if a.config.accounts.is_empty() {
        set_status(anyos_std::i18n::t("No accounts configured. Click 'Accounts' to add one."));
        return;
    }

    let ss = sync_worker::sync_state();

    // If already running, cancel instead
    if ss.worker_active.load(core::sync::atomic::Ordering::Relaxed) {
        ss.cancel_requested.store(true, core::sync::atomic::Ordering::Release);
        set_status(anyos_std::i18n::t("Cancelling sync..."));
        return;
    }

    // Prepare sync input
    ss.acquire();
    ss.accounts = a.config.accounts.clone();
    ss.base_dir = a.base_dir.clone();
    ss.target_account_idx = None;    // sync all accounts
    ss.target_folder = None;         // sync default folders
    ss.uid_offset = 0;
    ss.fetch_limit = sync_worker::LAZY_BATCH_SIZE;
    ss.reset_output();
    ss.release();

    ss.worker_active.store(true, core::sync::atomic::Ordering::Release);
    ss.phase.store(SyncPhase::Connecting as u32, core::sync::atomic::Ordering::Release);

    // Spawn background thread (128KB stack for network buffers + TLS)
    spawn_detached(sync_worker::sync_worker_entry, 128 * 1024, "mail-sync");

    a.btn_getmail.set_text(anyos_std::i18n::t("Cancel"));
    set_status(anyos_std::i18n::t("Syncing mail..."));
}

/// Trigger background fetch for a specific folder (used on folder selection).
fn trigger_folder_sync(acct_idx: usize, folder_name: &str) {
    let a = app();
    if acct_idx >= a.config.accounts.len() { return; }

    let ss = sync_worker::sync_state();
    if ss.worker_active.load(core::sync::atomic::Ordering::Relaxed) { return; }

    ss.acquire();
    ss.accounts = a.config.accounts.clone();
    ss.base_dir = a.base_dir.clone();
    ss.target_account_idx = Some(acct_idx);
    ss.target_folder = Some(String::from(folder_name));
    ss.uid_offset = a.all_messages.iter().map(|m| m.uid).max().unwrap_or(0);
    ss.fetch_limit = sync_worker::LAZY_BATCH_SIZE;
    ss.reset_output();
    ss.release();

    ss.worker_active.store(true, core::sync::atomic::Ordering::Release);
    ss.phase.store(SyncPhase::Connecting as u32, core::sync::atomic::Ordering::Release);

    spawn_detached(sync_worker::sync_worker_entry, 128 * 1024, "mail-folder");
}

/// Poll the sync worker state and update UI. Called by 500ms timer.
fn poll_sync_state() {
    use core::sync::atomic::Ordering;
    let ss = sync_worker::sync_state();

    // 1. Status updates
    if ss.status_ready.swap(false, Ordering::AcqRel) {
        ss.acquire();
        let text = ss.status_text.clone();
        let current = ss.progress_current;
        let total = ss.progress_total;
        ss.release();
        if total > 0 {
            set_status(&format!("{} ({}/{})", text, current, total));
        } else {
            set_status(&text);
        }
    }

    // 2. Folder structure updates
    if ss.folders_ready.swap(false, Ordering::AcqRel) {
        ss.acquire();
        let results: Vec<sync_worker::FolderSyncResult> = core::mem::take(&mut ss.folder_results);
        ss.release();
        for result in &results {
            update_folder_tree(result.account_idx, &result.folders);
        }
    }

    // 3. New message batches
    if ss.messages_ready.swap(false, Ordering::AcqRel) {
        ss.acquire();
        let batches: Vec<sync_worker::MessageBatch> = core::mem::take(&mut ss.message_batches);
        ss.release();

        let a = app();
        for batch in &batches {
            // Check if this batch is for the currently viewed folder
            let is_current = a.config.accounts.get(a.current_account)
                .map(|acc| acc.id == batch.account_id && a.current_folder == batch.folder_name)
                .unwrap_or(false);

            if is_current {
                // Append new messages
                for msg in &batch.messages {
                    if !a.all_messages.iter().any(|m| m.uid == msg.uid) {
                        a.all_messages.push(msg.clone());
                    }
                }
                apply_filters();

                a.folder_loaded_count = a.all_messages.len() as u32;
                a.folder_total_count = batch.total_on_server;
                a.has_more_messages = a.folder_loaded_count < a.folder_total_count;

                if batch.is_final {
                    a.loading_more = false;
                }

                let total = a.all_messages.len();
                let unread = a.all_messages.iter().filter(|m| !m.is_seen()).count();
                let t = anyos_std::i18n::t;
                if a.has_more_messages {
                    set_status(&format!("{} - {} {} ({} {}), {} {}",
                        a.current_folder, total, t("messages"),
                        a.folder_total_count, t("on server"),
                        unread, t("unread")));
                } else {
                    set_status(&format!("{} - {} {}, {} {}",
                        a.current_folder, total, t("messages"), unread, t("unread")));
                }
            }
        }
    }

    // 4. Completion or error
    let phase = ss.phase.load(Ordering::Acquire);
    if phase == SyncPhase::Done as u32 {
        ss.phase.store(SyncPhase::Idle as u32, Ordering::Release);
        let a = app();
        a.btn_getmail.set_text(anyos_std::i18n::t("Get Mail"));
        a.loading_more = false;
        // Reload current folder from disk (final authoritative state)
        load_folder_messages();
        set_status(&format!("{} ({})", anyos_std::i18n::t("Mail check complete"), now_string()));
    } else if phase == SyncPhase::Error as u32 {
        ss.phase.store(SyncPhase::Idle as u32, Ordering::Release);
        ss.acquire();
        let err = ss.error_text.clone();
        ss.release();
        let a = app();
        a.btn_getmail.set_text(anyos_std::i18n::t("Get Mail"));
        a.loading_more = false;
        set_status(&err);
    }

    // 5. Lazy loading: check if user scrolled near end
    check_lazy_load_needed();
}

/// Check if user has scrolled near the bottom and trigger lazy loading.
fn check_lazy_load_needed() {
    let a = app();
    if !a.has_more_messages || a.loading_more { return; }

    let ss = sync_worker::sync_state();
    if ss.worker_active.load(core::sync::atomic::Ordering::Relaxed) { return; }

    let scroll_offset = a.msg_grid.scroll_offset();
    let visible_rows = 15u32; // approximate: grid_height / row_height
    let total_rows = a.messages.len() as u32;

    if total_rows > 0 && scroll_offset + visible_rows + 20 >= total_rows {
        a.loading_more = true;

        // Fetch UIDs older than the oldest we have
        let min_uid = a.all_messages.iter().map(|m| m.uid).min().unwrap_or(0);

        ss.acquire();
        ss.accounts = a.config.accounts.clone();
        ss.base_dir = a.base_dir.clone();
        ss.target_account_idx = Some(a.current_account);
        ss.target_folder = Some(a.current_folder.clone());
        ss.uid_offset = 0; // fetch from beginning to find older UIDs
        ss.fetch_limit = sync_worker::LAZY_BATCH_SIZE;
        ss.reset_output();
        ss.release();

        ss.worker_active.store(true, core::sync::atomic::Ordering::Release);
        ss.phase.store(SyncPhase::Connecting as u32, core::sync::atomic::Ordering::Release);

        spawn_detached(sync_worker::sync_worker_entry, 128 * 1024, "mail-lazy");
        set_status(anyos_std::i18n::t("Loading more messages..."));
    }
}

fn update_folder_tree(acct_idx: usize, imap_folders: &[ImapFolder]) {
    let a = app();

    // Remove old folders for this account
    a.folders.retain(|f| f.account_idx != acct_idx);

    // Find the root node for this account
    // The root nodes are at indices 0, N+1, 2N+2, etc. in the tree
    // For simplicity, we rebuild the whole tree
    populate_folder_tree();

    // Add IMAP-discovered folders that aren't in the default set
    let default_names = ["INBOX", "Sent", "Drafts", "Trash", "Spam"];
    for imap_folder in imap_folders {
        let name_upper = to_upper(&imap_folder.name);
        let is_default = default_names.iter().any(|d| to_upper(d) == name_upper);
        if !is_default {
            // Add as custom folder under the account root
            // Find account root node (first root = index 0, etc.)
            let root_idx = acct_idx as u32; // simplified
            let node_id = a.folder_tree.add_child(root_idx, &imap_folder.name);
            a.folders.push(FolderInfo {
                account_idx: acct_idx,
                folder_name: imap_folder.name.clone(),
                special_use: imap_folder.special_use,
                node_id,
            });
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Build contact suggestions for autocomplete
// ═══════════════════════════════════════════════════════════════════════════

fn build_contact_suggestions(address_book: &AddressBook) -> String {
    let mut suggestions = Vec::new();
    for contact in &address_book.contacts {
        if !contact.name.is_empty() {
            suggestions.push(format!("{} <{}>", contact.name, contact.email));
        } else {
            suggestions.push(contact.email.clone());
        }
    }
    suggestions.join("|")
}

// ═══════════════════════════════════════════════════════════════════════════
// Compose Window
// ═══════════════════════════════════════════════════════════════════════════

fn open_compose(mode: ComposeMode) {
    let a = app();
    if a.config.accounts.is_empty() {
        anyui::MessageBox::show(anyui::MessageBoxType::Warning,
            anyos_std::i18n::t("No email account configured.\nPlease add an account first."), Some("OK"));
        return;
    }

    let account = &a.config.accounts[a.current_account];

    // Prepare compose data based on mode
    let (to_str, cc_str, subject, body, in_reply_to, references) = match mode {
        ComposeMode::New => {
            (String::new(), String::new(), String::new(), String::new(), String::new(), String::new())
        }
        ComposeMode::Reply | ComposeMode::ReplyAll => {
            if let Some(ref full) = a.current_full_msg {
                let (to, cc, subj, body, irt, refs) =
                    crate::mail::compose::prepare_reply(full, mode == ComposeMode::ReplyAll);
                let to_str = to.iter().map(|addr: &EmailAddress| addr.to_header_string()).collect::<Vec<_>>().join(", ");
                let cc_str = cc.iter().map(|addr: &EmailAddress| addr.to_header_string()).collect::<Vec<_>>().join(", ");
                (to_str, cc_str, subj, body, irt, refs)
            } else {
                return;
            }
        }
        ComposeMode::Forward => {
            if let Some(ref full) = a.current_full_msg {
                let (subj, body) = crate::mail::compose::prepare_forward(full);
                (String::new(), String::new(), subj, body, String::new(), String::new())
            } else {
                return;
            }
        }
        ComposeMode::Draft => {
            if let Some(ref full) = a.current_full_msg {
                let to_str = full.summary.to.iter().map(|addr: &EmailAddress| addr.to_header_string()).collect::<Vec<_>>().join(", ");
                let cc_str = full.cc.iter().map(|addr: &EmailAddress| addr.to_header_string()).collect::<Vec<_>>().join(", ");
                (to_str, cc_str, full.summary.subject.clone(), full.text_body.clone(), String::new(), String::new())
            } else {
                return;
            }
        }
    };

    // Create compose window
    let t = anyos_std::i18n::t;
    let compose_win = anyui::Window::new(t("Compose Message"), -1, -1, 700, 550);

    // Toolbar
    let comp_toolbar = anyui::Toolbar::new();
    comp_toolbar.set_dock(anyui::DOCK_TOP);
    comp_toolbar.set_size(700, 46);
    comp_toolbar.set_color(COL_TOOLBAR);
    comp_toolbar.set_padding(4, 4, 4, 4);

    let btn_send = comp_toolbar.add_icon_button("");
    btn_send.set_size(34, 34);
    btn_send.set_system_icon("mail-fast", IconType::Outline, 0xFFCCCCCC, 24);
    btn_send.set_tooltip(t("Send"));

    let btn_attach = comp_toolbar.add_icon_button("");
    btn_attach.set_size(34, 34);
    btn_attach.set_system_icon("paperclip", IconType::Outline, 0xFFCCCCCC, 24);
    btn_attach.set_tooltip(t("Attach"));

    let btn_save_draft = comp_toolbar.add_icon_button("");
    btn_save_draft.set_size(34, 34);
    btn_save_draft.set_system_icon("device-floppy", IconType::Outline, 0xFFCCCCCC, 24);
    btn_save_draft.set_tooltip(t("Save Draft"));

    compose_win.add(&comp_toolbar);

    // Header fields
    let header_panel = anyui::View::new();
    header_panel.set_dock(anyui::DOCK_TOP);
    header_panel.set_size(700, 120);
    header_panel.set_color(0xFF2A2A3A);
    header_panel.set_padding(8, 4, 8, 4);

    // From label
    let from_label = anyui::Label::new(&format!("{}: {}", t("From"), account.email));
    from_label.set_position(8, 6);
    from_label.set_size(680, 20);
    from_label.set_text_color(COL_DIM);
    header_panel.add(&from_label);

    // To field (with autocomplete)
    let to_label = anyui::Label::new(&format!("{}:", t("To")));
    to_label.set_position(8, 30);
    to_label.set_size(40, 20);
    to_label.set_text_color(COL_DIM);
    header_panel.add(&to_label);
    let to_field = anyui::AutoCompleteTextField::new();
    to_field.set_position(50, 28);
    to_field.set_size(630, 22);
    to_field.set_text(&to_str);
    to_field.set_placeholder(t("recipient@example.com"));
    let suggestions = build_contact_suggestions(&a.address_book);
    to_field.set_suggestions(&suggestions);
    header_panel.add(&to_field);

    // Cc field (with autocomplete)
    let cc_label = anyui::Label::new(&format!("{}:", t("Cc")));
    cc_label.set_position(8, 54);
    cc_label.set_size(40, 20);
    cc_label.set_text_color(COL_DIM);
    header_panel.add(&cc_label);
    let cc_field = anyui::AutoCompleteTextField::new();
    cc_field.set_position(50, 52);
    cc_field.set_size(630, 22);
    cc_field.set_text(&cc_str);
    cc_field.set_placeholder(t("recipient@example.com"));
    cc_field.set_suggestions(&suggestions);
    header_panel.add(&cc_field);

    // Subject field
    let subj_label = anyui::Label::new(&format!("{}:", t("Subject")));
    subj_label.set_position(8, 78);
    subj_label.set_size(40, 20);
    subj_label.set_text_color(COL_DIM);
    header_panel.add(&subj_label);
    let subj_field = anyui::TextField::new();
    subj_field.set_position(50, 76);
    subj_field.set_size(630, 22);
    subj_field.set_text(&subject);
    header_panel.add(&subj_field);

    compose_win.add(&header_panel);

    // Body editor (DOCK_FILL)
    let body_editor = anyui::TextEditor::new(700, 400);
    body_editor.set_dock(anyui::DOCK_FILL);
    body_editor.set_show_line_numbers(false);
    body_editor.set_text(&body);

    // Add signature if available
    if !account.signature.is_empty() && mode == ComposeMode::New {
        let with_sig = format!("\n\n-- \n{}", account.signature);
        body_editor.set_text(&with_sig);
    }

    compose_win.add(&body_editor);

    // ── Compose callbacks ──────────────────────────────────────────────

    // Clone values for closures
    let acct_idx = a.current_account;
    let irt = String::from(in_reply_to.as_str());
    let refs = String::from(references.as_str());

    btn_send.on_click(move |_| {
        on_send(&to_field, &cc_field, &subj_field, &body_editor, acct_idx, &irt, &refs);
    });

    btn_attach.on_click(|_| {
        if let Some(path) = anyui::FileDialog::open_file() {
            // Attachment handling - load file and add to message
            // For now just notify the user
            anyui::MessageBox::show(anyui::MessageBoxType::Info,
                &format!("{}: {}", anyos_std::i18n::t("Attachment added"), path), Some("OK"));
        }
    });

    btn_save_draft.on_click(move |_| {
        save_draft(&to_field, &cc_field, &subj_field, &body_editor, acct_idx);
    });

    compose_win.on_key_down(move |e| {
        if e.ctrl() && (e.char_code == b'S' as u32 || e.char_code == b's' as u32) {
            save_draft(&to_field, &cc_field, &subj_field, &body_editor, acct_idx);
        }
    });

    compose_win.on_close(|_| {});
}

fn on_send(
    to_field: &anyui::AutoCompleteTextField,
    cc_field: &anyui::AutoCompleteTextField,
    subj_field: &anyui::TextField,
    body_editor: &anyui::TextEditor,
    acct_idx: usize,
    in_reply_to: &str,
    references: &str,
) {
    let a = app();
    if acct_idx >= a.config.accounts.len() { return; }
    let account = a.config.accounts[acct_idx].clone();

    // Read fields
    let to_str = get_field_text(to_field);
    let cc_str = get_field_text(cc_field);
    let subj_str = get_field_text(subj_field);

    let mut body_buf = vec![0u8; 64 * 1024];
    let body_len = body_editor.get_text(&mut body_buf);
    let body_str = String::from(core::str::from_utf8(&body_buf[..body_len as usize]).unwrap_or(""));

    if to_str.is_empty() {
        anyui::MessageBox::show(anyui::MessageBoxType::Warning, anyos_std::i18n::t("Please enter a recipient."), Some("OK"));
        return;
    }

    // Parse addresses
    let from = EmailAddress::with_name(&account.display_name, &account.email);
    let to_addrs = crate::mail::rfc2822::parse_address_list(&to_str);
    let cc_addrs = crate::mail::rfc2822::parse_address_list(&cc_str);

    // Build MIME message
    let message_data = crate::mail::compose::build_message(
        &from, &to_addrs, &cc_addrs, &subj_str, &body_str, "", &[], in_reply_to, references,
    );

    // Connect to SMTP
    set_status(&format!("Sending via {}:{}...", account.smtp_host, account.smtp_port));

    let mut smtp = SmtpClient::new();
    let smtp_tls = account.smtp_use_tls();

    if let Err(e) = smtp.connect(&account.smtp_host, account.smtp_port, smtp_tls) {
        set_status(&format!("SMTP connect failed: {:?}", e));
        return;
    }

    // EHLO
    let domain = crate::mail::rfc2822::domain_of(&account.email);
    if let Err(e) = smtp.ehlo(domain) {
        set_status(&format!("SMTP EHLO failed: {:?}", e));
        smtp.quit();
        return;
    }

    // STARTTLS if needed
    if account.smtp_use_starttls() {
        if let Err(e) = smtp.starttls() {
            set_status(&format!("SMTP STARTTLS failed: {:?}", e));
            smtp.quit();
            return;
        }
        // Re-EHLO after TLS
        let _ = smtp.ehlo(domain);
    }

    // AUTH
    let user = if account.smtp_user.is_empty() { &account.incoming_user } else { &account.smtp_user };
    let pass = if account.smtp_pass.is_empty() { &account.incoming_pass } else { &account.smtp_pass };

    if smtp.has_capability("AUTH PLAIN") {
        if let Err(e) = smtp.auth_plain(user, pass) {
            set_status(&format!("SMTP AUTH failed: {:?}", e));
            smtp.quit();
            return;
        }
    } else {
        if let Err(e) = smtp.auth_login(user, pass) {
            set_status(&format!("SMTP AUTH failed: {:?}", e));
            smtp.quit();
            return;
        }
    }

    // Send
    let recipients: Vec<&str> = to_addrs.iter().chain(cc_addrs.iter())
        .map(|a| a.address.as_str())
        .collect();

    if let Err(e) = smtp.send_mail(&account.email, &recipients, &message_data) {
        set_status(&format!("Send failed: {:?}", e));
        smtp.quit();
        return;
    }

    smtp.quit();

    // Save to Sent folder
    let base = app().base_dir.clone();
    let sent_idx_path = maildir::index_path(&base, &account.id, "Sent");
    let mut sent_msgs = maildir::load_index(&sent_idx_path);
    let next_uid = sent_msgs.iter().map(|m| m.uid).max().unwrap_or(0) + 1;

    let mut summary = MessageSummary::new();
    summary.uid = next_uid;
    summary.from = from;
    summary.to = to_addrs;
    summary.subject = String::from(&subj_str);
    summary.date = now_string();
    summary.size = message_data.len() as u64;
    summary.flags = FLAG_SEEN;

    let msg_path = maildir::message_path(&base, &account.id, "Sent", next_uid);
    maildir::save_message(&msg_path, &message_data);
    sent_msgs.push(summary);
    maildir::save_index(&sent_idx_path, &sent_msgs);

    set_status(anyos_std::i18n::t("Message sent successfully"));
    anyui::MessageBox::show(anyui::MessageBoxType::Info, anyos_std::i18n::t("Message sent."), Some("OK"));
}

fn save_draft(
    to_field: &anyui::AutoCompleteTextField,
    cc_field: &anyui::AutoCompleteTextField,
    subj_field: &anyui::TextField,
    body_editor: &anyui::TextEditor,
    acct_idx: usize,
) {
    let a = app();
    if acct_idx >= a.config.accounts.len() { return; }
    let account = a.config.accounts[acct_idx].clone();

    let to_str = get_field_text(to_field);
    let cc_str = get_field_text(cc_field);
    let subj_str = get_field_text(subj_field);

    let mut body_buf = vec![0u8; 64 * 1024];
    let body_len = body_editor.get_text(&mut body_buf);
    let body_str = String::from(core::str::from_utf8(&body_buf[..body_len as usize]).unwrap_or(""));

    let from = EmailAddress::with_name(&account.display_name, &account.email);
    let to_addrs = crate::mail::rfc2822::parse_address_list(&to_str);
    let cc_addrs = crate::mail::rfc2822::parse_address_list(&cc_str);

    let message_data = crate::mail::compose::build_message(
        &from, &to_addrs, &cc_addrs, &subj_str, &body_str, "", &[], "", "",
    );

    let base = a.base_dir.clone();
    let drafts_path = maildir::index_path(&base, &account.id, "Drafts");
    let mut drafts = maildir::load_index(&drafts_path);
    let next_uid = drafts.iter().map(|m| m.uid).max().unwrap_or(0) + 1;

    let mut summary = MessageSummary::new();
    summary.uid = next_uid;
    summary.from = from;
    summary.to = to_addrs;
    summary.subject = String::from(&subj_str);
    summary.date = now_string();
    summary.size = message_data.len() as u64;
    summary.flags = FLAG_DRAFT | FLAG_SEEN;

    let msg_path = maildir::message_path(&base, &account.id, "Drafts", next_uid);
    maildir::save_message(&msg_path, &message_data);
    drafts.push(summary);
    maildir::save_index(&drafts_path, &drafts);

    set_status(anyos_std::i18n::t("Draft saved"));
}

// ═══════════════════════════════════════════════════════════════════════════
// Account Setup Dialog
// ═══════════════════════════════════════════════════════════════════════════

fn open_account_setup() {
    let t = anyos_std::i18n::t;
    let setup_win = anyui::Window::new(t("Account Settings"), -1, -1, 500, 580);

    let panel = anyui::View::new();
    panel.set_dock(anyui::DOCK_FILL);
    panel.set_color(COL_BG);
    panel.set_padding(16, 16, 16, 16);

    let mut y = 8i32;
    let label_w = 120u32;
    let field_w = 340u32;
    let row_h = 30i32;

    // Display name
    let lbl = anyui::Label::new(&format!("{}:", t("Display Name")));
    lbl.set_position(8, y); lbl.set_size(label_w, 20); lbl.set_text_color(COL_DIM);
    panel.add(&lbl);
    let name_field = anyui::TextField::new();
    name_field.set_position(130, y); name_field.set_size(field_w, 22);
    name_field.set_placeholder(t("Your Name"));
    panel.add(&name_field);
    y += row_h;

    // Email
    let lbl = anyui::Label::new(&format!("{}:", t("Email Address")));
    lbl.set_position(8, y); lbl.set_size(label_w, 20); lbl.set_text_color(COL_DIM);
    panel.add(&lbl);
    let email_field = anyui::TextField::new();
    email_field.set_position(130, y); email_field.set_size(field_w, 22);
    email_field.set_placeholder("user@example.com");
    panel.add(&email_field);
    y += row_h;

    // Protocol
    let lbl = anyui::Label::new(&format!("{}:", t("Protocol")));
    lbl.set_position(8, y); lbl.set_size(label_w, 20); lbl.set_text_color(COL_DIM);
    panel.add(&lbl);
    let proto_seg = anyui::SegmentedControl::new("IMAP|POP3");
    proto_seg.set_position(130, y); proto_seg.set_size(200, 24);
    panel.add(&proto_seg);
    y += row_h + 8;

    // Incoming server
    let lbl = anyui::Label::new(&format!("{}:", t("Incoming Server")));
    lbl.set_position(8, y); lbl.set_size(label_w, 20); lbl.set_text_color(COL_DIM);
    panel.add(&lbl);
    let in_host = anyui::TextField::new();
    in_host.set_position(130, y); in_host.set_size(260, 22);
    in_host.set_placeholder("imap.example.com");
    panel.add(&in_host);
    let in_port = anyui::TextField::new();
    in_port.set_position(396, y); in_port.set_size(74, 22);
    in_port.set_placeholder("993");
    panel.add(&in_port);
    y += row_h;

    // Incoming security
    let lbl = anyui::Label::new(&format!("{}:", t("Security")));
    lbl.set_position(8, y); lbl.set_size(label_w, 20); lbl.set_text_color(COL_DIM);
    panel.add(&lbl);
    let sec_items = format!("TLS|STARTTLS|{}", t("None"));
    let in_sec = anyui::SegmentedControl::new(&sec_items);
    in_sec.set_position(130, y); in_sec.set_size(240, 24);
    panel.add(&in_sec);
    y += row_h;

    // Username
    let lbl = anyui::Label::new(&format!("{}:", t("Username")));
    lbl.set_position(8, y); lbl.set_size(label_w, 20); lbl.set_text_color(COL_DIM);
    panel.add(&lbl);
    let user_field = anyui::TextField::new();
    user_field.set_position(130, y); user_field.set_size(field_w, 22);
    user_field.set_placeholder("user@example.com");
    panel.add(&user_field);
    y += row_h;

    // Password
    let lbl = anyui::Label::new(&format!("{}:", t("Password")));
    lbl.set_position(8, y); lbl.set_size(label_w, 20); lbl.set_text_color(COL_DIM);
    panel.add(&lbl);
    let pass_field = anyui::TextField::new();
    pass_field.set_position(130, y); pass_field.set_size(field_w, 22);
    pass_field.set_password_mode(true);
    panel.add(&pass_field);
    y += row_h + 12;

    // SMTP section header
    let smtp_hdr = anyui::Label::new(&format!("{} (SMTP):", t("Outgoing Server")));
    smtp_hdr.set_position(8, y); smtp_hdr.set_size(300, 20);
    smtp_hdr.set_text_color(COL_WHITE);
    panel.add(&smtp_hdr);
    y += 24;

    // SMTP host
    let lbl = anyui::Label::new(&format!("{} SMTP:", t("Server")));
    lbl.set_position(8, y); lbl.set_size(label_w, 20); lbl.set_text_color(COL_DIM);
    panel.add(&lbl);
    let smtp_host = anyui::TextField::new();
    smtp_host.set_position(130, y); smtp_host.set_size(260, 22);
    smtp_host.set_placeholder("smtp.example.com");
    panel.add(&smtp_host);
    let smtp_port = anyui::TextField::new();
    smtp_port.set_position(396, y); smtp_port.set_size(74, 22);
    smtp_port.set_placeholder("587");
    panel.add(&smtp_port);
    y += row_h;

    // SMTP security
    let lbl = anyui::Label::new(&format!("{} SMTP:", t("Security")));
    lbl.set_position(8, y); lbl.set_size(label_w, 20); lbl.set_text_color(COL_DIM);
    panel.add(&lbl);
    let smtp_sec_items = format!("TLS|STARTTLS|{}", t("None"));
    let smtp_sec = anyui::SegmentedControl::new(&smtp_sec_items);
    smtp_sec.set_position(130, y); smtp_sec.set_size(240, 24);
    panel.add(&smtp_sec);
    y += row_h + 16;

    // Buttons
    let btn_save = anyui::Button::new(t("Save Account"));
    btn_save.set_position(130, y); btn_save.set_size(120, 30);
    panel.add(&btn_save);

    let btn_test = anyui::Button::new(t("Test Connection"));
    btn_test.set_position(260, y); btn_test.set_size(120, 30);
    panel.add(&btn_test);

    y += 40;

    // Export / Import buttons
    let btn_export = anyui::Button::new(t("Export"));
    btn_export.set_position(130, y); btn_export.set_size(100, 30);
    panel.add(&btn_export);

    let btn_import = anyui::Button::new(t("Import"));
    btn_import.set_position(240, y); btn_import.set_size(100, 30);
    panel.add(&btn_import);

    // Pre-fill if editing existing account
    let a = app();
    if a.current_account < a.config.accounts.len() {
        let acc = &a.config.accounts[a.current_account];
        name_field.set_text(&acc.display_name);
        email_field.set_text(&acc.email);
        in_host.set_text(&acc.incoming_host);
        in_port.set_text(&format!("{}", acc.incoming_port));
        user_field.set_text(&acc.incoming_user);
        pass_field.set_text(&acc.incoming_pass);
        smtp_host.set_text(&acc.smtp_host);
        smtp_port.set_text(&format!("{}", acc.smtp_port));
    }

    setup_win.add(&panel);

    // Save callback
    btn_save.on_click(move |_| {
        let mut acc = Account::new();
        acc.display_name = get_field_text(&name_field);
        acc.email = get_field_text(&email_field);
        acc.id = Account::generate_id(&acc.email);
        acc.incoming_host = get_field_text(&in_host);
        acc.incoming_port = get_field_text(&in_port).parse().unwrap_or(993);
        acc.incoming_user = get_field_text(&user_field);
        acc.incoming_pass = get_field_text(&pass_field);
        acc.smtp_host = get_field_text(&smtp_host);
        acc.smtp_port = get_field_text(&smtp_port).parse().unwrap_or(587);

        // TODO: read protocol and security from SegmentedControl selection

        let a = app();
        // Check if this account already exists
        let existing_idx = a.config.accounts.iter().position(|a| a.id == acc.id);
        if let Some(idx) = existing_idx {
            a.config.accounts[idx] = acc;
        } else {
            a.config.accounts.push(acc);
        }

        a.config.save(&a.config_path);
        maildir::ensure_dirs(&a.base_dir, &a.config.accounts.last().unwrap().id);
        populate_folder_tree();
        set_status(anyos_std::i18n::t("Account saved"));
        anyui::MessageBox::show(anyui::MessageBoxType::Info, anyos_std::i18n::t("Account saved successfully."), Some("OK"));
    });

    // Test connection callback
    btn_test.on_click(move |_| {
        let host = get_field_text(&in_host);
        let port: u16 = get_field_text(&in_port).parse().unwrap_or(993);
        let user = get_field_text(&user_field);
        let pass = get_field_text(&pass_field);

        set_status(&format!("Testing connection to {}:{}...", host, port));

        // Try IMAP connection as a test
        let mut client = ImapClient::new();
        match client.connect(&host, port, true) {
            Ok(()) => {
                match client.login(&user, &pass) {
                    Ok(()) => {
                        client.logout();
                        set_status(anyos_std::i18n::t("Connection test successful!"));
                        anyui::MessageBox::show(anyui::MessageBoxType::Info,
                            anyos_std::i18n::t("Connection successful!\nLogin verified."), Some("OK"));
                    }
                    Err(e) => {
                        client.logout();
                        set_status(&format!("Login failed: {:?}", e));
                        anyui::MessageBox::show(anyui::MessageBoxType::Alert,
                            &format!("Login failed: {:?}", e), Some("OK"));
                    }
                }
            }
            Err(e) => {
                set_status(&format!("Connection failed: {:?}", e));
                anyui::MessageBox::show(anyui::MessageBoxType::Alert,
                    &format!("Connection failed: {:?}", e), Some("OK"));
            }
        }
    });

    // Export accounts
    btn_export.on_click(|_| {
        export_accounts();
    });

    // Import accounts
    btn_import.on_click(|_| {
        import_accounts();
    });

    setup_win.on_close(|_| {});
}

// ═══════════════════════════════════════════════════════════════════════════
// Account Export / Import
// ═══════════════════════════════════════════════════════════════════════════

fn export_accounts() {
    if let Some(path) = anyui::FileDialog::save_file("anymail-accounts.json") {
        let a = app();
        a.config.save(&path);
        set_status(&format!("{}: {}", anyos_std::i18n::t("Accounts exported to"), path));
        anyui::MessageBox::show(anyui::MessageBoxType::Info,
            anyos_std::i18n::t("Accounts exported successfully."), Some("OK"));
    }
}

fn import_accounts() {
    if let Some(path) = anyui::FileDialog::open_file() {
        let imported = MailConfig::load(&path);
        if imported.accounts.is_empty() {
            anyui::MessageBox::show(anyui::MessageBoxType::Warning,
                &format!("{}\n{}: {}",
                    anyos_std::i18n::t("No accounts found in the selected file."),
                    anyos_std::i18n::t("Path"), path), Some("OK"));
            return;
        }

        // Show import dialog with Merge / Replace options
        let t = anyos_std::i18n::t;
        let import_win = anyui::Window::new(t("Import Accounts"), -1, -1, 400, 180);

        let panel = anyui::View::new();
        panel.set_dock(anyui::DOCK_FILL);
        panel.set_color(COL_BG);
        panel.set_padding(16, 16, 16, 16);

        let info = anyui::Label::new(&format!("{}: {} {}",
            t("Found"), imported.accounts.len(), t("accounts")));
        info.set_position(16, 16);
        info.set_size(360, 24);
        info.set_text_color(COL_WHITE);
        panel.add(&info);

        let question = anyui::Label::new(t("Merge with existing or replace all?"));
        question.set_position(16, 48);
        question.set_size(360, 24);
        question.set_text_color(COL_DIM);
        panel.add(&question);

        let btn_merge = anyui::Button::new(t("Merge"));
        btn_merge.set_position(80, 90);
        btn_merge.set_size(100, 30);
        panel.add(&btn_merge);

        let btn_replace = anyui::Button::new(t("Replace All"));
        btn_replace.set_position(200, 90);
        btn_replace.set_size(100, 30);
        panel.add(&btn_replace);

        import_win.add(&panel);

        // Clone imported config for closures
        let imported_accounts = imported.accounts.clone();
        let imported_check = imported.check_on_startup;

        let imported_accounts2 = imported_accounts.clone();

        btn_merge.on_click(move |_| {
            let a = app();
            for acc in &imported_accounts {
                if !a.config.accounts.iter().any(|existing| existing.id == acc.id) {
                    a.config.accounts.push(acc.clone());
                    maildir::ensure_dirs(&a.base_dir, &acc.id);
                }
            }
            a.config.save(&a.config_path);
            populate_folder_tree();
            set_status(anyos_std::i18n::t("Accounts imported (merged)"));
            anyui::MessageBox::show(anyui::MessageBoxType::Info,
                &format!("{}\n{}",
                    anyos_std::i18n::t("Accounts merged successfully."),
                    anyos_std::i18n::t("Edit account settings to add missing passwords.")), Some("OK"));
        });

        btn_replace.on_click(move |_| {
            let a = app();
            a.config.accounts = imported_accounts2.clone();
            a.config.check_on_startup = imported_check;
            for acc in &a.config.accounts {
                maildir::ensure_dirs(&a.base_dir, &acc.id);
            }
            a.config.save(&a.config_path);
            populate_folder_tree();
            set_status(anyos_std::i18n::t("Accounts imported (replaced)"));
            anyui::MessageBox::show(anyui::MessageBoxType::Info,
                &format!("{}\n{}",
                    anyos_std::i18n::t("All accounts replaced successfully."),
                    anyos_std::i18n::t("Edit account settings to add missing passwords.")), Some("OK"));
        });

        import_win.on_close(|_| {});
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Contacts Dialog
// ═══════════════════════════════════════════════════════════════════════════

fn open_contacts() {
    let t = anyos_std::i18n::t;
    let contacts_win = anyui::Window::new(t("Address Book"), -1, -1, 600, 400);

    let toolbar = anyui::Toolbar::new();
    toolbar.set_dock(anyui::DOCK_TOP);
    toolbar.set_size(600, 46);
    toolbar.set_color(COL_TOOLBAR);
    toolbar.set_padding(4, 4, 4, 4);

    let btn_add = toolbar.add_icon_button("");
    btn_add.set_size(34, 34);
    btn_add.set_system_icon("user-plus", IconType::Outline, 0xFFCCCCCC, 24);
    btn_add.set_tooltip(t("Add Contact"));

    let btn_del = toolbar.add_icon_button("");
    btn_del.set_size(34, 34);
    btn_del.set_system_icon("user-minus", IconType::Outline, 0xFFCCCCCC, 24);
    btn_del.set_tooltip(t("Delete"));

    contacts_win.add(&toolbar);

    let grid = anyui::DataGrid::new(600, 350);
    grid.set_dock(anyui::DOCK_FILL);
    grid.set_columns(&[
        anyui::ColumnDef::new(t("Name")).width(200),
        anyui::ColumnDef::new(t("Email")).width(250),
        anyui::ColumnDef::new(t("Group")).width(100),
    ]);
    grid.set_color(COL_GRID_BG);
    grid.set_text_color(COL_WHITE);
    contacts_win.add(&grid);

    // Populate
    let a = app();
    let contacts = &a.address_book.contacts;
    grid.set_row_count(contacts.len() as u32);
    for (i, c) in contacts.iter().enumerate() {
        grid.set_cell(i as u32, 0, &c.name);
        grid.set_cell(i as u32, 1, &c.email);
        grid.set_cell(i as u32, 2, &c.group);
    }

    btn_add.on_click(move |_| {
        // Simple add contact dialog
        let add_win = anyui::Window::new(anyos_std::i18n::t("Add Contact"), -1, -1, 350, 200);
        let panel = anyui::View::new();
        panel.set_dock(anyui::DOCK_FILL);
        panel.set_color(COL_BG);
        panel.set_padding(16, 16, 16, 16);

        let t = anyos_std::i18n::t;
        let name_lbl = anyui::Label::new(&format!("{}:", t("Name")));
        name_lbl.set_position(8, 8); name_lbl.set_size(60, 20); name_lbl.set_text_color(COL_DIM);
        panel.add(&name_lbl);
        let name_f = anyui::TextField::new();
        name_f.set_position(70, 8); name_f.set_size(260, 22);
        panel.add(&name_f);

        let email_lbl = anyui::Label::new(&format!("{}:", t("Email")));
        email_lbl.set_position(8, 38); email_lbl.set_size(60, 20); email_lbl.set_text_color(COL_DIM);
        panel.add(&email_lbl);
        let email_f = anyui::TextField::new();
        email_f.set_position(70, 38); email_f.set_size(260, 22);
        panel.add(&email_f);

        let save_btn = anyui::Button::new(t("Save"));
        save_btn.set_position(70, 80); save_btn.set_size(100, 28);
        panel.add(&save_btn);

        add_win.add(&panel);

        save_btn.on_click(move |_| {
            let name = get_field_text(&name_f);
            let email = get_field_text(&email_f);
            if !email.is_empty() {
                let contact = Contact::new(&name, &email);
                let a = app();
                a.address_book.add(contact);
                let contacts_path = format!("{}/contacts.json", a.base_dir);
                a.address_book.save(&contacts_path);

                // Refresh the grid
                let contacts = &a.address_book.contacts;
                grid.set_row_count(contacts.len() as u32);
                for (i, c) in contacts.iter().enumerate() {
                    grid.set_cell(i as u32, 0, &c.name);
                    grid.set_cell(i as u32, 1, &c.email);
                    grid.set_cell(i as u32, 2, &c.group);
                }
            }
        });

        add_win.on_close(|_| {});
    });

    btn_del.on_click(move |_| {
        let row = grid.selected_row() as usize;
        let a = app();
        if row < a.address_book.contacts.len() {
            let email = a.address_book.contacts[row].email.clone();
            a.address_book.remove(&email);
            let contacts_path = format!("{}/contacts.json", a.base_dir);
            a.address_book.save(&contacts_path);

            let contacts = &a.address_book.contacts;
            grid.set_row_count(contacts.len() as u32);
            for (i, c) in contacts.iter().enumerate() {
                grid.set_cell(i as u32, 0, &c.name);
                grid.set_cell(i as u32, 1, &c.email);
                grid.set_cell(i as u32, 2, &c.group);
            }
        }
    });

    contacts_win.on_close(|_| {});
}

// ═══════════════════════════════════════════════════════════════════════════
// Utility functions
// ═══════════════════════════════════════════════════════════════════════════

fn get_field_text(field: &anyui::Control) -> String {
    let mut buf = [0u8; 1024];
    let len = field.get_text(&mut buf);
    String::from(core::str::from_utf8(&buf[..len as usize]).unwrap_or(""))
}

fn format_to_list(addrs: &[EmailAddress]) -> String {
    let mut s = String::new();
    for (i, a) in addrs.iter().enumerate() {
        if i > 0 { s.push_str(", "); }
        s.push_str(a.display_short());
    }
    if s.is_empty() { s.push_str(anyos_std::i18n::t("(none)")); }
    s
}

fn strip_html(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(c),
            _ => {}
        }
    }
    result
}

fn to_lower(s: &str) -> String {
    let mut r = String::with_capacity(s.len());
    for c in s.chars() {
        if c >= 'A' && c <= 'Z' {
            r.push((c as u8 + 32) as char);
        } else {
            r.push(c);
        }
    }
    r
}

fn to_upper(s: &str) -> String {
    let mut r = String::with_capacity(s.len());
    for c in s.chars() {
        if c >= 'a' && c <= 'z' {
            r.push((c as u8 - 32) as char);
        } else {
            r.push(c);
        }
    }
    r
}
