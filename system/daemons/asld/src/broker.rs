use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::errors::AsldError;
use crate::model::{DistroConfig, ShellSession};

pub fn sync_distro(cfg: &DistroConfig) -> Result<(), AsldError> {
    sync_network(cfg)?;
    sync_filesystem(cfg)?;
    Ok(())
}

pub fn clear_distro(name: &str) -> Result<(), AsldError> {
    request("aslnetd", &format!("CLEAR {}", name))?;
    request("aslfsd", &format!("CLEAR {}", name))?;
    request("aslconsoled", &format!("CLEAR {}", name))?;
    Ok(())
}

pub fn record_observation(name: &str, event: &str) -> Result<(), AsldError> {
    request("aslobsd", &format!("EVENT {}\t{}", name, event))
}

pub fn attach_console_session(name: &str, session: &ShellSession) -> Result<(), AsldError> {
    request(
        "aslconsoled",
        &format!(
            "ATTACH {}\t{}\t{}\t{}\t{}",
            name,
            session.session_id,
            session.mode.as_str(),
            session.console_pipe_name,
            session.stdin_pipe_name
        ),
    )
}

pub fn write_console_bytes(name: &str, bytes: &[u8]) -> Result<(), AsldError> {
    if bytes.is_empty() {
        return Ok(());
    }
    request(
        "aslconsoled",
        &format!("WRITE {}\t{}", name, encode_hex(bytes)),
    )
}

pub fn network_tx_frame(name: &str, bytes: &[u8]) -> Result<(), AsldError> {
    if bytes.is_empty() {
        return Ok(());
    }
    request("aslnetd", &format!("TX {}\t{}", name, encode_hex(bytes)))
}

pub fn network_rx_frame(name: &str, max_bytes: usize) -> Result<Option<Vec<u8>>, AsldError> {
    let lines = request_lines("aslnetd", &format!("RX_POLL {}\t{}", name, max_bytes))?;
    let mut has_packet = false;
    let mut data = None;
    for line in lines {
        if line == "packet\ttrue" {
            has_packet = true;
        } else if let Some(raw) = line.strip_prefix("data\t") {
            data = decode_hex(raw);
        }
    }
    Ok(if has_packet { data } else { None })
}

pub fn status(pipe_name: &'static str) -> Result<Vec<String>, AsldError> {
    request_lines(pipe_name, "STATUS")
}

pub fn sync_network(cfg: &DistroConfig) -> Result<(), AsldError> {
    request("aslnetd", &format!("CLEAR {}", cfg.name))?;
    for rule in &cfg.port_forwards {
        request(
            "aslnetd",
            &format!(
                "APPLY {}\t{}\t{}\t{}\t{}\t{}",
                cfg.name,
                rule.id,
                rule.listen_address,
                rule.listen_port,
                rule.guest_port,
                rule.protocol
            ),
        )?;
    }
    Ok(())
}

