// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! Account configuration backed by confd with JSON import/export compatibility.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::json::Value;
use libconf_schema::{ConfClient, RegistryScope};

use crate::storage::schema::schema;

/// Protocol type for incoming mail.
#[derive(Clone, Copy, PartialEq)]
pub enum IncomingProtocol {
    Imap,
    Pop3,
}

/// Security/encryption mode.
#[derive(Clone, Copy, PartialEq)]
pub enum Security {
    None,
    Tls,
    StartTls,
}

/// An email account configuration.
#[derive(Clone)]
pub struct Account {
    pub id: String,
    pub display_name: String,
    pub email: String,
    // Incoming server
    pub incoming_protocol: IncomingProtocol,
    pub incoming_host: String,
    pub incoming_port: u16,
    pub incoming_security: Security,
    pub incoming_user: String,
    pub incoming_pass: String,
    // Outgoing server (SMTP)
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_security: Security,
    pub smtp_user: String,
    pub smtp_pass: String,
    // Settings
    pub check_interval_secs: u32,
    pub signature: String,
    pub leave_on_server: bool, // POP3: don't delete after download
}

impl Account {
    pub fn new() -> Self {
        Self {
            id: String::new(),
            display_name: String::new(),
            email: String::new(),
            incoming_protocol: IncomingProtocol::Imap,
            incoming_host: String::new(),
            incoming_port: 993,
            incoming_security: Security::Tls,
            incoming_user: String::new(),
            incoming_pass: String::new(),
            smtp_host: String::new(),
            smtp_port: 587,
            smtp_security: Security::StartTls,
            smtp_user: String::new(),
            smtp_pass: String::new(),
            check_interval_secs: 300,
            signature: String::new(),
            leave_on_server: true,
        }
    }

    /// Generate an account ID from the email address.
    pub fn generate_id(email: &str) -> String {
        // Simple hash from email
        let hash = anyos_std::crypto::md5_hex(email.as_bytes());
        let hex = core::str::from_utf8(&hash).unwrap_or("default");
        String::from(&hex[..8.min(hex.len())])
    }

    pub fn is_imap(&self) -> bool {
        self.incoming_protocol == IncomingProtocol::Imap
    }

    pub fn is_pop3(&self) -> bool {
        self.incoming_protocol == IncomingProtocol::Pop3
    }

    pub fn incoming_use_tls(&self) -> bool {
        self.incoming_security == Security::Tls
    }

    pub fn smtp_use_tls(&self) -> bool {
        self.smtp_security == Security::Tls
    }

    pub fn smtp_use_starttls(&self) -> bool {
        self.smtp_security == Security::StartTls
    }
}

/// Global mail configuration.
pub struct MailConfig {
    pub accounts: Vec<Account>,
    pub active_account: usize,
    pub check_on_startup: bool,
    pub theme: String,
}

impl MailConfig {
    pub fn new() -> Self {
        Self {
            accounts: Vec::new(),
            active_account: 0,
            check_on_startup: true,
            theme: String::from("dark"),
        }
    }

    /// Load config from confd, falling back to a legacy JSON file on first run.
    pub fn load(legacy_path: &str) -> Self {
        let _ = schema().register();
        if let Some(config) = load_structured_from_confd() {
            return config;
        }
        if let Some(config) = load_legacy_blob_from_confd() {
            config.save();
            delete_legacy_accounts_blob();
            return config;
        }
        let config = Self::load_from_path(legacy_path);
        if !config.accounts.is_empty() || config.check_on_startup != Self::new().check_on_startup {
            config.save();
        }
        config
    }

    /// Load accounts from a JSON file for migration or import.
    pub fn load_from_path(path: &str) -> Self {
        let fd = anyos_std::fs::open(path, 0);
        if fd == u32::MAX {
            return Self::new();
        }

        let mut buf = alloc::vec![0u8; 64 * 1024];
        let mut total = 0usize;
        loop {
            let mut chunk = [0u8; 4096];
            let n = anyos_std::fs::read(fd, &mut chunk);
            if n == 0 || n == u32::MAX {
                break;
            }
            let n = n as usize;
            if total + n > buf.len() {
                break;
            }
            buf[total..total + n].copy_from_slice(&chunk[..n]);
            total += n;
        }
        anyos_std::fs::close(fd);

        let trimmed = core::str::from_utf8(&buf[..total]).unwrap_or("").trim();
        Self::from_json_str(trimmed)
    }

