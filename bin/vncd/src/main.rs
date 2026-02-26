//! anyOS VNC daemon — streams the compositor desktop via RFB/VNC.
//!
//! # Architecture
//! - Reads `/System/etc/vncd.conf` at startup and on "reload" control commands.
//! - Creates a named pipe "vncd" for management commands (`reload`, `stop`).
//! - Registers with the compositor event channel to inject keyboard/mouse input.
//! - Listens on TCP port (default 5900); forks a child process per connection.
//! - Maximum `MAX_CLIENTS` concurrent child sessions.
//!
//! # Authentication
//! 1. VNC type-2 (DES challenge/response) with the global VNC password.
//! 2. anyOS OS login screen: username + OS password, verified via `SYS_AUTHENTICATE`.
//!
//! The daemon stays running when `enabled=no` — it just refuses connections
//! until a "reload" command re-enables it.

#![no_std]
#![no_main]

anyos_std::entry!(main);

use anyos_std::{ipc, net, process, println};

mod config;
mod des;
mod font;
mod input;
mod login_ui;
mod rfb;

// ── Constants ─────────────────────────────────────────────────────────────────

/// Maximum concurrent VNC sessions. Each session allocates ~3-4 MB for the
/// pixel buffer, so limit to 2 to stay within heap budget.
const MAX_CLIENTS: usize = 2;

/// Named pipe used for management commands.
const PIPE_NAME: &str = "vncd";

/// Compositor event channel name.
const COMPOSITOR_CHAN: &str = "compositor";

// ── Entry point ───────────────────────────────────────────────────────────────

fn main() {
    println!("vncd: starting");

    // Load configuration.
    let mut cfg = config::load();

    // Create control pipe for management commands.
    let pipe_id = ipc::pipe_create(PIPE_NAME);
    if pipe_id == 0 || pipe_id == u32::MAX {
        println!("vncd: failed to create control pipe");
        return;
    }

    // Obtain compositor event channel ID (for input injection).
    // evt_chan_create returns the hash of the channel name; if the compositor
    // already created it, this returns the existing channel's ID (idempotent).
    let comp_chan = ipc::evt_chan_create(COMPOSITOR_CHAN);

    // Query local IP for log output.
    let mut net_cfg = [0u8; 24];
    let mut local_ip = if net::get_config(&mut net_cfg) == 0 {
        [net_cfg[0], net_cfg[1], net_cfg[2], net_cfg[3]]
    } else {
        [0u8; 4]
    };

    // Start TCP listener.
    let listener = if cfg.enabled {
        let l = net::tcp_listen(cfg.port, 4);
        if l == u32::MAX {
            println!("vncd: tcp_listen failed on port {}", cfg.port);
            u32::MAX
        } else {
            println!("vncd: listening on {}.{}.{}.{}:{}",
                local_ip[0], local_ip[1], local_ip[2], local_ip[3], cfg.port);
            l
        }
    } else {
        println!("vncd: disabled (enabled=no in config)");
        u32::MAX
    };

    // Track child session PIDs so we can reap them.
    let mut children = [0u32; MAX_CLIENTS];
    let mut child_count = 0usize;

    // Main accept / control loop.
    let mut current_listener = listener;
    loop {
        // ── Poll control pipe ─────────────────────────────────────────────
        let mut cmd_buf = [0u8; 64];
        let n = ipc::pipe_read(pipe_id, &mut cmd_buf);
        if n > 0 && n != u32::MAX {
            if let Ok(cmd) = core::str::from_utf8(&cmd_buf[..n as usize]) {
                let cmd = cmd.trim_end_matches('\n').trim();
                match cmd {
                    "stop" => {
                        println!("vncd: stopping");
                        // Kill all child sessions.
                        for i in 0..child_count {
                            process::kill(children[i]);
                        }
                        if current_listener != u32::MAX {
                            net::tcp_close(current_listener);
                        }
                        ipc::pipe_close(pipe_id);
                        return;
                    }
                    "reload" => {
                        println!("vncd: reloading config");
                        cfg = config::load();
                        // Re-query local IP (may have changed via DHCP).
                        if net::get_config(&mut net_cfg) == 0 {
                            local_ip = [net_cfg[0], net_cfg[1], net_cfg[2], net_cfg[3]];
                        }
                        // Restart listener if needed.
                        if current_listener != u32::MAX {
                            net::tcp_close(current_listener);
                        }
                        if cfg.enabled {
                            current_listener = net::tcp_listen(cfg.port, 4);
                            if current_listener == u32::MAX {
                                println!("vncd: tcp_listen failed on port {}", cfg.port);
                            } else {
                                println!("vncd: listening on {}.{}.{}.{}:{}",
                                    local_ip[0], local_ip[1], local_ip[2], local_ip[3], cfg.port);
                            }
                        } else {
                            println!("vncd: disabled after reload");
                            current_listener = u32::MAX;
                        }
                    }
                    _ => {}
                }
            }
        }

        // ── Reap finished children ────────────────────────────────────────
        let mut new_count = 0usize;
        let mut new_children = [0u32; MAX_CLIENTS];
        for i in 0..child_count {
            let status = process::try_waitpid(children[i]);
            if status == process::STILL_RUNNING {
                if new_count < MAX_CLIENTS {
                    new_children[new_count] = children[i];
                    new_count += 1;
                }
            }
        }
        child_count = new_count;
        children = new_children;

        // ── Accept new connections ────────────────────────────────────────
        if current_listener == u32::MAX || !cfg.enabled {
            // Nothing to accept — yield and try again.
            process::sleep(50);
            continue;
        }

        if child_count >= MAX_CLIENTS {
            // Too many sessions — wait before trying to accept.
            process::sleep(100);
            continue;
        }

        // tcp_accept blocks for up to ~30s; use a short-polling strategy
        // by briefly checking the status. Since we can't do non-blocking accept
        // directly, we yield here and the network stack will wake us on connect.
        let (sock, _ip, _port) = net::tcp_accept(current_listener);
        if sock == u32::MAX {
            // Timeout or error — continue.
            continue;
        }

        if !cfg.enabled {
            // Accepted a connection while disabled — close immediately.
            net::tcp_close(sock);
            continue;
        }

        // Fork a child process to handle this VNC session.
        // The child inherits `sock`, `comp_chan`, and a copy of `cfg`.
        //
        // NOTE: anyOS socket IDs are global — NOT per-process file descriptors.
        // tcp_close() on a socket ID destroys it system-wide.  Therefore:
        //   - Child must NOT close the listener (parent needs it).
        //   - Parent must NOT close the accepted socket (child needs it).
        let cfg_snapshot = cfg.clone();
        let tid = process::fork();
        if tid == 0 {
            // ── Child process ─────────────────────────────────────────────
            // Run the RFB session (blocks until client disconnects).
            rfb::run_session(sock, &cfg_snapshot, comp_chan);
            process::exit(0);
        } else if tid != u32::MAX {
            // ── Parent process ────────────────────────────────────────────
            // Do NOT close `sock` — child owns it and will close on exit.
            if child_count < MAX_CLIENTS {
                children[child_count] = tid;
                child_count += 1;
            }
        } else {
            // Fork failed — close socket (no child will use it).
            println!("vncd: fork failed");
            net::tcp_close(sock);
        }
    }
}
