// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
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
use anyos_std::vec;
use anyos_std::String;
use anyos_std::Vec;
use alloc::rc::Rc;
use core::cell::Cell;

use libanyui_client as anyui;
use libanyui_client::IconType;
use libanyui_client::Widget;

use crate::mail::message::*;
use crate::mail::rfc2822::EmailAddress;
use crate::protocol::imap::{ImapClient, ImapFolder, SpecialUse};
use crate::protocol::pop3::Pop3Client;
use crate::protocol::smtp::SmtpClient;
use crate::storage::account::*;
use crate::storage::contacts::{AddressBook, Contact};
use crate::storage::maildir;
use crate::sync_worker::SyncPhase;

anyos_std::entry!(main);

// ═══════════════════════════════════════════════════════════════════════════
// Constants
// ═══════════════════════════════════════════════════════════════════════════

const CHECK_INTERVAL_MS: u32 = 300_000; // 5 minutes
const NUM_GRID_COLS: usize = 6;

// ═══════════════════════════════════════════════════════════════════════════
// Folder info stored per tree node
// ═══════════════════════════════════════════════════════════════════════════

struct FolderInfo {
    account_idx: usize,
    folder_name: String,
    special_use: SpecialUse,
    node_id: u32,
}

#[derive(Clone, Copy, PartialEq)]
enum CategoryFilter {
    All,
    Primary,
    Transactions,
    Updates,
    Promotions,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AccountProvider {
    ICloud,
    Gmail,
    Outlook,
    Yahoo,
    Fastmail,
    Gmx,
    WebDe,
    Custom,
}

// ═══════════════════════════════════════════════════════════════════════════
// App State
// ═══════════════════════════════════════════════════════════════════════════

struct AppState {
    // Paths
    base_dir: String,

    // Config
    config: MailConfig,
    address_book: AddressBook,

    // UI handles — main window
    win: anyui::Window,
    toolbar: anyui::Toolbar,
    filter_bar: anyui::View,
    search_field: anyui::SearchField,
    filter_all: anyui::Button,
    filter_primary: anyui::Button,
    filter_transactions: anyui::Button,
    filter_updates: anyui::Button,
    filter_promotions: anyui::Button,
    filter_unread: anyui::Button,
    filter_starred: anyui::Button,
    filter_attach: anyui::Button,
    category_summary: anyui::Label,
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
    category_filter: CategoryFilter,
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

fn tc() -> &'static anyui::theme::ThemeColors {
    anyui::theme::colors()
}

fn category_label(filter: CategoryFilter) -> &'static str {
    match filter {
        CategoryFilter::All => "All Mail",
        CategoryFilter::Primary => "Primary",
        CategoryFilter::Transactions => "Transactions",
        CategoryFilter::Updates => "Updates",
        CategoryFilter::Promotions => "Promotions",
    }
}

fn provider_items() -> &'static str {
    "iCloud|Google Gmail|Microsoft Outlook|Yahoo Mail|Fastmail|GMX|WEB.DE|Custom"
}

fn provider_from_index(index: u32) -> AccountProvider {
    match index {
        0 => AccountProvider::ICloud,
        1 => AccountProvider::Gmail,
        2 => AccountProvider::Outlook,
        3 => AccountProvider::Yahoo,
        4 => AccountProvider::Fastmail,
        5 => AccountProvider::Gmx,
        6 => AccountProvider::WebDe,
        _ => AccountProvider::Custom,
    }
}

fn provider_to_index(provider: AccountProvider) -> u32 {
    match provider {
        AccountProvider::ICloud => 0,
        AccountProvider::Gmail => 1,
        AccountProvider::Outlook => 2,
        AccountProvider::Yahoo => 3,
        AccountProvider::Fastmail => 4,
        AccountProvider::Gmx => 5,
        AccountProvider::WebDe => 6,
        AccountProvider::Custom => 7,
    }
}

fn detect_provider(email: &str) -> AccountProvider {
    let domain = email.split('@').nth(1).unwrap_or("");
    let domain_lower = to_lower(domain);
    if domain_lower == "icloud.com" || domain_lower == "me.com" || domain_lower == "mac.com" {
        AccountProvider::ICloud
    } else if domain_lower == "gmail.com" || domain_lower == "googlemail.com" {
        AccountProvider::Gmail
    } else if domain_lower == "outlook.com"
        || domain_lower == "hotmail.com"
        || domain_lower == "live.com"
        || domain_lower.ends_with(".outlook.com")
    {
        AccountProvider::Outlook
    } else if domain_lower.starts_with("yahoo.") || domain_lower == "ymail.com" {
        AccountProvider::Yahoo
    } else if domain_lower.contains("fastmail") {
        AccountProvider::Fastmail
    } else if domain_lower.ends_with("gmx.net")
        || domain_lower.ends_with("gmx.de")
        || domain_lower.ends_with("gmx.com")
        || domain_lower.ends_with("gmx.at")
    {
        AccountProvider::Gmx
    } else if domain_lower == "web.de" {
        AccountProvider::WebDe
    } else {
        AccountProvider::Custom
    }
}

fn provider_heading(provider: AccountProvider) -> (&'static str, &'static str, &'static str) {
    match provider {
        AccountProvider::ICloud => (
            "iCloud Mail",
            "Optimized defaults for Apple-hosted mailboxes.",
            "Use your full iCloud address and an app-specific password if two-factor authentication is enabled.",
        ),
        AccountProvider::Gmail => (
            "Google Gmail",
            "Fast setup for Gmail and Google Workspace inboxes.",
            "Gmail often requires an app password when standard password login is enabled for mail apps.",
        ),
        AccountProvider::Outlook => (
            "Microsoft Outlook",
            "Preset for Outlook.com, Hotmail, Live and Microsoft 365.",
            "Your mailbox usually signs in with the full email address on both IMAP and SMTP.",
        ),
        AccountProvider::Yahoo => (
            "Yahoo Mail",
            "Yahoo defaults with secure incoming and outgoing servers.",
            "Yahoo commonly needs an app password for third-party mail clients.",
        ),
        AccountProvider::Fastmail => (
            "Fastmail",
            "Balanced preset for Fastmail-hosted personal and team mail.",
            "Fastmail usually works with your full email address and standard IMAP plus SMTP credentials.",
        ),
        AccountProvider::Gmx => (
            "GMX",
            "GMX defaults tuned for common German-speaking mailbox setups.",
            "Incoming and outgoing login typically use the same mailbox credentials.",
        ),
        AccountProvider::WebDe => (
            "WEB.DE",
            "WEB.DE preset with the expected secure server names.",
            "If login fails, verify whether the account allows external IMAP/SMTP clients.",
        ),
        AccountProvider::Custom => (
            "Custom Mail Provider",
            "Enter your provider manually or start with sensible generic defaults.",
            "Ideal for self-hosted domains, company mail, or providers outside the built-in list.",
        ),
    }
}

fn protocol_to_index(protocol: IncomingProtocol) -> u32 {
    match protocol {
        IncomingProtocol::Imap => 0,
        IncomingProtocol::Pop3 => 1,
    }
}

fn protocol_from_combo(combo: &anyui::ComboBox) -> IncomingProtocol {
    match combo.selected_index().unwrap_or(0) {
        1 => IncomingProtocol::Pop3,
        _ => IncomingProtocol::Imap,
    }
}

fn security_to_index(security: Security) -> u32 {
    match security {
        Security::Tls => 0,
        Security::StartTls => 1,
        Security::None => 2,
    }
}

fn security_from_combo(combo: &anyui::ComboBox) -> Security {
    match combo.selected_index().unwrap_or(0) {
        1 => Security::StartTls,
        2 => Security::None,
        _ => Security::Tls,
    }
}

fn set_widget_text(widget: &impl Widget, text: &str) {
    anyui::Control::from_id(widget.id()).set_text(text);
}

fn get_widget_text(widget: &impl Widget) -> String {
    let ctrl = anyui::Control::from_id(widget.id());
    let mut buf = [0u8; 1024];
    let len = ctrl.get_text(&mut buf);
    String::from(core::str::from_utf8(&buf[..len as usize]).unwrap_or(""))
}

fn set_text_if_empty(widget: &impl Widget, text: &str, force: bool) {
    if force || get_widget_text(widget).is_empty() {
        set_widget_text(widget, text);
    }
}

fn maybe_set_combo(combo: &anyui::ComboBox, idx: u32, force: bool) {
    if force || combo.selected_index().is_none() {
        combo.set_selected_index(Some(idx));
    }
}

fn apply_provider_defaults_to_form(
    provider: AccountProvider,
    email: &str,
    protocol: &anyui::ComboBox,
    incoming_host: &anyui::TextField,
    incoming_port: &anyui::TextField,
    incoming_security: &anyui::ComboBox,
    smtp_host: &anyui::TextField,
    smtp_port: &anyui::TextField,
    smtp_security: &anyui::ComboBox,
    force: bool,
) {
    let mut incoming_port_text = "993";
    let mut incoming_security_mode = Security::Tls;
    let mut smtp_port_text = "587";
    let mut smtp_security_mode = Security::StartTls;
    let (in_host, out_host, proto) = match provider {
        AccountProvider::ICloud => (
            "imap.mail.me.com",
            "smtp.mail.me.com",
            IncomingProtocol::Imap,
        ),
        AccountProvider::Gmail => ("imap.gmail.com", "smtp.gmail.com", IncomingProtocol::Imap),
        AccountProvider::Outlook => (
            "outlook.office365.com",
            "smtp.office365.com",
            IncomingProtocol::Imap,
        ),
        AccountProvider::Yahoo => (
            "imap.mail.yahoo.com",
            "smtp.mail.yahoo.com",
            IncomingProtocol::Imap,
        ),
        AccountProvider::Fastmail => (
            "imap.fastmail.com",
            "smtp.fastmail.com",
            IncomingProtocol::Imap,
        ),
        AccountProvider::Gmx => ("imap.gmx.net", "mail.gmx.net", IncomingProtocol::Imap),
        AccountProvider::WebDe => ("imap.web.de", "smtp.web.de", IncomingProtocol::Imap),
        AccountProvider::Custom => ("imap.example.com", "smtp.example.com", IncomingProtocol::Imap),
    };

    if provider == AccountProvider::Fastmail {
        smtp_port_text = "465";
        smtp_security_mode = Security::Tls;
    }
    if provider == AccountProvider::Custom {
        incoming_port_text = "993";
        incoming_security_mode = Security::Tls;
        smtp_port_text = "587";
        smtp_security_mode = Security::StartTls;
    }

    set_text_if_empty(incoming_host, in_host, force);
    set_text_if_empty(incoming_port, incoming_port_text, force);
    set_text_if_empty(smtp_host, out_host, force);
    set_text_if_empty(smtp_port, smtp_port_text, force);
    maybe_set_combo(protocol, protocol_to_index(proto), force);
    maybe_set_combo(incoming_security, security_to_index(incoming_security_mode), force);
    maybe_set_combo(smtp_security, security_to_index(smtp_security_mode), force);

    if !email.is_empty() && get_widget_text(incoming_host).contains("example.com") {
        let domain = email.split('@').nth(1).unwrap_or("");
        if !domain.is_empty() {
            if provider == AccountProvider::Custom {
                set_text_if_empty(incoming_host, &format!("imap.{}", domain), true);
                set_text_if_empty(smtp_host, &format!("smtp.{}", domain), true);
            }
        }
    }
    if !email.is_empty() && get_widget_text(smtp_host).contains("example.com") && provider == AccountProvider::Custom {
        let domain = email.split('@').nth(1).unwrap_or("");
        if !domain.is_empty() {
            set_text_if_empty(smtp_host, &format!("smtp.{}", domain), true);
        }
    }
}

