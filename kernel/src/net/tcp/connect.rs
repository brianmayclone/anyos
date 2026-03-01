//! TCP connection management — active open, passive open, and close.
//!
//! Implements `connect()`, `listen()`, `accept()`, `close()`,
//! `close_listener()`, `shutdown_write()`, and `status()`.

use core::sync::atomic::Ordering;
use super::tcb::*;
use super::send::{send_segment, send_syn_segment};
use super::util::alloc_ephemeral_port;
use super::{TCP_CONNECTIONS, TCP_ACTIVE_OPENS, TCP_PASSIVE_OPENS};
use crate::net::types::Ipv4Addr;

/// Active open: connect to a remote host. Returns socket ID or u32::MAX on error.
pub fn connect(remote_ip: Ipv4Addr, remote_port: u16, timeout_ticks: u32) -> u32 {
    let cfg = crate::net::config();
    let local_port = alloc_ephemeral_port();
    let tid = crate::task::scheduler::current_tid();

    // Find a free slot and insert TCB
    let slot_id = {
        let mut conns = TCP_CONNECTIONS.lock();
        let table = match conns.as_mut() {
            Some(t) => t,
            None => return u32::MAX,
        };
        let mut found = None;
        for (i, slot) in table.iter_mut().enumerate() {
            if slot.is_none() {
                let mut tcb = Tcb::new(cfg.ip, local_port, remote_ip, remote_port);
                tcb.state = TcpState::SynSent;
                tcb.snd_nxt = tcb.snd_iss.wrapping_add(1);
                tcb.last_send_tick = crate::arch::hal::timer_current_ticks();
                tcb.owner_tid = tid;
                *slot = Some(tcb);
                found = Some(i);
                break;
            }
        }
        match found {
            Some(id) => id,
            None => return u32::MAX,
        }
    };

    // Send SYN (outside lock!)
    let iss = {
        let conns = TCP_CONNECTIONS.lock();
        let table = conns.as_ref().unwrap();
        table[slot_id].as_ref().unwrap().snd_iss
    };

    crate::serial_println!("TCP: connecting to {}:{} from port {}", remote_ip, remote_port, local_port);
    TCP_ACTIVE_OPENS.fetch_add(1, Ordering::Relaxed);
    send_syn_segment(cfg.ip, local_port, remote_ip, remote_port, iss, 0, SYN);

    // Wait for connection to establish (blocking)
    let start = crate::arch::hal::timer_current_ticks();

    // Poll once eagerly to process any pending packets.
    crate::net::poll();

    loop {
        {
            let mut conns = TCP_CONNECTIONS.lock();
            let table = conns.as_mut().unwrap();
            if let Some(tcb) = table[slot_id].as_mut() {
                match tcb.state {
                    TcpState::Established => {
                        tcb.waiting_tid = 0;
                        crate::serial_println!("TCP: connected socket {}", slot_id);
                        return slot_id as u32;
                    }
                    TcpState::Closed => {
                        tcb.waiting_tid = 0;
                        crate::serial_println!("TCP: connection refused");
                        return u32::MAX;
                    }
                    _ => {}
                }
                if tcb.reset_received {
                    table[slot_id] = None;
                    return u32::MAX;
                }

                let now = crate::arch::hal::timer_current_ticks();
                if now.wrapping_sub(start) >= timeout_ticks {
                    crate::serial_println!("TCP: connect timeout");
                    table[slot_id] = None;
                    return u32::MAX;
                }

                tcb.waiting_tid = tid;
            } else {
                return u32::MAX;
            }
        }

        let wake_at = crate::arch::hal::timer_current_ticks() + 1;
        crate::task::scheduler::sleep_until(wake_at);
        crate::net::poll_rx();
    }
}

/// Passive open: listen on a local port. Returns listener socket ID or u32::MAX.
pub fn listen(port: u16, _backlog: u16) -> u32 {
    let cfg = crate::net::config();
    let tid = crate::task::scheduler::current_tid();
    let mut conns = TCP_CONNECTIONS.lock();
    let table = match conns.as_mut() {
        Some(t) => t,
        None => return u32::MAX,
    };

    // Check if port is already in use
    for (i, slot) in table.iter().enumerate() {
        if let Some(tcb) = slot {
            if tcb.local_port == port && tcb.state == TcpState::Listen {
                crate::serial_println!("TCP: port {} already listening (slot {} owner_tid={})",
                    port, i, tcb.owner_tid);
                return u32::MAX;
            }
        }
    }

    // Find a free slot
    let mut found = None;
    for (i, slot) in table.iter_mut().enumerate() {
        if slot.is_none() {
            let mut tcb = Tcb::new(cfg.ip, port, Ipv4Addr([0, 0, 0, 0]), 0);
            tcb.state = TcpState::Listen;
            tcb.owner_tid = tid;
            *slot = Some(tcb);
            found = Some(i);
            break;
        }
    }

    match found {
        Some(id) => {
            crate::serial_println!("TCP: listening on port {} (socket {})", port, id);
            id as u32
        }
        None => {
            crate::serial_println!("TCP: no free slots for listen");
            u32::MAX
        }
    }
}

