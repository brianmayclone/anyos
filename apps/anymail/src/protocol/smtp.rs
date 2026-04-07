// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! SMTP client implementation (RFC 5321, RFC 4954 AUTH).

use super::tcp_stream::{ConnError, TcpStream};
use crate::mail::base64;
use alloc::string::String;
use alloc::vec::Vec;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum SmtpError {
    Connection(ConnError),
    AuthFailed(String),
    ServerError(u16, String),
    SendFailed(String),
    NotConnected,
}

impl From<ConnError> for SmtpError {
    fn from(e: ConnError) -> Self {
        SmtpError::Connection(e)
    }
}

// ---------------------------------------------------------------------------
// SMTP Client
// ---------------------------------------------------------------------------

pub struct SmtpClient {
    stream: Option<TcpStream>,
    pub capabilities: Vec<String>,
}

impl SmtpClient {
    pub fn new() -> Self {
        Self {
            stream: None,
            capabilities: Vec::new(),
        }
    }

    /// Connect to an SMTP server.
    pub fn connect(&mut self, host: &str, port: u16, use_tls: bool) -> Result<(), SmtpError> {
        let mut stream = TcpStream::connect(host, port, use_tls)?;

        // Read greeting (may be multi-line: 220-... then 220 ...)
        loop {
            let line = stream.recv_line()?;
            if line.len() >= 4 && &line.as_bytes()[3..4] == b" " {
                // Final line of greeting
                let code = parse_smtp_code(&line);
                if code != 220 {
                    return Err(SmtpError::ServerError(code, line));
                }
                break;
            }
            // Continuation line (220-...)
        }

        self.stream = Some(stream);
        Ok(())
    }

    /// Send EHLO and parse capabilities.
    pub fn ehlo(&mut self, domain: &str) -> Result<(), SmtpError> {
        let cmd = alloc::format!("EHLO {}", domain);
        let lines = self.send_multiline(&cmd)?;

        self.capabilities.clear();
        for line in &lines {
            if line.len() > 4 {
                self.capabilities.push(String::from(&line[4..]));
            }
        }

        Ok(())
    }

    /// STARTTLS upgrade.
    pub fn starttls(&mut self) -> Result<(), SmtpError> {
        let resp = self.send_command("STARTTLS")?;
        let code = parse_smtp_code(&resp);
        if code != 220 {
            return Err(SmtpError::ServerError(code, resp));
        }
        self.stream_mut()?.upgrade_to_tls()?;
        // Re-EHLO after TLS upgrade
        Ok(())
    }

    /// AUTH LOGIN (base64 encoded user/pass).
    pub fn auth_login(&mut self, user: &str, pass: &str) -> Result<(), SmtpError> {
        let resp = self.send_command("AUTH LOGIN")?;
        let code = parse_smtp_code(&resp);
        if code != 334 {
            return Err(SmtpError::AuthFailed(resp));
        }

        // Send base64-encoded username
        let user_b64 = base64::encode_str(user.as_bytes());
        let resp = self.send_command(&user_b64)?;
        let code = parse_smtp_code(&resp);
        if code != 334 {
            return Err(SmtpError::AuthFailed(resp));
        }

        // Send base64-encoded password
        let pass_b64 = base64::encode_str(pass.as_bytes());
        let resp = self.send_command(&pass_b64)?;
        let code = parse_smtp_code(&resp);
        if code != 235 {
            return Err(SmtpError::AuthFailed(resp));
        }

        Ok(())
    }

    /// AUTH PLAIN (single base64 string: \0user\0pass).
    pub fn auth_plain(&mut self, user: &str, pass: &str) -> Result<(), SmtpError> {
        let mut plain = Vec::new();
        plain.push(0); // authzid (empty)
        plain.extend_from_slice(user.as_bytes());
        plain.push(0);
        plain.extend_from_slice(pass.as_bytes());

        let encoded = base64::encode_str(&plain);
        let cmd = alloc::format!("AUTH PLAIN {}", encoded);
        let resp = self.send_command(&cmd)?;
        let code = parse_smtp_code(&resp);
        if code != 235 {
            return Err(SmtpError::AuthFailed(resp));
        }

        Ok(())
    }

