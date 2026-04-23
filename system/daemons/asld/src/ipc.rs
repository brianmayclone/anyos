use alloc::format;
use alloc::string::{String, ToString};

use crate::config::ConfigStore;
use crate::errors::AsldError;
use crate::runtime::RuntimeService;

pub struct IpcState {
    pending_request: String,
}

impl IpcState {
    pub fn new() -> Self {
        Self {
            pending_request: String::new(),
        }
    }
}

pub fn handle_requests<S: ConfigStore>(
    runtime: &mut RuntimeService,
    store: &mut S,
    state: &mut IpcState,
    pipe_id: u32,
    buf: &mut [u8],
) -> bool {
    let n = anyos_std::ipc::pipe_read(pipe_id, buf);
    if n == 0 || n == u32::MAX {
        return false;
    }

    let data = match core::str::from_utf8(&buf[..n as usize]) {
        Ok(text) => text,
        Err(_) => return true,
    };
    state.pending_request.push_str(data);

    while let Some(pos) = state.pending_request.find('\n') {
        let mut line = state.pending_request[..pos].to_string();
        state.pending_request.drain(..=pos);
        if line.ends_with('\r') {
            line.pop();
        }
        if !line.is_empty() {
            handle_single_request(runtime, store, &line);
        }
    }

    true
}

fn handle_single_request<S: ConfigStore>(runtime: &mut RuntimeService, store: &mut S, line: &str) {
    let Some(tab_pos) = line.find('\t') else {
        return;
    };
    let Some(tid) = parse_u32(&line[..tab_pos]) else {
        return;
    };
    let cmd = line[tab_pos + 1..].trim();
    if cmd.is_empty() {
        return;
    }
    let response = dispatch(runtime, store, cmd);
    let reply_pipe_name = format!("asld-{}", tid);
    let reply_pipe = anyos_std::ipc::pipe_open(&reply_pipe_name);
    if reply_pipe != 0 {
        anyos_std::ipc::pipe_write(reply_pipe, response.as_bytes());
    }
}

fn dispatch<S: ConfigStore>(runtime: &mut RuntimeService, store: &mut S, cmd: &str) -> String {
    let (verb, rest) = split_first_word(cmd);
    match verb {
        "LIST" | "list" => match runtime.list(store) {
            Ok(items) => {
                let mut out = format!("OK\t{}\n", items.len());
                for item in items {
                    out.push_str(&format!("{}\t{}\t{}\n", item.name, item.state.as_str(), item.health.as_str()));
                }
                out.push('\n');
                out
            }
            Err(err) => err_line(&err),
        },
        "STATUS" | "status" => match runtime.status(store, rest) {
            Ok(status) => format!(
                "OK\t1\nname\t{}\nstate\t{}\nhealth\t{}\nagent\t{}\n\n",
                status.name,
                status.state.as_str(),
                status.health.as_str(),
                status.agent_state.as_str()
            ),
            Err(err) => err_line(&err),
        },
        "CREATE" | "create" => {
            let mut parts = rest.split_whitespace();
            let Some(name) = parts.next() else { return err_line(&AsldError::InvalidArgument("name")); };
            let Some(image_ref) = parts.next() else { return err_line(&AsldError::InvalidArgument("image_ref")); };
            let Some(owner) = parts.next() else { return err_line(&AsldError::InvalidArgument("owner")); };
            match runtime.create(store, name, image_ref, owner) {
                Ok(status) => format!("OK\t1\ncreated\t{}\nstate\t{}\n\n", status.name, status.state.as_str()),
                Err(err) => err_line(&err),
            }
        }
        "START" | "start" => match runtime.start(store, rest) {
            Ok(status) => format!("OK\t1\nstate\t{}\nhealth\t{}\n\n", status.state.as_str(), status.health.as_str()),
            Err(err) => err_line(&err),
        },
        "STOP" | "stop" => match runtime.stop(store, rest) {
            Ok(status) => format!("OK\t1\nstate\t{}\n\n", status.state.as_str()),
            Err(err) => err_line(&err),
        },
        "AGENT_STATUS" | "agent_status" => match runtime.status(store, rest) {
            Ok(status) => format!("OK\t1\nagent\t{}\n\n", status.agent_state.as_str()),
            Err(err) => err_line(&err),
        },
        _ => String::from("ERR\tunknown_command\n\n"),
    }
}

fn err_line(err: &AsldError) -> String {
    format!("ERR\t{}\t{}\n\n", err.code(), err.message())
}

fn split_first_word(s: &str) -> (&str, &str) {
    if let Some(pos) = s.find(' ') {
        (&s[..pos], s[pos + 1..].trim())
    } else {
        (s.trim(), "")
    }
}

fn parse_u32(s: &str) -> Option<u32> {
    let mut out = 0u32;
    if s.is_empty() {
        return None;
    }
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        out = out.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use crate::config::FakeStore;
    use crate::runtime::RuntimeService;

    use super::dispatch;

    #[test]
    fn list_empty_returns_zero_rows() {
        let mut runtime = RuntimeService::new();
        let mut store = FakeStore::default();
        let response = dispatch(&mut runtime, &mut store, "LIST");
        assert!(response.starts_with("OK\t0"));
    }

    #[test]
    fn create_then_status_roundtrip() {
        let mut runtime = RuntimeService::new();
        let mut store = FakeStore::default();
        let create = dispatch(&mut runtime, &mut store, "CREATE ubuntu-dev ubuntu-24.04-x86_64-v1 strati");
        assert!(create.starts_with("OK"));
        let status = dispatch(&mut runtime, &mut store, "STATUS ubuntu-dev");
        assert!(status.contains("name\tubuntu-dev"));
    }
}