    /// Save the authoritative config to confd.
    pub fn save(&self) {
        let _ = schema().register();
        let _ = schema().write_bool("config/check_on_startup", self.check_on_startup);
        let _ = schema().write_string("config/theme", &self.theme);
        let _ = schema().write_i64("config/active_account", self.active_account as i64);
        let _ = schema().write_i64("config/accounts_schema_version", 1);
        let _ = schema().write_i64("config/accounts_count", self.accounts.len() as i64);
        save_accounts_to_confd(&self.accounts);
        delete_legacy_accounts_blob();
    }

    /// Save JSON to a file for export compatibility.
    pub fn save_to_path(&self, path: &str) {
        let json_str = self.to_json_string();
        let _ = anyos_std::fs::write_bytes(path, json_str.as_bytes());
    }

    fn from_json_str(text: &str) -> Self {
        let mut config = Self::new();
        if text.is_empty() {
            return config;
        }
        let json = match Value::parse(text) {
            Ok(v) => v,
            Err(_) => return config,
        };

        config.active_account = json["active_account"].as_i64().unwrap_or(0).max(0) as usize;
        config.check_on_startup = json["check_on_startup"].as_bool().unwrap_or(true);
        config.theme = String::from(json["theme"].as_str().unwrap_or("dark"));

        if let Some(arr) = json["accounts"].as_array() {
            for item in arr {
                let mut acc = Account::new();
                acc.id = String::from(item["id"].as_str().unwrap_or(""));
                acc.display_name = String::from(item["display_name"].as_str().unwrap_or(""));
                acc.email = String::from(item["email"].as_str().unwrap_or(""));

                let proto = item["incoming_protocol"].as_str().unwrap_or("imap");
                acc.incoming_protocol = if proto == "pop3" {
                    IncomingProtocol::Pop3
                } else {
                    IncomingProtocol::Imap
                };
                acc.incoming_host = String::from(item["incoming_host"].as_str().unwrap_or(""));
                acc.incoming_port = item["incoming_port"].as_i64().unwrap_or(993) as u16;
                acc.incoming_security =
                    parse_security(item["incoming_security"].as_str().unwrap_or("tls"));
                acc.incoming_user = String::from(item["incoming_user"].as_str().unwrap_or(""));
                let pass_str = item["incoming_pass"].as_str().unwrap_or("");
                acc.incoming_pass = if pass_str.starts_with("$OBF$") {
                    deobfuscate(&pass_str[5..])
                } else {
                    String::from(pass_str)
                };

                acc.smtp_host = String::from(item["smtp_host"].as_str().unwrap_or(""));
                acc.smtp_port = item["smtp_port"].as_i64().unwrap_or(587) as u16;
                acc.smtp_security =
                    parse_security(item["smtp_security"].as_str().unwrap_or("starttls"));
                acc.smtp_user = String::from(item["smtp_user"].as_str().unwrap_or(""));
                let pass_str = item["smtp_pass"].as_str().unwrap_or("");
                acc.smtp_pass = if pass_str.starts_with("$OBF$") {
                    deobfuscate(&pass_str[5..])
                } else {
                    String::from(pass_str)
                };

                acc.check_interval_secs = item["check_interval"].as_i64().unwrap_or(300) as u32;
                acc.signature = String::from(item["signature"].as_str().unwrap_or(""));
                acc.leave_on_server = item["leave_on_server"].as_bool().unwrap_or(true);

                if acc.id.is_empty() {
                    acc.id = Account::generate_id(&acc.email);
                }

                if !acc.email.is_empty() {
                    config.accounts.push(acc);
                }
            }
        }

        config
    }

    fn to_json_string(&self) -> String {
        let mut root = Value::new_object();
        root.set("active_account", (self.active_account as i64).into());
        root.set("check_on_startup", self.check_on_startup.into());
        root.set("theme", self.theme.as_str().into());

        let mut arr = Value::new_array();
        for acc in &self.accounts {
            let mut obj = Value::new_object();
            obj.set("id", acc.id.as_str().into());
            obj.set("display_name", acc.display_name.as_str().into());
            obj.set("email", acc.email.as_str().into());
            obj.set(
                "incoming_protocol",
                (if acc.is_imap() { "imap" } else { "pop3" }).into(),
            );
            obj.set("incoming_host", acc.incoming_host.as_str().into());
            obj.set("incoming_port", (acc.incoming_port as i64).into());
            obj.set(
                "incoming_security",
                security_str(acc.incoming_security).into(),
            );
            obj.set("incoming_user", acc.incoming_user.as_str().into());
            // Store passwords in plain text for better compatibility and simpler import/export
            // (They're already protected by file permissions, not cryptographically)
            obj.set("incoming_pass", acc.incoming_pass.as_str().into());
            obj.set("smtp_host", acc.smtp_host.as_str().into());
            obj.set("smtp_port", (acc.smtp_port as i64).into());
            obj.set("smtp_security", security_str(acc.smtp_security).into());
            obj.set("smtp_user", acc.smtp_user.as_str().into());
            obj.set("smtp_pass", acc.smtp_pass.as_str().into());
            obj.set("check_interval", (acc.check_interval_secs as i64).into());
            obj.set("signature", acc.signature.as_str().into());
            obj.set("leave_on_server", acc.leave_on_server.into());
            arr.push(obj);
        }
        root.set("accounts", arr);

        root.to_json_string_pretty()
    }
}