fn provider_from_selector(listbox: &anyui::ListBox) -> AccountProvider {
    provider_from_index(listbox.selected_index())
}

fn update_provider_copy(
    provider: AccountProvider,
    title: &anyui::Label,
    subtitle: &anyui::Label,
    note: &anyui::Label,
) {
    let (heading, subheading, guidance) = provider_heading(provider);
    title.set_text(heading);
    subtitle.set_text(subheading);
    note.set_text(guidance);
}

fn update_wizard_step(
    step: u32,
    page_provider: &anyui::View,
    page_login: &anyui::View,
    page_review: &anyui::View,
    btn_back: &anyui::Button,
    btn_next: &anyui::Button,
    btn_test: &anyui::Button,
) {
    page_provider.set_visible(step == 0);
    page_login.set_visible(step == 1);
    page_review.set_visible(step == 2);
    btn_back.set_visible(step > 0);
    btn_test.set_visible(step == 2);
    match step {
        0 => btn_next.set_text("Continue"),
        1 => btn_next.set_text("Review"),
        _ => btn_next.set_text("Save Account"),
    }
}

fn message_category(summary: &MessageSummary) -> String {
    if summary.category.is_empty() {
        String::from("Primary")
    } else {
        summary.category.clone()
    }
}

fn thread_depth(msg: &MessageSummary, all: &[MessageSummary]) -> usize {
    let mut depth = 0usize;
    let mut parent_id = if !msg.in_reply_to.is_empty() {
        msg.in_reply_to.as_str()
    } else {
        ""
    };

    while !parent_id.is_empty() && depth < 6 {
        if let Some(parent) = all.iter().find(|m| m.message_id == parent_id) {
            depth += 1;
            if !parent.in_reply_to.is_empty() {
                parent_id = parent.in_reply_to.as_str();
            } else {
                break;
            }
        } else {
            break;
        }
    }
    depth
}

fn threaded_subject(msg: &MessageSummary, all: &[MessageSummary]) -> String {
    let depth = thread_depth(msg, all);
    if depth == 0 {
        return msg.subject.clone();
    }
    let mut out = String::new();
    for _ in 0..depth {
        out.push(' ');
        out.push(' ');
    }
    out.push_str("↳ ");
    out.push_str(&msg.subject);
    out
}

fn domain_of_email(email: &str) -> &str {
    email.split('@').nth(1).unwrap_or("localhost")
}

fn build_account_from_form(
    name_field: &anyui::TextField,
    email_field: &anyui::TextField,
    user_field: &anyui::TextField,
    pass_field: &anyui::TextField,
    proto_combo: &anyui::ComboBox,
    in_host: &anyui::TextField,
    in_port: &anyui::TextField,
    in_sec: &anyui::ComboBox,
    smtp_host: &anyui::TextField,
    smtp_port: &anyui::TextField,
    smtp_sec: &anyui::ComboBox,
) -> Account {
    let mut acc = Account::new();
    acc.display_name = get_widget_text(name_field);
    acc.email = get_widget_text(email_field);
    acc.id = Account::generate_id(&acc.email);
    acc.incoming_protocol = protocol_from_combo(proto_combo);
    acc.incoming_host = get_widget_text(in_host);
    acc.incoming_port = get_widget_text(in_port).parse().unwrap_or(993);
    acc.incoming_security = security_from_combo(in_sec);
    acc.incoming_user = get_widget_text(user_field);
    acc.incoming_pass = get_widget_text(pass_field);
    acc.smtp_host = get_widget_text(smtp_host);
    acc.smtp_port = get_widget_text(smtp_port).parse().unwrap_or(587);
    acc.smtp_security = security_from_combo(smtp_sec);
    acc.smtp_user = if acc.incoming_user.is_empty() {
        acc.email.clone()
    } else {
        acc.incoming_user.clone()
    };
    acc.smtp_pass = acc.incoming_pass.clone();
    acc
}

fn test_account_settings(account: &Account) -> Result<(), String> {
    if account.email.is_empty() {
        return Err(String::from("Please enter an email address."));
    }
    if account.incoming_host.is_empty() {
        return Err(String::from("Incoming server is missing."));
    }

    match account.incoming_protocol {
        IncomingProtocol::Imap => {
            let mut client = connect_imap_client(account)?;
            client.logout();
        }
        IncomingProtocol::Pop3 => {
            let mut client = Pop3Client::new();
            client
                .connect(
                    &account.incoming_host,
                    account.incoming_port,
                    account.incoming_use_tls(),
                )
                .map_err(|e| format!("{:?}", e))?;
            if account.incoming_security == Security::StartTls {
                client.starttls().map_err(|e| format!("{:?}", e))?;
            }
            let user = if account.incoming_user.is_empty() {
                &account.email
            } else {
                &account.incoming_user
            };
            client
                .login(user, &account.incoming_pass)
                .map_err(|e| format!("{:?}", e))?;
            client.quit();
        }
    }

    if !account.smtp_host.is_empty() {
        let mut smtp = SmtpClient::new();
        smtp.connect(&account.smtp_host, account.smtp_port, account.smtp_use_tls())
            .map_err(|e| format!("{:?}", e))?;
        smtp.ehlo(domain_of_email(&account.email))
            .map_err(|e| format!("{:?}", e))?;
        if account.smtp_use_starttls() {
            smtp.starttls().map_err(|e| format!("{:?}", e))?;
            smtp.ehlo(domain_of_email(&account.email))
                .map_err(|e| format!("{:?}", e))?;
        }
        let smtp_user = if account.smtp_user.is_empty() {
            &account.email
        } else {
            &account.smtp_user
        };
        let smtp_pass = if account.smtp_pass.is_empty() {
            &account.incoming_pass
        } else {
            &account.smtp_pass
        };
        if smtp.has_capability("AUTH PLAIN") {
            smtp.auth_plain(smtp_user, smtp_pass)
                .map_err(|e| format!("{:?}", e))?;
        } else if smtp.has_capability("AUTH LOGIN") {
            smtp.auth_login(smtp_user, smtp_pass)
                .map_err(|e| format!("{:?}", e))?;
        }
        smtp.quit();
    }

    Ok(())
}

fn remote_imap_apply_flag(account: &Account, folder: &str, uid: u32, flag: &str, set: bool) -> Result<(), String> {
    let mut client = connect_imap_client(account)?;
    client.select(folder).map_err(|e| format!("{:?}", e))?;
    let action = if set { "+FLAGS" } else { "-FLAGS" };
    client.store_flags(uid, action, flag).map_err(|e| format!("{:?}", e))?;
    client.logout();
    Ok(())
}

fn remote_imap_move(account: &Account, from_folder: &str, uid: u32, to_folder: &str) -> Result<(), String> {
    let mut client = connect_imap_client(account)?;
    client.select(from_folder).map_err(|e| format!("{:?}", e))?;
    client.move_message(uid, to_folder).map_err(|e| format!("{:?}", e))?;
    client.logout();
    Ok(())
}

fn remote_imap_append(account: &Account, folder: &str, message: &[u8]) -> Result<(), String> {
    let mut client = connect_imap_client(account)?;
    client.append(folder, "\\Seen", message).map_err(|e| format!("{:?}", e))?;
    client.logout();
    Ok(())
}

fn connect_imap_client(account: &Account) -> Result<ImapClient, String> {
    let mut client = ImapClient::new();
    client
        .connect(
            &account.incoming_host,
            account.incoming_port,
            account.incoming_use_tls(),
        )
        .map_err(|e| format!("{:?}", e))?;
    if account.incoming_security == Security::StartTls {
        client.starttls().map_err(|e| format!("{:?}", e))?;
    }
    client
        .login(&account.incoming_user, &account.incoming_pass)
        .map_err(|e| format!("{:?}", e))?;
    Ok(client)
}

fn category_matches(summary: &MessageSummary, filter: CategoryFilter) -> bool {
    let category = message_category(summary);
    match filter {
        CategoryFilter::All => true,
        CategoryFilter::Primary => category == "Primary",
        CategoryFilter::Transactions => category == "Transactions",
        CategoryFilter::Updates => category == "Updates",
        CategoryFilter::Promotions => category == "Promotions",
    }
}

fn category_color(category: &str) -> u32 {
    match category {
        "Primary" => tc().accent,
        "Transactions" => tc().success,
        "Updates" => tc().accent_hover,
        "Promotions" => tc().warning,
        _ => tc().text_secondary,
    }
}

fn update_category_summary() {
    let a = app();
    let mut primary = 0usize;
    let mut transactions = 0usize;
    let mut updates = 0usize;
    let mut promotions = 0usize;
    let mut unread = 0usize;

    for msg in &a.all_messages {
        if !msg.is_seen() {
            unread += 1;
        }
        match message_category(msg).as_str() {
            "Transactions" => transactions += 1,
            "Updates" => updates += 1,
            "Promotions" => promotions += 1,
            _ => primary += 1,
        }
    }

    a.category_summary.set_text(&format!(
        "Focus {} | T {} | U {} | P {} | Unread {}",
        primary, transactions, updates, promotions, unread
    ));
}

// ═══════════════════════════════════════════════════════════════════════════
// Entry Point
// ═══════════════════════════════════════════════════════════════════════════

