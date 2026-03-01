//! Account configuration (JSON persistence in $HOME/.anymail/accounts.json).

use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::json::Value;

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

    /// Load accounts from JSON file.
    pub fn load(path: &str) -> Self {
        let mut config = Self::new();

        let fd = anyos_std::fs::open(path, 0);
        if fd == u32::MAX {
            // File doesn't exist or can't be opened
            return config;
        }

        let mut buf = alloc::vec![0u8; 64 * 1024];  // Increased buffer size
        let mut total = 0usize;
        loop {
            let mut chunk = [0u8; 4096];
            let n = anyos_std::fs::read(fd, &mut chunk);
            if n == 0 || n == u32::MAX { break; }
            let n = n as usize;
            if total + n > buf.len() { break; }
            buf[total..total + n].copy_from_slice(&chunk[..n]);
            total += n;
        }
        anyos_std::fs::close(fd);

        if total == 0 {
            // File is empty
            return config;
        }

        // Trim whitespace and null bytes
        let text_bytes = &buf[..total];
        let trimmed = core::str::from_utf8(text_bytes).unwrap_or("").trim();

        if trimmed.is_empty() {
            return config;
        }

        let json = match Value::parse(trimmed) {
            Ok(v) => v,
            Err(_) => {
                // JSON parse failed - return default config
                return config;
            }
        };

        config.check_on_startup = json["check_on_startup"].as_bool().unwrap_or(true);
        config.theme = String::from(json["theme"].as_str().unwrap_or("dark"));

        if let Some(arr) = json["accounts"].as_array() {
            for item in arr {
                let mut acc = Account::new();
                acc.id = String::from(item["id"].as_str().unwrap_or(""));
                acc.display_name = String::from(item["display_name"].as_str().unwrap_or(""));
                acc.email = String::from(item["email"].as_str().unwrap_or(""));

                let proto = item["incoming_protocol"].as_str().unwrap_or("imap");
                acc.incoming_protocol = if proto == "pop3" { IncomingProtocol::Pop3 } else { IncomingProtocol::Imap };
                acc.incoming_host = String::from(item["incoming_host"].as_str().unwrap_or(""));
                acc.incoming_port = item["incoming_port"].as_i64().unwrap_or(993) as u16;
                acc.incoming_security = parse_security(item["incoming_security"].as_str().unwrap_or("tls"));
                acc.incoming_user = String::from(item["incoming_user"].as_str().unwrap_or(""));
                // Try obfuscated format first, fall back to plain text
                let pass_str = item["incoming_pass"].as_str().unwrap_or("");
                acc.incoming_pass = if pass_str.starts_with("$OBF$") {
                    deobfuscate(&pass_str[5..])  // Skip $OBF$ prefix
                } else {
                    String::from(pass_str)  // Plain text fallback
                };

                acc.smtp_host = String::from(item["smtp_host"].as_str().unwrap_or(""));
                acc.smtp_port = item["smtp_port"].as_i64().unwrap_or(587) as u16;
                acc.smtp_security = parse_security(item["smtp_security"].as_str().unwrap_or("starttls"));
                acc.smtp_user = String::from(item["smtp_user"].as_str().unwrap_or(""));
                // Try obfuscated format first, fall back to plain text
                let pass_str = item["smtp_pass"].as_str().unwrap_or("");
                acc.smtp_pass = if pass_str.starts_with("$OBF$") {
                    deobfuscate(&pass_str[5..])  // Skip $OBF$ prefix
                } else {
                    String::from(pass_str)  // Plain text fallback
                };

                acc.check_interval_secs = item["check_interval"].as_i64().unwrap_or(300) as u32;
                acc.signature = String::from(item["signature"].as_str().unwrap_or(""));
                acc.leave_on_server = item["leave_on_server"].as_bool().unwrap_or(true);

                if acc.id.is_empty() {
                    acc.id = Account::generate_id(&acc.email);
                }

                // Sanity check: only add account if it has email
                if !acc.email.is_empty() {
                    config.accounts.push(acc);
                }
            }
        }

        config
    }

    /// Save accounts to JSON file.
    pub fn save(&self, path: &str) {
        let mut root = Value::new_object();
        root.set("check_on_startup", self.check_on_startup.into());
        root.set("theme", self.theme.as_str().into());

        let mut arr = Value::new_array();
        for acc in &self.accounts {
            let mut obj = Value::new_object();
            obj.set("id", acc.id.as_str().into());
            obj.set("display_name", acc.display_name.as_str().into());
            obj.set("email", acc.email.as_str().into());
            obj.set("incoming_protocol", (if acc.is_imap() { "imap" } else { "pop3" }).into());
            obj.set("incoming_host", acc.incoming_host.as_str().into());
            obj.set("incoming_port", (acc.incoming_port as i64).into());
            obj.set("incoming_security", security_str(acc.incoming_security).into());
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

        let json_str = root.to_json_string_pretty();
        let _ = anyos_std::fs::write_bytes(path, json_str.as_bytes());
    }
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

/// Simple XOR obfuscation with machine-derived key (not cryptographically secure).
fn obfuscate(password: &str) -> String {
    let key = anyos_std::crypto::md5(b"anymail-key-seed");
    let mut out = Vec::new();
    for (i, &b) in password.as_bytes().iter().enumerate() {
        out.push(b ^ key[i % 16]);
    }
    crate::mail::base64::encode_str(&out)
}

fn deobfuscate(encoded: &str) -> String {
    if encoded.is_empty() { return String::new(); }
    let data = crate::mail::base64::decode_str(encoded);
    if data.is_empty() { return String::new(); }  // Fallback if base64 decode fails
    let key = anyos_std::crypto::md5(b"anymail-key-seed");
    let mut out = Vec::new();
    for (i, &b) in data.iter().enumerate() {
        out.push(b ^ key[i % 16]);
    }
    String::from(core::str::from_utf8(&out).unwrap_or(""))
}