/// Accept a connection from a listening socket.
/// Blocks until a connection is established or timeout.
pub fn accept(listener_id: u32, timeout_ticks: u32) -> (u32, Ipv4Addr, u16) {
    let lid = listener_id as usize;
    if lid >= MAX_CONNECTIONS {
        return (u32::MAX, Ipv4Addr([0; 4]), 0);
    }

    let tid = crate::task::scheduler::current_tid();
    let start = crate::arch::hal::timer_current_ticks();

    // Poll once eagerly
    crate::net::poll();

    loop {
        {
            let mut conns = TCP_CONNECTIONS.lock();
            let table = match conns.as_mut() {
                Some(t) => t,
                None => return (u32::MAX, Ipv4Addr([0; 4]), 0),
            };

            // Verify listener is still valid
            let listen_valid = table[lid].as_ref()
                .map(|t| t.state == TcpState::Listen)
                .unwrap_or(false);
            if !listen_valid {
                if let Some(tcb) = table[lid].as_mut() {
                    tcb.waiting_tid = 0;
                }
                return (u32::MAX, Ipv4Addr([0; 4]), 0);
            }

            // Find a ready connection spawned by this listener
            for i in 0..table.len() {
                let ready = table[i].as_ref().map(|tcb| {
                    tcb.parent_listener == Some(lid as u8)
                        && tcb.state == TcpState::Established
                        && !tcb.accepted
                }).unwrap_or(false);

                if ready {
                    let tcb = table[i].as_mut().unwrap();
                    tcb.accepted = true;
                    tcb.parent_listener = None;
                    tcb.owner_tid = crate::task::scheduler::current_tid();
                    let rip = tcb.remote_ip;
                    let rport = tcb.remote_port;
                    if let Some(listener) = table[lid].as_mut() {
                        listener.waiting_tid = 0;
                    }
                    crate::serial_println!("TCP: accepted socket {} from {}:{}", i, rip, rport);
                    TCP_PASSIVE_OPENS.fetch_add(1, Ordering::Relaxed);
                    return (i as u32, rip, rport);
                }
            }

            // Check timeout
            let now = crate::arch::hal::timer_current_ticks();
            if now.wrapping_sub(start) >= timeout_ticks {
                if let Some(tcb) = table[lid].as_mut() {
                    tcb.waiting_tid = 0;
                }
                return (u32::MAX, Ipv4Addr([0; 4]), 0);
            }

            if let Some(tcb) = table[lid].as_mut() {
                tcb.waiting_tid = tid;
            }
        }

        let wake_at = crate::arch::hal::timer_current_ticks() + 1;
        crate::task::scheduler::sleep_until(wake_at);
        crate::net::poll_rx();
    }
}

/// Close a listening socket. Also cleans up any pending (unaccepted) connections.
pub fn close_listener(socket_id: u32) -> u32 {
    let id = socket_id as usize;
    if id >= MAX_CONNECTIONS {
        return u32::MAX;
    }

    let mut conns = TCP_CONNECTIONS.lock();
    let table = match conns.as_mut() {
        Some(t) => t,
        None => return u32::MAX,
    };

    let is_listener = table[id].as_ref()
        .map(|t| t.state == TcpState::Listen)
        .unwrap_or(false);

    if !is_listener {
        return u32::MAX;
    }

    // Clean up pending connections
    for i in 0..table.len() {
        let is_pending = table[i].as_ref().map(|tcb| {
            tcb.parent_listener == Some(id as u8) && !tcb.accepted
        }).unwrap_or(false);
        if is_pending {
            table[i] = None;
        }
    }

    table[id] = None;
    crate::serial_println!("TCP: listener socket {} closed", id);
    0
}