fn main() {
    if !anyui::init() {
        return;
    }
    anyos_std::i18n::init();
    let _ = libdb_client::init();

    let home = home_dir();
    let default_base_dir = format!("{}/.anymail", home);
    let stored_ref = storage::schema::schema().read_external_ref("state/mail_store_ref");
    let base_dir = storage::maildir::prepare_base_dir(stored_ref.as_deref(), &default_base_dir)
        .unwrap_or_else(|| String::from("/tmp/.anymail"));
    let config_path = format!("{}/accounts.json", base_dir);
    let contacts_path = format!("{}/contacts.json", base_dir);
    let _ = storage::schema::schema().write_external_ref("state/mail_store_ref", &base_dir);

    // Load configuration
    let config = MailConfig::load(&config_path);
    let address_book = AddressBook::load(&contacts_path);
    let colors = tc();
    let mut storage_notice = if let Some(old_ref) = stored_ref {
        if old_ref != base_dir {
            format!("Mail storage path repaired: {}", base_dir)
        } else {
            String::new()
        }
    } else {
        String::new()
    };
    for account in &config.accounts {
        if !maildir::ensure_dirs(&base_dir, &account.id) && storage_notice.is_empty() {
            storage_notice = format!("Mail storage validation failed under {}", base_dir);
        }
    }

    // ── Window ─────────────────────────────────────────────────────────
    let t = anyos_std::i18n::t;
    let win = anyui::Window::new("anyMail", -1, -1, 1100, 700);
    win.set_color(colors.window_bg);

    // ── Toolbar (DOCK_TOP) ─────────────────────────────────────────────
    let toolbar = anyui::Toolbar::new();
    toolbar.set_dock(anyui::DOCK_TOP);
    toolbar.set_size(1100, 46);
    toolbar.set_color(colors.toolbar_bg);
    toolbar.set_padding(4, 4, 4, 4);

    // Compose/Reply buttons with icons (34x34 with tooltips, no inline text)
    let btn_new = toolbar.add_icon_button("");
    btn_new.set_size(34, 34);
    btn_new.set_icon(anyui::ICON_NEW_FILE);
    btn_new.set_tooltip(t("New"));

    let btn_reply = toolbar.add_icon_button("");
    btn_reply.set_size(34, 34);
    btn_reply.set_system_icon("arrow-back-up", anyui::IconType::Outline, colors.text, 24);
    btn_reply.set_tooltip(t("Reply"));

    let btn_reply_all = toolbar.add_icon_button("");
    btn_reply_all.set_size(34, 34);
    btn_reply_all.set_system_icon("corner-up-left", anyui::IconType::Outline, colors.text, 24);
    btn_reply_all.set_tooltip(t("Reply All"));
    btn_reply_all.set_visible(false);

    let btn_forward = toolbar.add_icon_button("");
    btn_forward.set_size(34, 34);
    btn_forward.set_system_icon("mail-forward", anyui::IconType::Outline, colors.text, 24);
    btn_forward.set_tooltip(t("Forward"));
    btn_forward.set_visible(false);

    toolbar.add_separator();

    // Message management buttons
    let btn_archive = toolbar.add_icon_button("");
    btn_archive.set_size(34, 34);
    btn_archive.set_system_icon("archive", anyui::IconType::Outline, colors.text, 24);
    btn_archive.set_tooltip(t("Archive"));

    let btn_junk = toolbar.add_icon_button("");
    btn_junk.set_size(34, 34);
    btn_junk.set_system_icon("mail-x", anyui::IconType::Outline, colors.text, 24);
    btn_junk.set_tooltip(t("Junk"));
    btn_junk.set_visible(false);

    let btn_delete = toolbar.add_icon_button("");
    btn_delete.set_size(34, 34);
    btn_delete.set_system_icon("trash", anyui::IconType::Outline, colors.text, 24);
    btn_delete.set_tooltip(t("Delete"));

    toolbar.add_separator();

    // Account/Sync buttons
    let btn_getmail = toolbar.add_icon_button("");
    btn_getmail.set_size(34, 34);
    btn_getmail.set_system_icon("refresh", anyui::IconType::Outline, colors.text, 24);
    btn_getmail.set_tooltip(t("Get Mail"));

    let btn_accounts = toolbar.add_icon_button("");
    btn_accounts.set_size(34, 34);
    btn_accounts.set_system_icon("settings", anyui::IconType::Outline, colors.text, 24);
    btn_accounts.set_tooltip(t("Accounts"));

    let btn_contacts = toolbar.add_icon_button("");
    btn_contacts.set_size(34, 34);
    btn_contacts.set_system_icon("address-book", anyui::IconType::Outline, colors.text, 24);
    btn_contacts.set_tooltip(t("Contacts"));
    btn_contacts.set_visible(false);

    win.add(&toolbar);

    // ── Filter Bar (DOCK_TOP) ──────────────────────────────────────────
    let filter_bar = anyui::View::new();
    filter_bar.set_dock(anyui::DOCK_TOP);
    filter_bar.set_size(1100, 58);
    filter_bar.set_color(colors.card_bg);
    filter_bar.set_padding(8, 6, 8, 6);

    let search_field = anyui::SearchField::new();
    search_field.set_placeholder(t("Search messages..."));
    search_field.set_position(8, 6);
    search_field.set_size(310, 24);
    filter_bar.add(&search_field);

    let category_summary = anyui::Label::new("Focus 0 | T 0 | U 0 | P 0 | Unread 0");
    category_summary.set_position(640, 8);
    category_summary.set_size(430, 18);
    category_summary.set_text_color(colors.text_secondary);
    category_summary.set_font_size(12);
    filter_bar.add(&category_summary);

    let filter_all = anyui::Button::new(t("All Mail"));
    filter_all.set_position(8, 30);
    filter_all.set_size(86, 22);
    filter_bar.add(&filter_all);

    let filter_primary = anyui::Button::new(t("Primary"));
    filter_primary.set_position(100, 30);
    filter_primary.set_size(82, 22);
    filter_bar.add(&filter_primary);

    let filter_transactions = anyui::Button::new(t("Transactions"));
    filter_transactions.set_position(188, 30);
    filter_transactions.set_size(98, 22);
    filter_bar.add(&filter_transactions);

    let filter_updates = anyui::Button::new(t("Updates"));
    filter_updates.set_position(292, 30);
    filter_updates.set_size(78, 22);
    filter_bar.add(&filter_updates);

    let filter_promotions = anyui::Button::new(t("Promotions"));
    filter_promotions.set_position(376, 30);
    filter_promotions.set_size(94, 22);
    filter_bar.add(&filter_promotions);

    let filter_unread = anyui::Button::new(t("Unread"));
    filter_unread.set_position(640, 30);
    filter_unread.set_size(70, 22);
    filter_unread.set_visible(false);
    filter_bar.add(&filter_unread);

    let filter_starred = anyui::Button::new(t("Starred"));
    filter_starred.set_position(716, 30);
    filter_starred.set_size(76, 22);
    filter_starred.set_visible(false);
    filter_bar.add(&filter_starred);

    let filter_attach = anyui::Button::new(t("Attach"));
    filter_attach.set_position(798, 30);
    filter_attach.set_size(74, 22);
    filter_attach.set_visible(false);
    filter_bar.add(&filter_attach);

    win.add(&filter_bar);

    // ── Status Bar (DOCK_BOTTOM) ───────────────────────────────────────
    let status_bar = anyui::View::new();
    status_bar.set_dock(anyui::DOCK_BOTTOM);
    status_bar.set_size(1100, 24);
    status_bar.set_color(colors.toolbar_bg);
    status_bar.set_padding(8, 2, 8, 2);

    let status_label = anyui::Label::new(t("Ready"));
    status_label.set_dock(anyui::DOCK_FILL);
    status_label.set_text_color(colors.text_secondary);
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
    folder_tree.set_color(colors.sidebar_bg);
    folder_tree.set_text_color(colors.text);
    folder_tree.set_row_height(22);
    main_split.add(&folder_tree);

    let folder_ctx_menu = anyui::ContextMenu::new("Refresh Folder|Check Mail|-|Compose");
    folder_tree.set_context_menu(&folder_ctx_menu);
    main_split.add(&folder_ctx_menu);

    // Right panel: vertical split (message grid 40% | preview 60%)
    let content_split = anyui::SplitView::new();
    content_split.set_orientation(anyui::ORIENTATION_VERTICAL);
    content_split.set_split_ratio(40);

    // Message grid (top of right)
    let msg_grid = anyui::DataGrid::new(800, 300);
    msg_grid.set_columns(&[
        anyui::ColumnDef::new("").width(30), // Flags (star/unread)
        anyui::ColumnDef::new(t("From")).width(180),
        anyui::ColumnDef::new(t("Subject")).width(300),
        anyui::ColumnDef::new(t("Category")).width(110),
        anyui::ColumnDef::new(t("Date")).width(120),
        anyui::ColumnDef::new(t("Size"))
            .width(70)
            .align(anyui::ALIGN_RIGHT),
    ]);
    msg_grid.set_row_height(22);
    msg_grid.set_header_height(24);
    msg_grid.set_color(colors.editor_bg);
    msg_grid.set_text_color(colors.text);
    content_split.add(&msg_grid);

    // Preview pane (bottom of right)
    let preview_panel = anyui::View::new();

    // Preview header (from/to/subject/date)
    let preview_header = anyui::Label::new("");
    preview_header.set_dock(anyui::DOCK_TOP);
    preview_header.set_size(800, 74);
    preview_header.set_color(colors.card_bg);
    preview_header.set_text_color(colors.text_secondary);
    preview_header.set_font_size(12);
    preview_header.set_padding(8, 4, 8, 4);
    preview_panel.add(&preview_header);

    // Preview body
    let preview_body = anyui::TextEditor::new(800, 350);
    preview_body.set_dock(anyui::DOCK_FILL);
    preview_body.set_color(colors.editor_bg);
    preview_body.set_text_color(colors.text);
    preview_body.set_show_line_numbers(false);
    preview_panel.add(&preview_body);

    content_split.add(&preview_panel);
    main_split.add(&content_split);

    win.add(&main_split);

    // ── Context menu for message grid ──────────────────────────────────
    let ctx_items = format!(
        "{}|{}|{}|{}|{}|{}|{}|{}",
        t("Mark as Read"),
        t("Mark as Unread"),
        t("Toggle Star"),
        t("Mark as Junk"),
        t("Move to Trash"),
        t("Move to Archive"),
        t("Copy to Folder..."),
        t("Delete Permanently")
    );
    let ctx_menu = anyui::ContextMenu::new(&ctx_items);
    msg_grid.set_context_menu(&ctx_menu);

    // ── Initialize state ───────────────────────────────────────────────
    let has_accounts = !config.accounts.is_empty();

    unsafe {
        APP = Some(AppState {
            base_dir: base_dir.clone(),
            config,
            address_book,
            win: win.clone(),
            toolbar: toolbar.clone(),
            filter_bar: filter_bar.clone(),
            search_field: search_field.clone(),
            filter_all: filter_all.clone(),
            filter_primary: filter_primary.clone(),
            filter_transactions: filter_transactions.clone(),
            filter_updates: filter_updates.clone(),
            filter_promotions: filter_promotions.clone(),
            filter_unread: filter_unread.clone(),
            filter_starred: filter_starred.clone(),
            filter_attach: filter_attach.clone(),
            category_summary: category_summary.clone(),
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
            category_filter: CategoryFilter::All,
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
    menu_bar.on_item(|e| match e.item_id {
        1 => on_get_mail(),
        2 => anyui::quit(),
        10 => open_compose(ComposeMode::New),
        11 => open_compose(ComposeMode::Reply),
        12 => open_compose(ComposeMode::Forward),
        13 => on_delete(),
        20 => open_account_setup(false),
        21 => open_contacts(),
        _ => {}
    });

    // ── Populate folder tree ───────────────────────────────────────────
    populate_folder_tree();

    // ── Wire up events ─────────────────────────────────────────────────
    btn_new.on_click(|_| {
        open_compose(ComposeMode::New);
    });
    btn_reply.on_click(|_| {
        open_compose(ComposeMode::Reply);
    });
    btn_reply_all.on_click(|_| {
        open_compose(ComposeMode::ReplyAll);
    });
    btn_forward.on_click(|_| {
        open_compose(ComposeMode::Forward);
    });
    btn_delete.on_click(|_| {
        on_delete();
    });
    btn_junk.on_click(|_| {
        on_mark_junk();
    });
    btn_archive.on_click(|_| {
        on_archive();
    });
    btn_getmail.on_click(|_| {
        on_get_mail();
    });
    btn_accounts.on_click(|_| {
        open_account_setup(false);
    });
    btn_contacts.on_click(|_| {
        open_contacts();
    });

    search_field.on_text_changed(|e| {
        let mut buf = [0u8; 256];
        let len = app().search_field.get_text(&mut buf);
        app().filter_text = String::from(core::str::from_utf8(&buf[..len as usize]).unwrap_or(""));
        apply_filters();
    });

    filter_all.on_click(|_| {
        app().category_filter = CategoryFilter::All;
        apply_filters();
    });

    filter_primary.on_click(|_| {
        app().category_filter = CategoryFilter::Primary;
        apply_filters();
    });

    filter_transactions.on_click(|_| {
        app().category_filter = CategoryFilter::Transactions;
        apply_filters();
    });

    filter_updates.on_click(|_| {
        app().category_filter = CategoryFilter::Updates;
        apply_filters();
    });

    filter_promotions.on_click(|_| {
        app().category_filter = CategoryFilter::Promotions;
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

    folder_ctx_menu.on_item_click(|e| match e.index {
        0 => {
            load_folder_messages();
            let a = app();
            trigger_folder_sync(a.current_account, &a.current_folder.clone());
        }
        1 => on_get_mail(),
        3 => open_compose(ComposeMode::New),
        _ => {}
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
                open_account_setup(false);
            }
        }
    });

    win.on_close(|_| {
        anyui::quit();
    });

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

    if !storage_notice.is_empty() {
        set_status(&storage_notice);
    } else if has_accounts {
        set_status(t("Ready"));
    } else {
        set_status("Start by adding a mail account.");
    }

    if !has_accounts {
        open_account_setup(true);
    }

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
        let root = a
            .folder_tree
            .add_root(anyos_std::i18n::t("No accounts configured"));
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
        let default_folders = ["INBOX", "Sent", "Drafts", "Archive", "Trash", "Spam"];
        let special_uses = [
            SpecialUse::Inbox,
            SpecialUse::Sent,
            SpecialUse::Drafts,
            SpecialUse::Archive,
            SpecialUse::Trash,
            SpecialUse::Spam,
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
    if a.current_account >= a.config.accounts.len() {
        return;
    }

    let account = &a.config.accounts[a.current_account];
    let idx_path = maildir::index_path(&a.base_dir, &account.id, &a.current_folder);
    a.all_messages = maildir::load_index(&idx_path);
    a.selected_msg_idx = None;
    a.current_full_msg = None;

    // Clear preview
    a.preview_header.set_text("");
    a.preview_body.set_text("");

    update_category_summary();
    apply_filters();

    let total = a.all_messages.len();
    let unread = a.all_messages.iter().filter(|m| !m.is_seen()).count();
    let t = anyos_std::i18n::t;
    set_status(&format!(
        "{} - {} {}, {} {}",
        a.current_folder,
        total,
        t("messages"),
        unread,
        t("unread")
    ));
}

// ═══════════════════════════════════════════════════════════════════════════
// Filtering & Grid Display
// ═══════════════════════════════════════════════════════════════════════════

fn apply_filters() {
    let a = app();
    let query = a.filter_text.clone();
    let query_lower = to_lower(&query);
    let category_filter = a.category_filter;
    let show_unread = a.filter_unread_on;
    let show_starred = a.filter_starred_on;
    let show_attach = a.filter_attach_on;
    let source: Vec<MessageSummary> = if !query_lower.is_empty() && a.current_account < a.config.accounts.len() {
        let account = &a.config.accounts[a.current_account];
        maildir::search_messages(&a.base_dir, &account.id, Some(&a.current_folder), &query)
    } else {
        a.all_messages.clone()
    };

    a.messages = source
        .into_iter()
        .filter(|m| {
            if !query_lower.is_empty() {
                let from_match = to_lower(m.from.display_short()).contains(&query_lower);
                let subj_match = to_lower(&m.subject).contains(&query_lower);
                let prev_match = to_lower(&m.preview).contains(&query_lower);
                let ref_match = to_lower(&m.references).contains(&query_lower);
                if !from_match && !subj_match && !prev_match && !ref_match {
                    return false;
                }
            }
            if !category_matches(m, category_filter) {
                return false;
            }
            if show_unread && m.is_seen() {
                return false;
            }
            if show_starred && !m.is_flagged() {
                return false;
            }
            if show_attach && !m.has_attachment() {
                return false;
            }
            true
        })
        .collect();

    let unread = a.all_messages.iter().filter(|m| !m.is_seen()).count();
    let filter_text = format!(
        "{} | {} shown | {} unread",
        category_label(category_filter),
        a.messages.len(),
        unread
    );
    a.category_summary.set_text(&filter_text);
    refresh_grid();
}

fn refresh_grid() {
    let a = app();
    let colors = tc();
    let count = a.messages.len();
    a.msg_grid.set_row_count(count as u32);

    if count == 0 {
        return;
    }

    // Build cell data and colors
    let mut text_colors = vec![colors.text_secondary; count * NUM_GRID_COLS];
    let mut bg_colors = vec![0u32; count * NUM_GRID_COLS];

    for (row, msg) in a.messages.iter().enumerate() {
        // Flags column
        let flags_str = {
            let mut s = String::new();
            if msg.is_flagged() {
                s.push('*');
            }
            if !msg.is_seen() {
                s.push('!');
            }
            if msg.has_attachment() {
                s.push('@');
            }
            s
        };
        a.msg_grid.set_cell(row as u32, 0, &flags_str);

        // From
        a.msg_grid.set_cell(row as u32, 1, msg.from.display_short());

        // Subject
        let subject = threaded_subject(msg, &a.all_messages);
        a.msg_grid.set_cell(row as u32, 2, &subject);

        // Category
        let category = message_category(msg);
        a.msg_grid.set_cell(row as u32, 3, &category);

        // Date
        let date_short = crate::mail::rfc2822::format_date_short(&msg.date);
        a.msg_grid.set_cell(row as u32, 4, &date_short);

        // Size
        let size_str = crate::mail::rfc2822::format_size(msg.size);
        a.msg_grid.set_cell(row as u32, 5, &size_str);

        // Determine text color based on message state
        let row_color = if msg.is_deleted() {
            colors.text_disabled
        } else if msg.is_junk() {
            colors.warning
        } else if msg.is_flagged() {
            colors.warning
        } else if !msg.is_seen() {
            colors.text
        } else {
            colors.text_secondary
        };

        for col in 0..NUM_GRID_COLS {
            text_colors[row * NUM_GRID_COLS + col] = row_color;
        }
        text_colors[row * NUM_GRID_COLS + 3] = category_color(&category);
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
        "{}: {}\n{}: {}\n{}: {}\n{}: {}\n{}: {}",
        t("From"),
        msg.from.to_header_string(),
        t("To"),
        format_to_list(&msg.to),
        t("Subject"),
        msg.subject,
        t("Category"),
        message_category(msg),
        t("Date"),
        crate::mail::rfc2822::format_date_short(&msg.date)
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
                a.preview_body
                    .set_text(anyos_std::i18n::t("(No message body)"));
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
    if idx >= a.messages.len() {
        return;
    }

    let uid = a.messages[idx].uid;
    a.messages[idx].flags |= FLAG_SEEN;

    let mut remote_err = None;
    if a.current_account < a.config.accounts.len() {
        let account = a.config.accounts[a.current_account].clone();
        if account.is_imap() && a.current_folder != "Sent" && a.current_folder != "Drafts" {
            remote_err = remote_imap_apply_flag(&account, &a.current_folder, uid, "\\Seen", true).err();
        }
    }

    for m in &mut a.all_messages {
        if m.uid == uid {
            m.flags |= FLAG_SEEN;
            break;
        }
    }

    // Save index
    save_current_index();
    refresh_grid();
    if let Some(err) = remote_err {
        set_status(&format!("Seen synced locally, IMAP pending: {}", err));
    }
}

fn save_current_index() {
    let a = app();
    if a.current_account >= a.config.accounts.len() {
        return;
    }
    let account = &a.config.accounts[a.current_account];
    let idx_path = maildir::index_path(&a.base_dir, &account.id, &a.current_folder);
    maildir::save_index(&idx_path, &a.all_messages);
    update_category_summary();
}

fn move_selected_message_to(target_folder: &str, extra_flags: u32, status_text: &str) {
    let a = app();
    let Some(idx) = a.selected_msg_idx else {
        return;
    };
    if idx >= a.messages.len() || a.current_account >= a.config.accounts.len() {
        return;
    }

    let account = a.config.accounts[a.current_account].clone();
    let current_folder = a.current_folder.clone();
    let uid = a.messages[idx].uid;
    let remote_result = if account.is_imap()
        && current_folder != "Drafts"
        && current_folder != "Sent"
        && current_folder != target_folder
    {
        remote_imap_move(&account, &current_folder, uid, target_folder).err()
    } else {
        None
    };

    let Some(existing_idx) = a.all_messages.iter().position(|m| m.uid == uid) else {
        return;
    };

    let mut moved = a.all_messages[existing_idx].clone();
    moved.flags |= extra_flags;
    moved.category = maildir::classify_message(&moved, target_folder);

    if current_folder != target_folder {
        let target_path = maildir::index_path(&a.base_dir, &account.id, target_folder);
        let mut target_messages = maildir::load_index(&target_path);
        target_messages.retain(|m| m.uid != uid);
        target_messages.push(moved.clone());
        maildir::save_index(&target_path, &target_messages);
        maildir::move_message(&a.base_dir, &account.id, &current_folder, target_folder, uid);
    }

    a.all_messages.remove(existing_idx);
    save_current_index();
    a.selected_msg_idx = None;
    a.current_full_msg = None;
    a.preview_header.set_text("");
    a.preview_body.set_text("");
    apply_filters();
    if let Some(err) = remote_result {
        set_status(&format!("{} ({})", status_text, err));
    } else {
        set_status(status_text);
    }
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
                save_current_index();
                update_category_summary();
                a.selected_msg_idx = None;
                a.current_full_msg = None;
                a.preview_header.set_text("");
                a.preview_body.set_text("");
                apply_filters();
                set_status(anyos_std::i18n::t("Message deleted"));
            } else {
                move_selected_message_to("Trash", FLAG_DELETED, anyos_std::i18n::t("Message moved to Trash"));
            }
        }
    }
}

fn on_mark_junk() {
    let a = app();
    if let Some(idx) = a.selected_msg_idx {
        if idx < a.messages.len() {
            let uid = a.messages[idx].uid;
            let mut is_junk_now = false;
            for m in &mut a.all_messages {
                if m.uid == uid {
                    m.flags ^= FLAG_JUNK;
                    is_junk_now = m.is_junk();
                    m.category = maildir::classify_message(m, &a.current_folder);
                    break;
                }
            }
            save_current_index();
            apply_filters();
            if a.current_account < a.config.accounts.len() {
                let account = a.config.accounts[a.current_account].clone();
                if account.is_imap() {
                    if is_junk_now && a.current_folder != "Spam" {
                        if let Err(err) = remote_imap_move(&account, &a.current_folder, uid, "Spam") {
                            set_status(&format!("Marked junk locally, IMAP pending: {}", err));
                        }
                    }
                }
            }
        }
    }
}

fn on_archive() {
    if app().current_folder == "Archive" {
        return;
    }
    move_selected_message_to("Archive", FLAG_SEEN, anyos_std::i18n::t("Message archived"));
}

fn on_context_action(index: u32) {
    match index {
        0 => {
            // Mark as Read
            if let Some(idx) = app().selected_msg_idx {
                mark_message_flag(idx, FLAG_SEEN, true);
            }
        }
        1 => {
            // Mark as Unread
            if let Some(idx) = app().selected_msg_idx {
                mark_message_flag(idx, FLAG_SEEN, false);
            }
        }
        2 => {
            // Toggle Star
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
    if idx >= a.messages.len() {
        return;
    }
    let uid = a.messages[idx].uid;
    let folder = a.current_folder.clone();
    let mut remote_err = None;

    if set {
        a.messages[idx].flags |= flag;
    } else {
        a.messages[idx].flags &= !flag;
    }

    for m in &mut a.all_messages {
        if m.uid == uid {
            if set {
                m.flags |= flag;
            } else {
                m.flags &= !flag;
            }
            break;
        }
    }

    save_current_index();
    refresh_grid();

    if a.current_account < a.config.accounts.len() {
        let account = a.config.accounts[a.current_account].clone();
        if account.is_imap() {
            remote_err = match flag {
                FLAG_SEEN => remote_imap_apply_flag(&account, &folder, uid, "\\Seen", set).err(),
                FLAG_FLAGGED => remote_imap_apply_flag(&account, &folder, uid, "\\Flagged", set).err(),
                _ => None,
            };
        }
    }
    if let Some(err) = remote_err {
        set_status(&format!("Updated locally, IMAP pending: {}", err));
    }
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
        set_status(anyos_std::i18n::t(
            "No accounts configured. Click 'Accounts' to add one.",
        ));
        return;
    }

    let ss = sync_worker::sync_state();

    // If already running, cancel instead
    if ss.worker_active.load(core::sync::atomic::Ordering::Relaxed) {
        ss.cancel_requested
            .store(true, core::sync::atomic::Ordering::Release);
        set_status(anyos_std::i18n::t("Cancelling sync..."));
        return;
    }

    // Prepare sync input
    ss.acquire();
    ss.accounts = a.config.accounts.clone();
    ss.base_dir = a.base_dir.clone();
    ss.target_account_idx = None; // sync all accounts
    ss.target_folder = None; // sync default folders
    ss.uid_offset = 0;
    ss.fetch_limit = sync_worker::LAZY_BATCH_SIZE;
    ss.reset_output();
    ss.release();

    ss.worker_active
        .store(true, core::sync::atomic::Ordering::Release);
    ss.phase.store(
        SyncPhase::Connecting as u32,
        core::sync::atomic::Ordering::Release,
    );

    // Spawn background thread (128KB stack for network buffers + TLS)
    spawn_detached(sync_worker::sync_worker_entry, 128 * 1024, "mail-sync");

    a.btn_getmail.set_text(anyos_std::i18n::t("Cancel"));
    set_status(anyos_std::i18n::t("Syncing mail..."));
}

/// Trigger background fetch for a specific folder (used on folder selection).
fn trigger_folder_sync(acct_idx: usize, folder_name: &str) {
    let a = app();
    if acct_idx >= a.config.accounts.len() {
        return;
    }

    let ss = sync_worker::sync_state();
    if ss.worker_active.load(core::sync::atomic::Ordering::Relaxed) {
        return;
    }

    ss.acquire();
    ss.accounts = a.config.accounts.clone();
    ss.base_dir = a.base_dir.clone();
    ss.target_account_idx = Some(acct_idx);
    ss.target_folder = Some(String::from(folder_name));
    ss.uid_offset = a.all_messages.iter().map(|m| m.uid).max().unwrap_or(0);
    ss.fetch_limit = sync_worker::LAZY_BATCH_SIZE;
    ss.reset_output();
    ss.release();

    ss.worker_active
        .store(true, core::sync::atomic::Ordering::Release);
    ss.phase.store(
        SyncPhase::Connecting as u32,
        core::sync::atomic::Ordering::Release,
    );

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
            let is_current = a
                .config
                .accounts
                .get(a.current_account)
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
                    set_status(&format!(
                        "{} - {} {} ({} {}), {} {}",
                        a.current_folder,
                        total,
                        t("messages"),
                        a.folder_total_count,
                        t("on server"),
                        unread,
                        t("unread")
                    ));
                } else {
                    set_status(&format!(
                        "{} - {} {}, {} {}",
                        a.current_folder,
                        total,
                        t("messages"),
                        unread,
                        t("unread")
                    ));
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
        set_status(&format!(
            "{} ({})",
            anyos_std::i18n::t("Mail check complete"),
            now_string()
        ));
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
    if !a.has_more_messages || a.loading_more {
        return;
    }

    let ss = sync_worker::sync_state();
    if ss.worker_active.load(core::sync::atomic::Ordering::Relaxed) {
        return;
    }

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

        ss.worker_active
            .store(true, core::sync::atomic::Ordering::Release);
        ss.phase.store(
            SyncPhase::Connecting as u32,
            core::sync::atomic::Ordering::Release,
        );

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
    let default_names = ["INBOX", "Sent", "Drafts", "Archive", "Trash", "Spam"];
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
        anyui::MessageBox::show(
            anyui::MessageBoxType::Warning,
            anyos_std::i18n::t("No email account configured.\nPlease add an account first."),
            Some("OK"),
        );
        return;
    }

    let account = &a.config.accounts[a.current_account];

    // Prepare compose data based on mode
    let (to_str, cc_str, subject, body, in_reply_to, references) = match mode {
        ComposeMode::New => (
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
        ),
        ComposeMode::Reply | ComposeMode::ReplyAll => {
            if let Some(ref full) = a.current_full_msg {
                let (to, cc, subj, body, irt, refs) =
                    crate::mail::compose::prepare_reply(full, mode == ComposeMode::ReplyAll);
                let to_str = to
                    .iter()
                    .map(|addr: &EmailAddress| addr.to_header_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                let cc_str = cc
                    .iter()
                    .map(|addr: &EmailAddress| addr.to_header_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                (to_str, cc_str, subj, body, irt, refs)
            } else {
                return;
            }
        }
        ComposeMode::Forward => {
            if let Some(ref full) = a.current_full_msg {
                let (subj, body) = crate::mail::compose::prepare_forward(full);
                (
                    String::new(),
                    String::new(),
                    subj,
                    body,
                    String::new(),
                    String::new(),
                )
            } else {
                return;
            }
        }
        ComposeMode::Draft => {
            if let Some(ref full) = a.current_full_msg {
                let to_str = full
                    .summary
                    .to
                    .iter()
                    .map(|addr: &EmailAddress| addr.to_header_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                let cc_str = full
                    .cc
                    .iter()
                    .map(|addr: &EmailAddress| addr.to_header_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                (
                    to_str,
                    cc_str,
                    full.summary.subject.clone(),
                    full.text_body.clone(),
                    String::new(),
                    String::new(),
                )
            } else {
                return;
            }
        }
    };

    // Create compose window
    let t = anyos_std::i18n::t;
    let compose_win = anyui::Window::new(t("Compose Message"), -1, -1, 700, 550);
    let colors = tc();
    compose_win.set_color(colors.window_bg);

    // Toolbar
    let comp_toolbar = anyui::Toolbar::new();
    comp_toolbar.set_dock(anyui::DOCK_TOP);
    comp_toolbar.set_size(700, 46);
    comp_toolbar.set_color(colors.toolbar_bg);
    comp_toolbar.set_padding(4, 4, 4, 4);

    let btn_send = comp_toolbar.add_icon_button("");
    btn_send.set_size(34, 34);
    btn_send.set_system_icon("mail-fast", IconType::Outline, colors.text, 24);
    btn_send.set_tooltip(t("Send"));

    let btn_attach = comp_toolbar.add_icon_button("");
    btn_attach.set_size(34, 34);
    btn_attach.set_system_icon("paperclip", IconType::Outline, colors.text, 24);
    btn_attach.set_tooltip(t("Attach"));

    let btn_save_draft = comp_toolbar.add_icon_button("");
    btn_save_draft.set_size(34, 34);
    btn_save_draft.set_system_icon("device-floppy", IconType::Outline, colors.text, 24);
    btn_save_draft.set_tooltip(t("Save Draft"));

    compose_win.add(&comp_toolbar);

    // Header fields
    let header_panel = anyui::View::new();
    header_panel.set_dock(anyui::DOCK_TOP);
    header_panel.set_size(700, 120);
    header_panel.set_color(colors.card_bg);
    header_panel.set_padding(8, 4, 8, 4);

    // From label
    let from_label = anyui::Label::new(&format!("{}: {}", t("From"), account.email));
    from_label.set_position(8, 6);
    from_label.set_size(680, 20);
    from_label.set_text_color(colors.text_secondary);
    header_panel.add(&from_label);

    // To field (with autocomplete)
    let to_label = anyui::Label::new(&format!("{}:", t("To")));
    to_label.set_position(8, 30);
    to_label.set_size(40, 20);
    to_label.set_text_color(colors.text_secondary);
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
    cc_label.set_text_color(colors.text_secondary);
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
    subj_label.set_text_color(colors.text_secondary);
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
    body_editor.set_color(colors.editor_bg);
    body_editor.set_text_color(colors.text);
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
        on_send(
            &to_field,
            &cc_field,
            &subj_field,
            &body_editor,
            acct_idx,
            &irt,
            &refs,
        );
    });

    btn_attach.on_click(|_| {
        if let Some(path) = anyui::FileDialog::open_file() {
            // Attachment handling - load file and add to message
            // For now just notify the user
            anyui::MessageBox::show(
                anyui::MessageBoxType::Info,
                &format!("{}: {}", anyos_std::i18n::t("Attachment added"), path),
                Some("OK"),
            );
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
    if acct_idx >= a.config.accounts.len() {
        return;
    }
    let account = a.config.accounts[acct_idx].clone();

    // Read fields
    let to_str = get_field_text(to_field);
    let cc_str = get_field_text(cc_field);
    let subj_str = get_field_text(subj_field);

    let mut body_buf = vec![0u8; 64 * 1024];
    let body_len = body_editor.get_text(&mut body_buf);
    let body_str = String::from(core::str::from_utf8(&body_buf[..body_len as usize]).unwrap_or(""));

    if to_str.is_empty() {
        anyui::MessageBox::show(
            anyui::MessageBoxType::Warning,
            anyos_std::i18n::t("Please enter a recipient."),
            Some("OK"),
        );
        return;
    }

    // Parse addresses
    let from = EmailAddress::with_name(&account.display_name, &account.email);
    let to_addrs = crate::mail::rfc2822::parse_address_list(&to_str);
    let cc_addrs = crate::mail::rfc2822::parse_address_list(&cc_str);

    // Build MIME message
    let message_data = crate::mail::compose::build_message(
        &from,
        &to_addrs,
        &cc_addrs,
        &subj_str,
        &body_str,
        "",
        &[],
        in_reply_to,
        references,
    );

    // Connect to SMTP
    set_status(&format!(
        "Sending via {}:{}...",
        account.smtp_host, account.smtp_port
    ));

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
    let user = if account.smtp_user.is_empty() {
        &account.incoming_user
    } else {
        &account.smtp_user
    };
    let pass = if account.smtp_pass.is_empty() {
        &account.incoming_pass
    } else {
        &account.smtp_pass
    };

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
    let recipients: Vec<&str> = to_addrs
        .iter()
        .chain(cc_addrs.iter())
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

    if account.is_imap() {
        if let Err(err) = remote_imap_append(&account, "Sent", &message_data) {
            set_status(&format!("Sent locally, IMAP append pending: {}", err));
            return;
        }
    }

    set_status(anyos_std::i18n::t("Message sent successfully"));
    anyui::MessageBox::show(
        anyui::MessageBoxType::Info,
        anyos_std::i18n::t("Message sent."),
        Some("OK"),
    );
}

fn save_draft(
    to_field: &anyui::AutoCompleteTextField,
    cc_field: &anyui::AutoCompleteTextField,
    subj_field: &anyui::TextField,
    body_editor: &anyui::TextEditor,
    acct_idx: usize,
) {
    let a = app();
    if acct_idx >= a.config.accounts.len() {
        return;
    }
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
        &from,
        &to_addrs,
        &cc_addrs,
        &subj_str,
        &body_str,
        "",
        &[],
        "",
        "",
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

    if account.is_imap() {
        let _ = remote_imap_append(&account, "Drafts", &message_data);
    }

    set_status(anyos_std::i18n::t("Draft saved"));
}

// ═══════════════════════════════════════════════════════════════════════════
// Account Setup Dialog
// ═══════════════════════════════════════════════════════════════════════════

fn open_account_setup(first_run: bool) {
    let t = anyos_std::i18n::t;
    let title = if first_run {
        "Add Mail Account"
    } else {
        t("Account Settings")
    };
    let setup_win = anyui::Window::new(title, -1, -1, 620, 560);
    let colors = tc();
    setup_win.set_color(colors.window_bg);

    let editing_index = {
        let a = app();
        if !first_run && a.current_account < a.config.accounts.len() {
            Some(a.current_account)
        } else {
            None
        }
    };

    let root = anyui::View::new();
    root.set_dock(anyui::DOCK_FILL);
    root.set_color(colors.window_bg);
    root.set_padding(20, 20, 20, 20);

    let hero = anyui::View::new();
    hero.set_position(0, 0);
    hero.set_size(580, 92);
    hero.set_color(colors.sidebar_bg);
    root.add(&hero);

    let hero_title = anyui::Label::new(if first_run {
        "Welcome to anyMail"
    } else {
        "Refine your mail account"
    });
    hero_title.set_position(18, 16);
    hero_title.set_size(360, 28);
    hero_title.set_font_size(24);
    hero_title.set_text_color(colors.text);
    hero.add(&hero_title);

    let hero_subtitle = anyui::Label::new(if first_run {
        "Choose a provider and let anyMail prepare a clean account setup."
    } else {
        "Adjust provider defaults, credentials and transport settings in one guided flow."
    });
    hero_subtitle.set_position(18, 48);
    hero_subtitle.set_size(540, 24);
    hero_subtitle.set_text_color(colors.text_secondary);
    hero.add(&hero_subtitle);

    let page_provider = anyui::View::new();
    page_provider.set_position(0, 108);
    page_provider.set_size(580, 340);
    page_provider.set_color(colors.window_bg);
    root.add(&page_provider);

    let provider_intro = anyui::Label::new("Mail provider");
    provider_intro.set_position(4, 0);
    provider_intro.set_size(160, 22);
    provider_intro.set_font_size(18);
    provider_intro.set_text_color(colors.text);
    page_provider.add(&provider_intro);

    let provider_help = anyui::Label::new(
        "Pick the provider you want to connect. You can still fine-tune server settings before saving.",
    );
    provider_help.set_position(4, 28);
    provider_help.set_size(560, 22);
    provider_help.set_text_color(colors.text_secondary);
    page_provider.add(&provider_help);

    let provider_list = anyui::ListBox::new(provider_items());
    provider_list.set_position(4, 66);
    provider_list.set_size(240, 220);
    page_provider.add(&provider_list);

    let provider_card_title = anyui::Label::new("");
    provider_card_title.set_position(272, 72);
    provider_card_title.set_size(280, 24);
    provider_card_title.set_font_size(20);
    provider_card_title.set_text_color(colors.text);
    page_provider.add(&provider_card_title);

    let provider_card_subtitle = anyui::Label::new("");
    provider_card_subtitle.set_position(272, 104);
    provider_card_subtitle.set_size(280, 54);
    provider_card_subtitle.set_text_color(colors.text_secondary);
    page_provider.add(&provider_card_subtitle);

    let provider_card_note = anyui::Label::new("");
    provider_card_note.set_position(272, 176);
    provider_card_note.set_size(280, 88);
    provider_card_note.set_text_color(colors.text);
    page_provider.add(&provider_card_note);

    let btn_import = anyui::Button::new(t("Import"));
    btn_import.set_position(4, 300);
    btn_import.set_size(110, 30);
    page_provider.add(&btn_import);

    let btn_export = anyui::Button::new(t("Export"));
    btn_export.set_position(124, 300);
    btn_export.set_size(110, 30);
    page_provider.add(&btn_export);

    let page_login = anyui::View::new();
    page_login.set_position(0, 108);
    page_login.set_size(580, 340);
    page_login.set_color(colors.window_bg);
    page_login.set_visible(false);
    root.add(&page_login);

    let login_title = anyui::Label::new("Sign in");
    login_title.set_position(4, 0);
    login_title.set_size(160, 22);
    login_title.set_font_size(18);
    login_title.set_text_color(colors.text);
    page_login.add(&login_title);

    let login_note = anyui::Label::new("");
    login_note.set_position(4, 30);
    login_note.set_size(560, 42);
    login_note.set_text_color(colors.text_secondary);
    page_login.add(&login_note);

    let mut y = 86i32;
    let label_w = 140u32;
    let field_w = 380u32;
    let row_h = 34i32;

    let lbl_name = anyui::Label::new(&format!("{}:", t("Display Name")));
    lbl_name.set_position(8, y);
    lbl_name.set_size(label_w, 22);
    lbl_name.set_text_color(colors.text_secondary);
    page_login.add(&lbl_name);
    let name_field = anyui::TextField::new();
    name_field.set_position(160, y);
    name_field.set_size(field_w, 24);
    name_field.set_placeholder(t("Your Name"));
    page_login.add(&name_field);
    y += row_h;

    let lbl_email = anyui::Label::new(&format!("{}:", t("Email Address")));
    lbl_email.set_position(8, y);
    lbl_email.set_size(label_w, 22);
    lbl_email.set_text_color(colors.text_secondary);
    page_login.add(&lbl_email);
    let email_field = anyui::TextField::new();
    email_field.set_position(160, y);
    email_field.set_size(field_w, 24);
    email_field.set_placeholder("user@example.com");
    page_login.add(&email_field);
    y += row_h;

    let lbl_user = anyui::Label::new(&format!("{}:", t("Username")));
    lbl_user.set_position(8, y);
    lbl_user.set_size(label_w, 22);
    lbl_user.set_text_color(colors.text_secondary);
    page_login.add(&lbl_user);
    let user_field = anyui::TextField::new();
    user_field.set_position(160, y);
    user_field.set_size(field_w, 24);
    user_field.set_placeholder("user@example.com");
    page_login.add(&user_field);
    y += row_h;

    let lbl_pass = anyui::Label::new(&format!("{}:", t("Password")));
    lbl_pass.set_position(8, y);
    lbl_pass.set_size(label_w, 22);
    lbl_pass.set_text_color(colors.text_secondary);
    page_login.add(&lbl_pass);
    let pass_field = anyui::TextField::new();
    pass_field.set_position(160, y);
    pass_field.set_size(field_w, 24);
    pass_field.set_password_mode(true);
    page_login.add(&pass_field);
    y += row_h + 12;

    let autofill_note = anyui::Label::new(
        "anyMail will prefill incoming and outgoing servers from the chosen provider on the next step.",
    );
    autofill_note.set_position(8, y);
    autofill_note.set_size(548, 44);
    autofill_note.set_text_color(colors.text_secondary);
    page_login.add(&autofill_note);

    let page_review = anyui::View::new();
    page_review.set_position(0, 108);
    page_review.set_size(580, 340);
    page_review.set_color(colors.window_bg);
    page_review.set_visible(false);
    root.add(&page_review);

    let review_title = anyui::Label::new("Review server settings");
    review_title.set_position(4, 0);
    review_title.set_size(240, 22);
    review_title.set_font_size(18);
    review_title.set_text_color(colors.text);
    page_review.add(&review_title);

    let review_note = anyui::Label::new(
        "Provider defaults are ready below. Fine-tune anything you need before saving the account.",
    );
    review_note.set_position(4, 30);
    review_note.set_size(560, 40);
    review_note.set_text_color(colors.text_secondary);
    page_review.add(&review_note);

    y = 82;
    let proto_label = anyui::Label::new(&format!("{}:", t("Protocol")));
    proto_label.set_position(8, y);
    proto_label.set_size(label_w, 22);
    proto_label.set_text_color(colors.text_secondary);
    page_review.add(&proto_label);
    let proto_combo = anyui::ComboBox::new();
    proto_combo.set_position(160, y);
    proto_combo.set_size(220, 26);
    proto_combo.set_items("IMAP|POP3");
    proto_combo.set_selected_index(Some(0));
    page_review.add(&proto_combo);
    y += row_h;

    let incoming_label = anyui::Label::new(&format!("{}:", t("Incoming Server")));
    incoming_label.set_position(8, y);
    incoming_label.set_size(label_w, 22);
    incoming_label.set_text_color(colors.text_secondary);
    page_review.add(&incoming_label);
    let in_host = anyui::TextField::new();
    in_host.set_position(160, y);
    in_host.set_size(296, 24);
    in_host.set_placeholder("imap.example.com");
    page_review.add(&in_host);
    let in_port = anyui::TextField::new();
    in_port.set_position(466, y);
    in_port.set_size(74, 24);
    in_port.set_placeholder("993");
    page_review.add(&in_port);
    y += row_h;

    let incoming_sec_label = anyui::Label::new(&format!("{}:", t("Security")));
    incoming_sec_label.set_position(8, y);
    incoming_sec_label.set_size(label_w, 22);
    incoming_sec_label.set_text_color(colors.text_secondary);
    page_review.add(&incoming_sec_label);
    let in_sec = anyui::ComboBox::new();
    in_sec.set_position(160, y);
    in_sec.set_size(220, 26);
    in_sec.set_items("TLS|STARTTLS|None");
    in_sec.set_selected_index(Some(0));
    page_review.add(&in_sec);
    y += row_h + 10;

    let smtp_header = anyui::Label::new("Outgoing (SMTP)");
    smtp_header.set_position(8, y);
    smtp_header.set_size(220, 22);
    smtp_header.set_font_size(18);
    smtp_header.set_text_color(colors.text);
    page_review.add(&smtp_header);
    y += 32;

    let smtp_label = anyui::Label::new(&format!("{}:", t("Outgoing Server")));
    smtp_label.set_position(8, y);
    smtp_label.set_size(label_w, 22);
    smtp_label.set_text_color(colors.text_secondary);
    page_review.add(&smtp_label);
    let smtp_host = anyui::TextField::new();
    smtp_host.set_position(160, y);
    smtp_host.set_size(296, 24);
    smtp_host.set_placeholder("smtp.example.com");
    page_review.add(&smtp_host);
    let smtp_port = anyui::TextField::new();
    smtp_port.set_position(466, y);
    smtp_port.set_size(74, 24);
    smtp_port.set_placeholder("587");
    page_review.add(&smtp_port);
    y += row_h;

    let smtp_sec_label = anyui::Label::new(&format!("{}:", t("Security")));
    smtp_sec_label.set_position(8, y);
    smtp_sec_label.set_size(label_w, 22);
    smtp_sec_label.set_text_color(colors.text_secondary);
    page_review.add(&smtp_sec_label);
    let smtp_sec = anyui::ComboBox::new();
    smtp_sec.set_position(160, y);
    smtp_sec.set_size(220, 26);
    smtp_sec.set_items("TLS|STARTTLS|None");
    smtp_sec.set_selected_index(Some(1));
    page_review.add(&smtp_sec);

    let footer = anyui::View::new();
    footer.set_position(0, 462);
    footer.set_size(580, 56);
    footer.set_color(colors.window_bg);
    root.add(&footer);

    let btn_back = anyui::Button::new("Back");
    btn_back.set_position(230, 10);
    btn_back.set_size(100, 32);
    btn_back.set_visible(false);
    footer.add(&btn_back);

    let btn_test = anyui::Button::new(t("Test Connection"));
    btn_test.set_position(340, 10);
    btn_test.set_size(120, 32);
    btn_test.set_visible(false);
    footer.add(&btn_test);

    let btn_next = anyui::Button::new("Continue");
    btn_next.set_position(470, 10);
    btn_next.set_size(110, 32);
    footer.add(&btn_next);

    setup_win.add(&root);

    let initial_provider = if let Some(idx) = editing_index {
        let a = app();
        detect_provider(&a.config.accounts[idx].email)
    } else {
        AccountProvider::Gmail
    };
    provider_list.set_selected_index(provider_to_index(initial_provider));
    update_provider_copy(
        initial_provider,
        &provider_card_title,
        &provider_card_subtitle,
        &provider_card_note,
    );
    login_note.set_text(provider_heading(initial_provider).2);

    if let Some(idx) = editing_index {
        let a = app();
        let acc = &a.config.accounts[idx];
        name_field.set_text(&acc.display_name);
        email_field.set_text(&acc.email);
        user_field.set_text(&acc.incoming_user);
        pass_field.set_text(&acc.incoming_pass);
        in_host.set_text(&acc.incoming_host);
        in_port.set_text(&format!("{}", acc.incoming_port));
        in_sec.set_selected_index(Some(security_to_index(acc.incoming_security)));
        proto_combo.set_selected_index(Some(protocol_to_index(acc.incoming_protocol)));
        smtp_host.set_text(&acc.smtp_host);
        smtp_port.set_text(&format!("{}", acc.smtp_port));
        smtp_sec.set_selected_index(Some(security_to_index(acc.smtp_security)));
    }

    let step = Rc::new(Cell::new(0u32));

    provider_list.on_selection_changed({
        let provider_card_title = provider_card_title.clone();
        let provider_card_subtitle = provider_card_subtitle.clone();
        let provider_card_note = provider_card_note.clone();
        let login_note = login_note.clone();
        move |_| {
            let provider = provider_from_selector(&provider_list);
            update_provider_copy(
                provider,
                &provider_card_title,
                &provider_card_subtitle,
                &provider_card_note,
            );
            login_note.set_text(provider_heading(provider).2);
        }
    });

    email_field.on_text_changed({
        let user_field = user_field.clone();
        move |_| {
            let email = get_widget_text(&email_field);
            if !email.is_empty() && get_widget_text(&user_field).is_empty() {
                user_field.set_text(&email);
            }
        }
    });

    btn_import.on_click(|_| {
        import_accounts();
    });

    btn_export.on_click(|_| {
        export_accounts();
    });

    btn_back.on_click({
        let page_provider = page_provider.clone();
        let page_login = page_login.clone();
        let page_review = page_review.clone();
        let btn_back = btn_back.clone();
        let btn_next = btn_next.clone();
        let btn_test = btn_test.clone();
        let step = step.clone();
        move |_| {
            let current = step.get();
            if current > 0 {
                let next = current - 1;
                step.set(next);
                update_wizard_step(next, &page_provider, &page_login, &page_review, &btn_back, &btn_next, &btn_test);
            }
        }
    });

    btn_next.on_click({
        let page_provider = page_provider.clone();
        let page_login = page_login.clone();
        let page_review = page_review.clone();
        let btn_back = btn_back.clone();
        let btn_next = btn_next.clone();
        let btn_test = btn_test.clone();
        let step = step.clone();
        let provider_list = provider_list.clone();
        let email_field = email_field.clone();
        let user_field = user_field.clone();
        let proto_combo = proto_combo.clone();
        let in_host = in_host.clone();
        let in_port = in_port.clone();
        let in_sec = in_sec.clone();
        let smtp_host = smtp_host.clone();
        let smtp_port = smtp_port.clone();
        let smtp_sec = smtp_sec.clone();
        let name_field = name_field.clone();
        let pass_field = pass_field.clone();
        move |_| {
            match step.get() {
                0 => {
                    step.set(1);
                    update_wizard_step(1, &page_provider, &page_login, &page_review, &btn_back, &btn_next, &btn_test);
                    email_field.focus();
                }
                1 => {
                    let email = get_widget_text(&email_field);
                    if email.is_empty() {
                        anyui::MessageBox::show(
                            anyui::MessageBoxType::Warning,
                            "Please enter an email address before continuing.",
                            Some("OK"),
                        );
                        return;
                    }
                    let provider = provider_from_selector(&provider_list);
                    let force_defaults = editing_index.is_none();
                    apply_provider_defaults_to_form(
                        provider,
                        &email,
                        &proto_combo,
                        &in_host,
                        &in_port,
                        &in_sec,
                        &smtp_host,
                        &smtp_port,
                        &smtp_sec,
                        force_defaults,
                    );
                    if get_widget_text(&name_field).is_empty() {
                        let local_part = email.split('@').next().unwrap_or("");
                        if !local_part.is_empty() {
                            name_field.set_text(local_part);
                        }
                    }
                    if get_widget_text(&pass_field).is_empty() {
                        set_status("Enter your password, then test and save the account.");
                    }
                    step.set(2);
                    update_wizard_step(2, &page_provider, &page_login, &page_review, &btn_back, &btn_next, &btn_test);
                }
                _ => {
                    let acc = build_account_from_form(
                        &name_field,
                        &email_field,
                        &user_field,
                        &pass_field,
                        &proto_combo,
                        &in_host,
                        &in_port,
                        &in_sec,
                        &smtp_host,
                        &smtp_port,
                        &smtp_sec,
                    );
                    if acc.email.is_empty() {
                        anyui::MessageBox::show(
                            anyui::MessageBoxType::Warning,
                            "Please complete the email address before saving.",
                            Some("OK"),
                        );
                        return;
                    }
                    let a = app();
                    let target_idx = editing_index.or_else(|| {
                        a.config.accounts.iter().position(|existing| existing.id == acc.id)
                    });
                    if let Some(idx) = target_idx {
                        a.config.accounts[idx] = acc.clone();
                        a.current_account = idx;
                    } else {
                        a.config.accounts.push(acc.clone());
                        a.current_account = a.config.accounts.len().saturating_sub(1);
                    }
                    a.current_folder = String::from("INBOX");
                    a.config.active_account = a.current_account;
                    a.config.save();
                    maildir::ensure_dirs(&a.base_dir, &acc.id);
                    populate_folder_tree();
                    load_folder_messages();
                    set_status(anyos_std::i18n::t("Account saved"));
                    anyui::MessageBox::show(
                        anyui::MessageBoxType::Info,
                        anyos_std::i18n::t("Account saved successfully."),
                        Some("OK"),
                    );
                }
            }
        }
    });

    btn_test.on_click({
        let name_field = name_field.clone();
        let email_field = email_field.clone();
        let user_field = user_field.clone();
        let pass_field = pass_field.clone();
        let proto_combo = proto_combo.clone();
        let in_host = in_host.clone();
        let in_port = in_port.clone();
        let in_sec = in_sec.clone();
        let smtp_host = smtp_host.clone();
        let smtp_port = smtp_port.clone();
        let smtp_sec = smtp_sec.clone();
        move |_| {
            let acc = build_account_from_form(
                &name_field,
                &email_field,
                &user_field,
                &pass_field,
                &proto_combo,
                &in_host,
                &in_port,
                &in_sec,
                &smtp_host,
                &smtp_port,
                &smtp_sec,
            );
            set_status(&format!(
                "Testing {} and {} ...",
                acc.incoming_host,
                if acc.smtp_host.is_empty() {
                    "outgoing server"
                } else {
                    &acc.smtp_host
                }
            ));
            match test_account_settings(&acc) {
                Ok(()) => {
                    set_status(anyos_std::i18n::t("Connection test successful!"));
                    anyui::MessageBox::show(
                        anyui::MessageBoxType::Info,
                        "Incoming and outgoing login look good.",
                        Some("OK"),
                    );
                }
                Err(err) => {
                    set_status(&format!("Connection failed: {}", err));
                    anyui::MessageBox::show(anyui::MessageBoxType::Alert, &err, Some("OK"));
                }
            }
        }
    });

    update_wizard_step(0, &page_provider, &page_login, &page_review, &btn_back, &btn_next, &btn_test);
    setup_win.on_close(|_| {});
}

// ═══════════════════════════════════════════════════════════════════════════
// Account Export / Import
// ═══════════════════════════════════════════════════════════════════════════

fn export_accounts() {
    if let Some(path) = anyui::FileDialog::save_file("anymail-accounts.json") {
        let a = app();
        a.config.save_to_path(&path);
        set_status(&format!(
            "{}: {}",
            anyos_std::i18n::t("Accounts exported to"),
            path
        ));
        anyui::MessageBox::show(
            anyui::MessageBoxType::Info,
            anyos_std::i18n::t("Accounts exported successfully."),
            Some("OK"),
        );
    }
}

fn import_accounts() {
    if let Some(path) = anyui::FileDialog::open_file() {
        let imported = MailConfig::load_from_path(&path);
        if imported.accounts.is_empty() {
            anyui::MessageBox::show(
                anyui::MessageBoxType::Warning,
                &format!(
                    "{}\n{}: {}",
                    anyos_std::i18n::t("No accounts found in the selected file."),
                    anyos_std::i18n::t("Path"),
                    path
                ),
                Some("OK"),
            );
            return;
        }

        // Show import dialog with Merge / Replace options
        let t = anyos_std::i18n::t;
        let import_win = anyui::Window::new(t("Import Accounts"), -1, -1, 400, 180);
        let colors = tc();
        import_win.set_color(colors.window_bg);

        let panel = anyui::View::new();
        panel.set_dock(anyui::DOCK_FILL);
        panel.set_color(colors.window_bg);
        panel.set_padding(16, 16, 16, 16);

        let info = anyui::Label::new(&format!(
            "{}: {} {}",
            t("Found"),
            imported.accounts.len(),
            t("accounts")
        ));
        info.set_position(16, 16);
        info.set_size(360, 24);
        info.set_text_color(colors.text);
        panel.add(&info);

        let question = anyui::Label::new(t("Merge with existing or replace all?"));
        question.set_position(16, 48);
        question.set_size(360, 24);
        question.set_text_color(colors.text_secondary);
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
                if !a
                    .config
                    .accounts
                    .iter()
                    .any(|existing| existing.id == acc.id)
                {
                    a.config.accounts.push(acc.clone());
                    maildir::ensure_dirs(&a.base_dir, &acc.id);
                }
            }
            a.config.save();
            populate_folder_tree();
            set_status(anyos_std::i18n::t("Accounts imported (merged)"));
            anyui::MessageBox::show(
                anyui::MessageBoxType::Info,
                &format!(
                    "{}\n{}",
                    anyos_std::i18n::t("Accounts merged successfully."),
                    anyos_std::i18n::t("Edit account settings to add missing passwords.")
                ),
                Some("OK"),
            );
        });

        btn_replace.on_click(move |_| {
            let a = app();
            a.config.accounts = imported_accounts2.clone();
            a.config.check_on_startup = imported_check;
            for acc in &a.config.accounts {
                maildir::ensure_dirs(&a.base_dir, &acc.id);
            }
            a.config.save();
            populate_folder_tree();
            set_status(anyos_std::i18n::t("Accounts imported (replaced)"));
            anyui::MessageBox::show(
                anyui::MessageBoxType::Info,
                &format!(
                    "{}\n{}",
                    anyos_std::i18n::t("All accounts replaced successfully."),
                    anyos_std::i18n::t("Edit account settings to add missing passwords.")
                ),
                Some("OK"),
            );
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
    let colors = tc();
    contacts_win.set_color(colors.window_bg);

    let toolbar = anyui::Toolbar::new();
    toolbar.set_dock(anyui::DOCK_TOP);
    toolbar.set_size(600, 46);
    toolbar.set_color(colors.toolbar_bg);
    toolbar.set_padding(4, 4, 4, 4);

    let btn_add = toolbar.add_icon_button("");
    btn_add.set_size(34, 34);
    btn_add.set_system_icon("user-plus", IconType::Outline, colors.text, 24);
    btn_add.set_tooltip(t("Add Contact"));

    let btn_del = toolbar.add_icon_button("");
    btn_del.set_size(34, 34);
    btn_del.set_system_icon("user-minus", IconType::Outline, colors.text, 24);
    btn_del.set_tooltip(t("Delete"));

    contacts_win.add(&toolbar);

    let grid = anyui::DataGrid::new(600, 350);
    grid.set_dock(anyui::DOCK_FILL);
    grid.set_columns(&[
        anyui::ColumnDef::new(t("Name")).width(200),
        anyui::ColumnDef::new(t("Email")).width(250),
        anyui::ColumnDef::new(t("Group")).width(100),
    ]);
    grid.set_color(colors.editor_bg);
    grid.set_text_color(colors.text);
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
        let colors = tc();
        add_win.set_color(colors.window_bg);
        let panel = anyui::View::new();
        panel.set_dock(anyui::DOCK_FILL);
        panel.set_color(colors.window_bg);
        panel.set_padding(16, 16, 16, 16);

        let t = anyos_std::i18n::t;
        let name_lbl = anyui::Label::new(&format!("{}:", t("Name")));
        name_lbl.set_position(8, 8);
        name_lbl.set_size(60, 20);
        name_lbl.set_text_color(colors.text_secondary);
        panel.add(&name_lbl);
        let name_f = anyui::TextField::new();
        name_f.set_position(70, 8);
        name_f.set_size(260, 22);
        panel.add(&name_f);

        let email_lbl = anyui::Label::new(&format!("{}:", t("Email")));
        email_lbl.set_position(8, 38);
        email_lbl.set_size(60, 20);
        email_lbl.set_text_color(colors.text_secondary);
        panel.add(&email_lbl);
        let email_f = anyui::TextField::new();
        email_f.set_position(70, 38);
        email_f.set_size(260, 22);
        panel.add(&email_f);

        let save_btn = anyui::Button::new(t("Save"));
        save_btn.set_position(70, 80);
        save_btn.set_size(100, 28);
        panel.add(&save_btn);

        add_win.add(&panel);

        save_btn.on_click(move |_| {
            let name = get_field_text(&name_f);
            let email = get_field_text(&email_f);
            if !email.is_empty() {
                let contact = Contact::new(&name, &email);
                let a = app();
                a.address_book.add(contact);
                a.address_book.save();

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
            a.address_book.save();

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
        if i > 0 {
            s.push_str(", ");
        }
        s.push_str(a.display_short());
    }
    if s.is_empty() {
        s.push_str(anyos_std::i18n::t("(none)"));
    }
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