pub fn sync_filesystem(cfg: &DistroConfig) -> Result<(), AsldError> {
    request("aslfsd", &format!("CLEAR {}", cfg.name))?;
    for mount in &cfg.mounts {
        request(
            "aslfsd",
            &format!(
                "APPLY {}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                cfg.name,
                mount.id,
                mount.host_path,
                mount.guest_path,
                mount.mode,
                mount.metadata_mode,
                mount.case_mode,
                mount.exec_policy,
                mount.watch_policy
            ),
        )?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn request(pipe_name: &'static str, command: &str) -> Result<(), AsldError> {
    let _ = request_lines(pipe_name, command)?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn request_lines(pipe_name: &'static str, _command: &str) -> Result<Vec<String>, AsldError> {
    Ok(alloc::vec![
        format!("broker\t{}", pipe_name),
        String::from("state\thost-unchecked"),
    ])
}

#[cfg(not(target_os = "linux"))]
fn request(pipe_name: &'static str, command: &str) -> Result<(), AsldError> {
    let _ = request_lines(pipe_name, command)?;
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn request_lines(pipe_name: &'static str, command: &str) -> Result<Vec<String>, AsldError> {
    let reply_name = format!("{}-1", pipe_name);
    let old_reply = anyos_std::ipc::pipe_open(&reply_name);
    if old_reply != 0 {
        anyos_std::ipc::pipe_close(old_reply);
    }
    let reply_pipe = anyos_std::ipc::pipe_create(&reply_name);
    if reply_pipe == 0 || reply_pipe == u32::MAX {
        return Err(broker_error(pipe_name));
    }
    let main_pipe = anyos_std::ipc::pipe_open(pipe_name);
    if main_pipe == 0 || main_pipe == u32::MAX {
        anyos_std::ipc::pipe_close(reply_pipe);
        return Err(broker_error(pipe_name));
    }

    let line = format!("1\t{}\n", command);
    let written = anyos_std::ipc::pipe_write(main_pipe, line.as_bytes());
    if written == 0 || written == u32::MAX {
        anyos_std::ipc::pipe_close(reply_pipe);
        return Err(broker_error(pipe_name));
    }

    let mut data = alloc::vec::Vec::new();
    let mut chunk = [0u8; 512];
    for _ in 0..50 {
        let n = anyos_std::ipc::pipe_read(reply_pipe, &mut chunk);
        if n == u32::MAX {
            anyos_std::ipc::pipe_close(reply_pipe);
            return Err(broker_error(pipe_name));
        }
        if n > 0 {
            data.extend_from_slice(&chunk[..n as usize]);
            if data.ends_with(b"\n\n") {
                anyos_std::ipc::pipe_close(reply_pipe);
                return parse_response(pipe_name, &data);
            }
        } else {
            anyos_std::process::sleep(10);
        }
    }
    anyos_std::ipc::pipe_close(reply_pipe);
    Err(broker_error(pipe_name))
}

#[cfg(not(target_os = "linux"))]
fn parse_response(pipe_name: &'static str, data: &[u8]) -> Result<Vec<String>, AsldError> {
    let Ok(text) = core::str::from_utf8(data) else {
        return Err(broker_error(pipe_name));
    };
    let trimmed = text.trim_matches('\n');
    let mut lines = trimmed.split('\n');
    let Some(header) = lines.next() else {
        return Err(broker_error(pipe_name));
    };
    if !header.starts_with("OK\t") {
        return Err(broker_error(pipe_name));
    }
    let mut out = Vec::new();
    for line in lines {
        if !line.is_empty() {
            out.push(String::from(line));
        }
    }
    Ok(out)
}

#[cfg_attr(target_os = "linux", allow(dead_code))]
fn broker_error(pipe_name: &'static str) -> AsldError {
    match pipe_name {
        "aslnetd" => AsldError::BackendUnavailable("aslnetd"),
        "aslfsd" => AsldError::BackendUnavailable("aslfsd"),
        "aslconsoled" => AsldError::BackendUnavailable("aslconsoled"),
        "aslobsd" => AsldError::BackendUnavailable("aslobsd"),
        _ => AsldError::BackendUnavailable("asl broker"),
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::new();
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

fn decode_hex(input: &str) -> Option<Vec<u8>> {
    if input.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::new();
    let bytes = input.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        let high = hex_nibble(bytes[index])?;
        let low = hex_nibble(bytes[index + 1])?;
        out.push((high << 4) | low);
        index += 2;
    }
    Some(out)
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{decode_hex, encode_hex};

    #[test]
    fn encodes_console_bytes_as_hex() {
        assert_eq!(encode_hex(b"A\n"), "410a");
    }

    #[test]
    fn decodes_packet_hex() {
        assert_eq!(
            decode_hex("deadBEEF"),
            Some(alloc::vec![0xde, 0xad, 0xbe, 0xef])
        );
        assert_eq!(decode_hex("abc"), None);
    }
}