    /// Send an email.
    pub fn send_mail(
        &mut self,
        from: &str,
        recipients: &[&str],
        message_data: &[u8],
    ) -> Result<(), SmtpError> {
        // MAIL FROM
        let cmd = alloc::format!("MAIL FROM:<{}>", from);
        let resp = self.send_command(&cmd)?;
        let code = parse_smtp_code(&resp);
        if code != 250 {
            return Err(SmtpError::SendFailed(resp));
        }

        // RCPT TO for each recipient
        for rcpt in recipients {
            let cmd = alloc::format!("RCPT TO:<{}>", rcpt);
            let resp = self.send_command(&cmd)?;
            let code = parse_smtp_code(&resp);
            if code != 250 && code != 251 {
                return Err(SmtpError::SendFailed(resp));
            }
        }

        // DATA
        let resp = self.send_command("DATA")?;
        let code = parse_smtp_code(&resp);
        if code != 354 {
            return Err(SmtpError::SendFailed(resp));
        }

        // Send message body
        self.stream_mut()?.send_all(message_data)?;

        // Byte-stuff lines starting with "."
        // (The caller should handle this, but we add the terminator)

        // End with \r\n.\r\n
        if !message_data.ends_with(b"\r\n") {
            self.stream_mut()?.send_all(b"\r\n")?;
        }
        self.stream_mut()?.send_line(".")?;

        // Read response
        let resp = self.stream_mut()?.recv_line()?;
        let code = parse_smtp_code(&resp);
        if code != 250 {
            return Err(SmtpError::SendFailed(resp));
        }

        Ok(())
    }

    /// RSET - Reset the session (abort current mail transaction).
    pub fn rset(&mut self) -> Result<(), SmtpError> {
        let resp = self.send_command("RSET")?;
        let code = parse_smtp_code(&resp);
        if code != 250 {
            return Err(SmtpError::ServerError(code, resp));
        }
        Ok(())
    }

    /// QUIT and disconnect.
    pub fn quit(&mut self) {
        if self.stream.is_some() {
            let _ = self.send_command("QUIT");
            if let Some(mut s) = self.stream.take() {
                s.close();
            }
        }
    }

    /// Check if a capability is available.
    pub fn has_capability(&self, cap: &str) -> bool {
        let cap_upper = to_upper(cap);
        self.capabilities
            .iter()
            .any(|c| to_upper(c).starts_with(&cap_upper))
    }

    /// Check if connected.
    pub fn is_connected(&self) -> bool {
        self.stream.is_some()
    }

    // --- Internal helpers ---

    fn stream_mut(&mut self) -> Result<&mut TcpStream, SmtpError> {
        self.stream.as_mut().ok_or(SmtpError::NotConnected)
    }

    fn send_command(&mut self, cmd: &str) -> Result<String, SmtpError> {
        self.stream_mut()?.send_line(cmd)?;
        let resp = self.stream_mut()?.recv_line()?;
        Ok(resp)
    }

    /// Send a command and read a multi-line response.
    fn send_multiline(&mut self, cmd: &str) -> Result<Vec<String>, SmtpError> {
        self.stream_mut()?.send_line(cmd)?;

        let mut lines = Vec::new();
        loop {
            let line = self.stream_mut()?.recv_line()?;
            let is_last = line.len() >= 4 && line.as_bytes()[3] == b' ';
            lines.push(line);
            if is_last {
                break;
            }
        }

        // Check final status
        if let Some(last) = lines.last() {
            let code = parse_smtp_code(last);
            if code >= 400 {
                return Err(SmtpError::ServerError(code, last.clone()));
            }
        }

        Ok(lines)
    }
}

fn parse_smtp_code(line: &str) -> u16 {
    if line.len() >= 3 {
        line[..3].parse().unwrap_or(0)
    } else {
        0
    }
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