fn load_structured_from_confd() -> Option<MailConfig> {
    if schema().read_i64("config/accounts_schema_version").unwrap_or(0) < 1 {
        return None;
    }

    let mut config = MailConfig::new();
    config.check_on_startup = schema()
        .read_bool("config/check_on_startup")
        .unwrap_or(config.check_on_startup);
    config.theme = schema()
        .read_string("config/theme")
        .unwrap_or(config.theme);

    let count = schema().read_i64("config/accounts_count").unwrap_or(0).max(0) as usize;
    for index in 0..count {
        if let Some(account) = load_structured_account(index) {
            config.accounts.push(account);
        }
    }

    config.active_account = schema()
        .read_i64("config/active_account")
        .unwrap_or(0)
        .max(0) as usize;
    clamp_active_account(&mut config);
    Some(config)
}

fn load_legacy_blob_from_confd() -> Option<MailConfig> {
    let json = schema().read_string("config/accounts_json")?;
    if json.trim().is_empty() {
        return None;
    }
    let mut config = MailConfig::from_json_str(&json);
    config.check_on_startup = schema()
        .read_bool("config/check_on_startup")
        .unwrap_or(config.check_on_startup);
    config.theme = schema()
        .read_string("config/theme")
        .unwrap_or(config.theme);
    config.active_account = schema()
        .read_i64("config/active_account")
        .unwrap_or(config.active_account as i64)
        .max(0) as usize;
    clamp_active_account(&mut config);
    Some(config)
}

fn load_structured_account(index: usize) -> Option<Account> {
    let mut acc = Account::new();
    let prefix = account_prefix(index);

    acc.id = schema().read_string(&join_path(&prefix, "id")).unwrap_or_default();
    acc.display_name = schema()
        .read_string(&join_path(&prefix, "display_name"))
        .unwrap_or_default();
    acc.email = schema().read_string(&join_path(&prefix, "email"))?;

    let proto = schema()
        .read_string(&join_path(&prefix, "incoming_protocol"))
        .unwrap_or_else(|| String::from("imap"));
    acc.incoming_protocol = if proto == "pop3" {
        IncomingProtocol::Pop3
    } else {
        IncomingProtocol::Imap
    };
    acc.incoming_host = schema()
        .read_string(&join_path(&prefix, "incoming_host"))
        .unwrap_or_default();
    acc.incoming_port = schema()
        .read_i64(&join_path(&prefix, "incoming_port"))
        .unwrap_or(993) as u16;
    acc.incoming_security = parse_security(
        &schema()
            .read_string(&join_path(&prefix, "incoming_security"))
            .unwrap_or_else(|| String::from("tls")),
    );
    acc.incoming_user = schema()
        .read_string(&join_path(&prefix, "incoming_user"))
        .unwrap_or_default();
    acc.incoming_pass = schema()
        .read_string(&join_path(&prefix, "incoming_pass"))
        .unwrap_or_default();
    acc.smtp_host = schema()
        .read_string(&join_path(&prefix, "smtp_host"))
        .unwrap_or_default();
    acc.smtp_port = schema()
        .read_i64(&join_path(&prefix, "smtp_port"))
        .unwrap_or(587) as u16;
    acc.smtp_security = parse_security(
        &schema()
            .read_string(&join_path(&prefix, "smtp_security"))
            .unwrap_or_else(|| String::from("starttls")),
    );
    acc.smtp_user = schema()
        .read_string(&join_path(&prefix, "smtp_user"))
        .unwrap_or_default();
    acc.smtp_pass = schema()
        .read_string(&join_path(&prefix, "smtp_pass"))
        .unwrap_or_default();
    acc.check_interval_secs = schema()
        .read_i64(&join_path(&prefix, "check_interval_secs"))
        .unwrap_or(300)
        .max(0) as u32;
    acc.signature = schema()
        .read_string(&join_path(&prefix, "signature"))
        .unwrap_or_default();
    acc.leave_on_server = schema()
        .read_bool(&join_path(&prefix, "leave_on_server"))
        .unwrap_or(true);

    if acc.id.is_empty() {
        acc.id = Account::generate_id(&acc.email);
    }
    Some(acc)
}

