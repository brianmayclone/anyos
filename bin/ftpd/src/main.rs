//! anyOS FTP daemon — RFC 959 compliant FTP server.
//!
//! # Architecture
//! - Reads `/System/etc/ftpd/ftpd.conf` at startup and on "reload" control commands.
//! - Reads `/System/etc/ftpd/shares.conf` for per-user directory access rules.
//! - Creates a named pipe "ftpd" for management commands (`reload`, `stop`).
//! - Listens on TCP port (default 21); forks a child process per connection.
//! - Maximum `MAX_CLIENTS` concurrent sessions.
//!
//! # Authentication
//! Users are authenticated against the anyOS user database via the authenticate syscall.
//! Anonymous access (user "anonymous" / "ftp") can be enabled in ftpd.conf.

#![no_std]
#![no_main]

anyos_std::entry!(main);

use anyos_std::{fs, ipc, net, process, println};

mod config;
mod session;

const MAX_CLIENTS: usize = 10;
const PIPE_NAME: &str = "ftpd";

fn print_config(cfg: &config::FtpdConfig) {
    println!("ftpd: --- config ---");
    println!("ftpd:   enabled       = {}", if cfg.enabled { "yes" } else { "no" });
    println!("ftpd:   port          = {}", cfg.port);
    println!("ftpd:   passive_mode  = {}", if cfg.passive_mode { "yes" } else { "no" });
    println!("ftpd:   passive_ports = {}-{}", cfg.passive_port_min, cfg.passive_port_max);
    println!("ftpd:   allow_anon    = {}", if cfg.allow_anonymous { "yes" } else { "no" });
    if cfg.allow_anonymous {
        println!("ftpd:   anon_root     = {}", cfg.anonymous_root_str());
    }
    println!("ftpd:   max_clients   = {}", cfg.max_clients);
    println!("ftpd:   chroot_users  = {}", if cfg.chroot_users { "yes" } else { "no" });
    println!("ftpd:   shares ({}):", cfg.shares_count);
    if cfg.shares_count == 0 {
        println!("ftpd:     (none)");
    }
    for i in 0..cfg.shares_count {
        let s = &cfg.shares[i];
        let perms = match (s.can_read, s.can_write) {
            (true, true)  => "rw",
            (true, false) => "r",
            (false, true) => "w",
            _             => "-",
        };
        println!("ftpd:     {}:{}: {}", s.user_str(), s.path_str(), perms);
    }
    println!("ftpd: ---");
}

/// Recursively create all directories in `path` that don't exist yet.
fn ensure_dir_exists(path: &str) {
    let mut st = [0u32; 7];
    if fs::stat(path, &mut st) != u32::MAX {
        return; // already exists
    }
    // Build each component and mkdir
    let mut cur = alloc::string::String::new();
    for part in path.split('/') {
        if part.is_empty() {
            cur.push('/');
            continue;
        }
        if !cur.ends_with('/') { cur.push('/'); }
        cur.push_str(part);
        if fs::stat(&cur, &mut st) == u32::MAX {
            fs::mkdir(&cur);
            println!("ftpd: created directory {}", cur);
        }
    }
}

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    if raw.contains("--help") {
        anyos_std::println!("ftpd - FTP server daemon\n\nUsage: ftpd");
        return;
    }

    println!("ftpd: starting");

    let mut cfg = config::load(); // Box<FtpdConfig> — lives on heap
    print_config(&cfg);

    // Ensure anonymous root directory exists (and parent dirs).
    if cfg.allow_anonymous {
        let root = cfg.anonymous_root_str();
        ensure_dir_exists(root);
    }
    // Ensure share directories exist.
    for i in 0..cfg.shares_count {
        ensure_dir_exists(cfg.shares[i].path_str());
    }

    let pipe_id = ipc::pipe_create(PIPE_NAME);
    if pipe_id == 0 || pipe_id == u32::MAX {
        println!("ftpd: failed to create control pipe");
        return;
    }

    // Query local IP for log output.
    let mut net_cfg = [0u8; 24];
    let mut local_ip = if net::get_config(&mut net_cfg) == 0 {
        [net_cfg[0], net_cfg[1], net_cfg[2], net_cfg[3]]
    } else {
        [0u8; 4]
    };

    let mut current_listener = if cfg.enabled {
        let l = net::tcp_listen(cfg.port, 10);
        if l == u32::MAX {
            println!("ftpd: tcp_listen failed on port {}", cfg.port);
            u32::MAX
        } else {
            println!("ftpd: listening on {}.{}.{}.{}:{}",
                local_ip[0], local_ip[1], local_ip[2], local_ip[3], cfg.port);
            l
        }
    } else {
        println!("ftpd: disabled (enabled=no in config)");
        u32::MAX
    };

    let mut children = [0u32; MAX_CLIENTS];
    let mut child_count = 0usize;

    loop {
        // ── Poll control pipe ────────────────────────────────────────────────
        let mut cmd_buf = [0u8; 64];
        let n = ipc::pipe_read(pipe_id, &mut cmd_buf);
        if n > 0 && n != u32::MAX {
            if let Ok(cmd) = core::str::from_utf8(&cmd_buf[..n as usize]) {
                let cmd = cmd.trim_end_matches('\n').trim();
                match cmd {
                    "stop" => {
                        println!("ftpd: stopping");
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
                        println!("ftpd: reloading config");
                        cfg = config::load();
                        print_config(&cfg);
                        if net::get_config(&mut net_cfg) == 0 {
                            local_ip = [net_cfg[0], net_cfg[1], net_cfg[2], net_cfg[3]];
                        }
                        if current_listener != u32::MAX {
                            net::tcp_close(current_listener);
                        }
                        if cfg.enabled {
                            current_listener = net::tcp_listen(cfg.port, 10);
                            if current_listener == u32::MAX {
                                println!("ftpd: tcp_listen failed on port {}", cfg.port);
                            } else {
                                println!("ftpd: listening on {}.{}.{}.{}:{}",
                                    local_ip[0], local_ip[1], local_ip[2], local_ip[3], cfg.port);
                            }
                        } else {
                            println!("ftpd: disabled after reload");
                            current_listener = u32::MAX;
                        }
                    }
                    _ => {}
                }
            }
        }

        // ── Reap finished children ───────────────────────────────────────────
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

        // ── Accept new connections ───────────────────────────────────────────
        if current_listener == u32::MAX || !cfg.enabled {
            process::sleep(50);
            continue;
        }

        if child_count >= MAX_CLIENTS {
            process::sleep(100);
            continue;
        }

        let (sock, remote_ip, _remote_port) = net::tcp_accept(current_listener);
        if sock == u32::MAX {
            continue;
        }

        if !cfg.enabled {
            net::tcp_close(sock);
            continue;
        }

        println!("ftpd: connection from {}.{}.{}.{}",
            remote_ip[0], remote_ip[1], remote_ip[2], remote_ip[3]);

        // Check client limit
        if child_count >= cfg.max_clients as usize {
            // Send 421 too many connections
            net::tcp_send(sock, b"421 Too many connections.\r\n");
            net::tcp_close(sock);
            continue;
        }

        let cfg_snapshot = cfg.clone(); // Box<FtpdConfig> — heap allocated
        let tid = process::fork();
        if tid == 0 {
            // Child process: handle FTP session
            session::run(sock, &*cfg_snapshot, remote_ip);
            process::exit(0);
        } else if tid != u32::MAX {
            if child_count < MAX_CLIENTS {
                children[child_count] = tid;
                child_count += 1;
            }
        } else {
            println!("ftpd: fork failed");
            net::tcp_close(sock);
        }
    }
}
