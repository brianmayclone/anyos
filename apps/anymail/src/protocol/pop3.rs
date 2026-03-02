//! POP3 client implementation (RFC 1939).

use alloc::string::String;
use alloc::vec::Vec;
use super::tcp_stream::{TcpStream, ConnError};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum Pop3Error {
    Connection(ConnError),
    AuthFailed(String),
    ServerError(String),
    NotConnected,
}

impl From<ConnError> for Pop3Error {
    fn from(e: ConnError) -> Self { Pop3Error::Connection(e) }
}

/// POP3 message listing entry.
pub struct Pop3Message {
    pub num: u32,
    pub size: u32,
    pub uidl: String,
}

// ---------------------------------------------------------------------------
// POP3 Client
// ---------------------------------------------------------------------------

pub struct Pop3Client {
    stream: Option<TcpStream>,
}

impl Pop3Client {
    pub fn new() -> Self {
        Self { stream: None }
    }

    /// Connect to a POP3 server.
    pub fn connect(&mut self, host: &str, port: u16, use_tls: bool) -> Result<String, Pop3Error> {
        let mut stream = TcpStream::connect(host, port, use_tls)?;

        // Read greeting
        let greeting = stream.recv_line()?;
        if !greeting.starts_with("+OK") {
            return Err(Pop3Error::ServerError(greeting));
        }

        self.stream = Some(stream);
        Ok(greeting)
    }

    /// STLS (STARTTLS upgrade).
    pub fn starttls(&mut self) -> Result<(), Pop3Error> {
        let resp = self.send_command("STLS")?;
        if !resp.starts_with("+OK") {
            return Err(Pop3Error::ServerError(resp));
        }
        self.stream_mut()?.upgrade_to_tls()?;
        Ok(())
    }

    /// Login with USER/PASS commands.
    pub fn login(&mut self, user: &str, pass: &str) -> Result<(), Pop3Error> {
        let cmd = alloc::format!("USER {}", user);
        let resp = self.send_command(&cmd)?;
        if !resp.starts_with("+OK") {
            return Err(Pop3Error::AuthFailed(resp));
        }

        let cmd = alloc::format!("PASS {}", pass);
        let resp = self.send_command(&cmd)?;
        if !resp.starts_with("+OK") {
            return Err(Pop3Error::AuthFailed(resp));
        }

        Ok(())
    }

    /// STAT - Get mailbox statistics.
    /// Returns (message_count, total_size).
    pub fn stat(&mut self) -> Result<(u32, u32), Pop3Error> {
        let resp = self.send_command("STAT")?;
        if !resp.starts_with("+OK") {
            return Err(Pop3Error::ServerError(resp));
        }

        let parts: Vec<&str> = resp.split_whitespace().collect();
        let count = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
        let size = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
        Ok((count, size))
    }

    /// LIST - Get message sizes.
    pub fn list(&mut self) -> Result<Vec<Pop3Message>, Pop3Error> {
        let resp = self.send_command("LIST")?;
        if !resp.starts_with("+OK") {
            return Err(Pop3Error::ServerError(resp));
        }

        let mut messages = Vec::new();
        loop {
            let line = self.stream_mut()?.recv_line()?;
            if line == "." { break; }

            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                let num: u32 = parts[0].parse().unwrap_or(0);
                let size: u32 = parts[1].parse().unwrap_or(0);
                messages.push(Pop3Message {
                    num,
                    size,
                    uidl: String::new(),
                });
            }
        }

        Ok(messages)
    }

    /// UIDL - Get unique IDs for all messages.
    pub fn uidl(&mut self) -> Result<Vec<(u32, String)>, Pop3Error> {
        let resp = self.send_command("UIDL")?;
        if !resp.starts_with("+OK") {
            return Err(Pop3Error::ServerError(resp));
        }

        let mut result = Vec::new();
        loop {
            let line = self.stream_mut()?.recv_line()?;
            if line == "." { break; }

            let parts: Vec<&str> = line.splitn(2, ' ').collect();
            if parts.len() >= 2 {
                let num: u32 = parts[0].parse().unwrap_or(0);
                result.push((num, String::from(parts[1])));
            }
        }

        Ok(result)
    }

    /// TOP - Get message headers (and optionally first N lines of body).
    pub fn top(&mut self, msg_num: u32, lines: u32) -> Result<Vec<u8>, Pop3Error> {
        let cmd = alloc::format!("TOP {} {}", msg_num, lines);
        let resp = self.send_command(&cmd)?;
        if !resp.starts_with("+OK") {
            return Err(Pop3Error::ServerError(resp));
        }

        self.read_multiline_response()
    }

    /// RETR - Retrieve a full message.
    pub fn retr(&mut self, msg_num: u32) -> Result<Vec<u8>, Pop3Error> {
        let cmd = alloc::format!("RETR {}", msg_num);
        let resp = self.send_command(&cmd)?;
        if !resp.starts_with("+OK") {
            return Err(Pop3Error::ServerError(resp));
        }

        self.read_multiline_response()
    }

    /// DELE - Mark a message for deletion.
    pub fn dele(&mut self, msg_num: u32) -> Result<(), Pop3Error> {
        let cmd = alloc::format!("DELE {}", msg_num);
        let resp = self.send_command(&cmd)?;
        if !resp.starts_with("+OK") {
            return Err(Pop3Error::ServerError(resp));
        }
        Ok(())
    }

    /// RSET - Unmark all messages marked for deletion.
    pub fn rset(&mut self) -> Result<(), Pop3Error> {
        let resp = self.send_command("RSET")?;
        if !resp.starts_with("+OK") {
            return Err(Pop3Error::ServerError(resp));
        }
        Ok(())
    }

    /// NOOP - Keep-alive.
    pub fn noop(&mut self) -> Result<(), Pop3Error> {
        let resp = self.send_command("NOOP")?;
        if !resp.starts_with("+OK") {
            return Err(Pop3Error::ServerError(resp));
        }
        Ok(())
    }

    /// QUIT - Disconnect (commits deletions).
    pub fn quit(&mut self) {
        if self.stream.is_some() {
            let _ = self.send_command("QUIT");
            if let Some(mut s) = self.stream.take() {
                s.close();
            }
        }
    }

    /// Check if connected.
    pub fn is_connected(&self) -> bool {
        self.stream.is_some()
    }

    // --- Internal helpers ---

    fn stream_mut(&mut self) -> Result<&mut TcpStream, Pop3Error> {
        self.stream.as_mut().ok_or(Pop3Error::NotConnected)
    }

    fn send_command(&mut self, cmd: &str) -> Result<String, Pop3Error> {
        self.stream_mut()?.send_line(cmd)?;
        let resp = self.stream_mut()?.recv_line()?;
        Ok(resp)
    }

    /// Read a multi-line response terminated by a "." line.
    /// Handles byte-stuffing (lines starting with ".." become ".").
    fn read_multiline_response(&mut self) -> Result<Vec<u8>, Pop3Error> {
        let mut data = Vec::new();

        loop {
            let line = self.stream_mut()?.recv_line()?;

            if line == "." {
                break;
            }

            // Byte-stuffing: ".." at start becomes "."
            let actual_line = if line.starts_with("..") {
                &line[1..]
            } else {
                &line
            };

            data.extend_from_slice(actual_line.as_bytes());
            data.push(b'\r');
            data.push(b'\n');
        }

        Ok(data)
    }
}