fn save_accounts_to_confd(accounts: &[Account]) {
    let mut client = match ConfClient::connect("anymail") {
        Ok(client) => client,
        Err(_) => return,
    };

    let scope = RegistryScope::User;
    let root = schema().full_path("config/accounts");
    let _ = client.mkdir(scope, &root);

    for (index, account) in accounts.iter().enumerate() {
        let prefix = schema().full_path(&account_prefix(index));
        let _ = client.mkdir(scope, &prefix);
        write_account_field(index, "id", &account.id);
        write_account_field(index, "display_name", &account.display_name);
        write_account_field(index, "email", &account.email);
        write_account_field(
            index,
            "incoming_protocol",
            if account.is_imap() { "imap" } else { "pop3" },
        );
        write_account_field(index, "incoming_host", &account.incoming_host);
        let _ = schema().write_i64(
            &join_path(&account_prefix(index), "incoming_port"),
            account.incoming_port as i64,
        );
        write_account_field(
            index,
            "incoming_security",
            security_str(account.incoming_security),
        );
        write_account_field(index, "incoming_user", &account.incoming_user);
        write_account_field(index, "incoming_pass", &account.incoming_pass);
        write_account_field(index, "smtp_host", &account.smtp_host);
        let _ = schema().write_i64(
            &join_path(&account_prefix(index), "smtp_port"),
            account.smtp_port as i64,
        );
        write_account_field(index, "smtp_security", security_str(account.smtp_security));
        write_account_field(index, "smtp_user", &account.smtp_user);
        write_account_field(index, "smtp_pass", &account.smtp_pass);
        let _ = schema().write_i64(
            &join_path(&account_prefix(index), "check_interval_secs"),
            account.check_interval_secs as i64,
        );
        write_account_field(index, "signature", &account.signature);
        let _ = schema().write_bool(
            &join_path(&account_prefix(index), "leave_on_server"),
            account.leave_on_server,
        );
    }

    if let Ok(children) = client.list_children(scope, &root) {
        for child in children {
            let name = leaf_name(&child.path);
            if let Ok(index) = name.parse::<usize>() {
                if index >= accounts.len() {
                    let _ = client.del(scope, &child.path);
                }
            }
        }
    }
}

fn write_account_field(index: usize, field: &str, value: &str) {
    let _ = schema().write_string(&join_path(&account_prefix(index), field), value);
}

fn account_prefix(index: usize) -> String {
    format!("config/accounts/{}", index)
}

fn join_path(prefix: &str, field: &str) -> String {
    format!("{}/{}", prefix, field)
}

fn leaf_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn clamp_active_account(config: &mut MailConfig) {
    if config.accounts.is_empty() {
        config.active_account = 0;
    } else if config.active_account >= config.accounts.len() {
        config.active_account = config.accounts.len() - 1;
    }
}

fn delete_legacy_accounts_blob() {
    let mut client = match ConfClient::connect("anymail") {
        Ok(client) => client,
        Err(_) => return,
    };
    let _ = client.del(RegistryScope::User, &schema().full_path("config/accounts_json"));
}

fn parse_security(s: &str) -> Security {
    match s {
        "tls" | "ssl" => Security::Tls,
        "starttls" => Security::StartTls,
        _ => Security::None,
    }
}

fn security_str(s: Security) -> &'static str {
    match s {
        Security::Tls => "tls",
        Security::StartTls => "starttls",
        Security::None => "none",
    }
}

fn deobfuscate(encoded: &str) -> String {
    if encoded.is_empty() {
        return String::new();
    }
    let data = crate::mail::base64::decode_str(encoded);
    if data.is_empty() {
        return String::new();
    } // Fallback if base64 decode fails
    let key = anyos_std::crypto::md5(b"anymail-key-seed");
    let mut out = Vec::new();
    for (i, &b) in data.iter().enumerate() {
        out.push(b ^ key[i % 16]);
    }
    String::from(core::str::from_utf8(&out).unwrap_or(""))
}