/// Close a TCP connection. Sends FIN, waits for ACK.
pub fn close(socket_id: u32) -> u32 {
    let id = socket_id as usize;
    if id >= MAX_CONNECTIONS {
        return u32::MAX;
    }

    // Check if it's a listener
    {
        let conns = TCP_CONNECTIONS.lock();
        if let Some(table) = conns.as_ref() {
            if let Some(tcb) = &table[id] {
                if tcb.state == TcpState::Listen {
                    drop(conns);
                    return close_listener(socket_id);
                }
            }
        }
    }

    // Get info and update state
    let send_info = {
        let mut conns = TCP_CONNECTIONS.lock();
        let table = match conns.as_mut() {
            Some(t) => t,
            None => return u32::MAX,
        };
        let tcb = match table[id].as_mut() {
            Some(t) => t,
            None => return u32::MAX,
        };

        match tcb.state {
            TcpState::Established => {
                tcb.state = TcpState::FinWait1;
                let info = (tcb.local_ip, tcb.local_port, tcb.remote_ip, tcb.remote_port,
                           tcb.snd_nxt, tcb.rcv_nxt, tcb.advertised_window());
                tcb.last_send_tick = crate::arch::hal::timer_current_ticks();
                tcb.retransmit_count = 0;
                tcb.snd_nxt = tcb.snd_nxt.wrapping_add(1);
                Some(info)
            }
            TcpState::CloseWait => {
                tcb.state = TcpState::LastAck;
                let info = (tcb.local_ip, tcb.local_port, tcb.remote_ip, tcb.remote_port,
                           tcb.snd_nxt, tcb.rcv_nxt, tcb.advertised_window());
                tcb.last_send_tick = crate::arch::hal::timer_current_ticks();
                tcb.retransmit_count = 0;
                tcb.snd_nxt = tcb.snd_nxt.wrapping_add(1);
                Some(info)
            }
            TcpState::Closed => {
                table[id] = None;
                return 0;
            }
            _ => {
                table[id] = None;
                return 0;
            }
        }
    };

    if let Some((local_ip, local_port, remote_ip, remote_port, seq, ack_num, win)) = send_info {
        send_segment(local_ip, local_port, remote_ip, remote_port, seq, ack_num, FIN | ACK, win, &[]);
    }

    // Wait for close to complete (with timeout)
    let start = crate::arch::hal::timer_current_ticks();
    let timeout = 500; // 5 seconds
    loop {
        crate::net::poll();

        {
            let mut conns = TCP_CONNECTIONS.lock();
            let table = match conns.as_mut() {
                Some(t) => t,
                None => return 0,
            };
            if let Some(tcb) = &table[id] {
                match tcb.state {
                    TcpState::Closed | TcpState::TimeWait => {
                        if tcb.state == TcpState::Closed {
                            table[id] = None;
                        }
                        return 0;
                    }
                    _ => {}
                }
                if tcb.reset_received {
                    table[id] = None;
                    return 0;
                }
            } else {
                return 0;
            }
        }

        let now = crate::arch::hal::timer_current_ticks();
        if now.wrapping_sub(start) >= timeout {
            // Force close with RST
            let rst_info = {
                let mut conns = TCP_CONNECTIONS.lock();
                let table = conns.as_mut().unwrap();
                let info = table[id].as_ref().map(|tcb| {
                    (tcb.local_ip, tcb.local_port, tcb.remote_ip, tcb.remote_port,
                     tcb.snd_nxt, tcb.rcv_nxt)
                });
                table[id] = None;
                info
            };
            if let Some((lip, lp, rip, rp, sn, rn)) = rst_info {
                send_segment(lip, lp, rip, rp, sn, rn, RST, 0, &[]);
            }
            return 0;
        }

        let wake_at = crate::arch::hal::timer_current_ticks() + 5;
        crate::task::scheduler::sleep_until(wake_at);
    }
}

/// Get connection state. Returns TcpState as u32, or u32::MAX if not found.
pub fn status(socket_id: u32) -> u32 {
    let id = socket_id as usize;
    if id >= MAX_CONNECTIONS {
        return u32::MAX;
    }
    let conns = TCP_CONNECTIONS.lock();
    let table = match conns.as_ref() {
        Some(t) => t,
        None => return u32::MAX,
    };
    match &table[id] {
        Some(tcb) => tcb.state as u32,
        None => u32::MAX,
    }
}

/// Half-close (SHUT_WR): send FIN but don't block. Connection can still receive.
pub fn shutdown_write(socket_id: u32) -> u32 {
    let id = socket_id as usize;
    if id >= MAX_CONNECTIONS {
        return u32::MAX;
    }

    let send_info = {
        let mut conns = TCP_CONNECTIONS.lock();
        let table = match conns.as_mut() {
            Some(t) => t,
            None => return u32::MAX,
        };
        let tcb = match table[id].as_mut() {
            Some(t) => t,
            None => return u32::MAX,
        };

        match tcb.state {
            TcpState::Established => {
                tcb.state = TcpState::FinWait1;
                let info = (tcb.local_ip, tcb.local_port, tcb.remote_ip, tcb.remote_port,
                           tcb.snd_nxt, tcb.rcv_nxt, tcb.advertised_window());
                tcb.last_send_tick = crate::arch::hal::timer_current_ticks();
                tcb.retransmit_count = 0;
                tcb.snd_nxt = tcb.snd_nxt.wrapping_add(1);
                Some(info)
            }
            _ => None,
        }
    };

    if let Some((local_ip, local_port, remote_ip, remote_port, seq, ack_num, win)) = send_info {
        send_segment(local_ip, local_port, remote_ip, remote_port, seq, ack_num, FIN | ACK, win, &[]);
    }

    0
}
