//! TCP connection management — active open, passive open, and close.
//!
//! Implements `connect()`, `listen()`, `accept()`, `close()`,
//! `close_listener()`, `shutdown_write()`, and `status()`.

use super::send::{send_segment, send_segment_v6, send_syn_segment, send_syn_segment_v6};
use super::tcb::*;
use super::util::{alloc_ephemeral_port, insert_slot_hash, remove_slot_hash};
use super::{TCP_ACTIVE_OPENS, TCP_CONNECTIONS, TCP_PASSIVE_OPENS};
use crate::net::types::{Ipv4Addr, Ipv6Addr};
use core::sync::atomic::Ordering;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectStatus {
    InProgress,
    Established,
    Failed,
}

/// Active open: connect to a remote host. Returns socket ID or u32::MAX on error.
pub fn connect_start(remote_ip: Ipv4Addr, remote_port: u16) -> u32 {
    let cfg = crate::net::config();
    let local_ip = if remote_ip.0[0] == 127 {
        Ipv4Addr([127, 0, 0, 1])
    } else {
        cfg.ip
    };
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
                let mut tcb = Tcb::new(local_ip, local_port, remote_ip, remote_port);
                tcb.state = TcpState::SynSent;
                tcb.snd_nxt = tcb.snd_iss.wrapping_add(1);
                tcb.last_send_tick = crate::arch::hal::timer_current_ticks();
                tcb.owner_tid = tid;
                *slot = Some(tcb);
                insert_slot_hash(table, i);
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
        let table = match conns.as_ref() {
            Some(t) => t,
            None => return u32::MAX,
        };
        match table[slot_id].as_ref() {
            Some(tcb) => tcb.snd_iss,
            None => return u32::MAX,
        }
    };

    crate::serial_verbose_println!(
        "TCP: connecting to {}:{} from port {}",
        remote_ip,
        remote_port,
        local_port
    );
    TCP_ACTIVE_OPENS.fetch_add(1, Ordering::Relaxed);
    if !send_syn_segment(local_ip, local_port, remote_ip, remote_port, iss, 0, SYN) {
        let mut conns = TCP_CONNECTIONS.lock();
        if let Some(table) = conns.as_mut() {
            remove_slot_hash(table, slot_id);
            table[slot_id] = None;
        }
        return u32::MAX;
    }

    slot_id as u32
}

pub fn connect_status(socket_id: u32) -> ConnectStatus {
    let slot_id = socket_id as usize;
    let mut conns = TCP_CONNECTIONS.lock();
    let table = match conns.as_mut() {
        Some(t) => t,
        None => return ConnectStatus::Failed,
    };
    let Some(tcb) = table.get_mut(slot_id).and_then(Option::as_mut) else {
        return ConnectStatus::Failed;
    };
    match tcb.state {
        TcpState::Established => {
            tcb.waiting_tid = 0;
            ConnectStatus::Established
        }
        TcpState::Closed => {
            tcb.waiting_tid = 0;
            remove_slot_hash(table, slot_id);
            table[slot_id] = None;
            ConnectStatus::Failed
        }
        _ if tcb.reset_received => {
            remove_slot_hash(table, slot_id);
            table[slot_id] = None;
            ConnectStatus::Failed
        }
        _ => ConnectStatus::InProgress,
    }
}

/// Start an IPv6 active open without blocking.
pub fn connect_start_v6(remote_ip: Ipv6Addr, remote_port: u16) -> u32 {
    let cfg = crate::net::config();
    let local_ip6 = if remote_ip.is_loopback() {
        Ipv6Addr::LOOPBACK
    } else if remote_ip.is_link_local() {
        cfg.ipv6_link_local
    } else if !cfg.ipv6_addr.is_unspecified() {
        cfg.ipv6_addr
    } else {
        cfg.ipv6_link_local
    };
    if local_ip6.is_unspecified() {
        return u32::MAX;
    }

    let local_port = alloc_ephemeral_port();
    let tid = crate::task::scheduler::current_tid();

    let (slot_id, iss) = {
        let mut conns = TCP_CONNECTIONS.lock();
        let table = match conns.as_mut() {
            Some(t) => t,
            None => return u32::MAX,
        };
        let mut found = None;
        for (i, slot) in table.iter_mut().enumerate() {
            if slot.is_none() {
                let mut tcb = Tcb::new_v6(local_ip6, local_port, remote_ip, remote_port);
                tcb.state = TcpState::SynSent;
                tcb.snd_nxt = tcb.snd_iss.wrapping_add(1);
                tcb.owner_tid = tid;
                tcb.last_send_tick = crate::arch::hal::timer_current_ticks();
                let iss = tcb.snd_iss;
                *slot = Some(tcb);
                found = Some((i as u32, iss));
                break;
            }
        }
        match found {
            Some(v) => v,
            None => return u32::MAX,
        }
    };

    TCP_ACTIVE_OPENS.fetch_add(1, Ordering::Relaxed);
    if !send_syn_segment_v6(local_ip6, local_port, remote_ip, remote_port, iss, 0, SYN) {
        let mut conns = TCP_CONNECTIONS.lock();
        if let Some(table) = conns.as_mut() {
            remove_slot_hash(table, slot_id as usize);
            table[slot_id as usize] = None;
        }
        return u32::MAX;
    }

    slot_id
}

/// Active open: connect to a remote host. Returns socket ID or u32::MAX on error.
pub fn connect(remote_ip: Ipv4Addr, remote_port: u16, timeout_ticks: u32) -> u32 {
    let socket_id = connect_start(remote_ip, remote_port);
    if socket_id == u32::MAX {
        return u32::MAX;
    }
    let tid = crate::task::scheduler::current_tid();

    // Wait for connection to establish (blocking)
    let start = crate::arch::hal::timer_current_ticks();

    // Poll once eagerly to process any pending packets.
    crate::net::poll();

    loop {
        match connect_status(socket_id) {
            ConnectStatus::Established => {
                crate::serial_verbose_println!("TCP: connected socket {}", socket_id);
                return socket_id;
            }
            ConnectStatus::Failed => {
                crate::serial_verbose_println!("TCP: connection refused");
                return u32::MAX;
            }
            ConnectStatus::InProgress => {}
        }

        {
            let mut conns = TCP_CONNECTIONS.lock();
            let table = match conns.as_mut() {
                Some(t) => t,
                None => return u32::MAX,
            };
            if let Some(tcb) = table.get_mut(socket_id as usize).and_then(Option::as_mut) {
                tcb.waiting_tid = tid;
            } else {
                return u32::MAX;
            }
        }

        let now = crate::arch::hal::timer_current_ticks();
        if now.wrapping_sub(start) >= timeout_ticks {
            crate::serial_verbose_println!("TCP: connect timeout");
            let mut conns = TCP_CONNECTIONS.lock();
            if let Some(table) = conns.as_mut() {
                let slot_id = socket_id as usize;
                remove_slot_hash(table, slot_id);
                table[slot_id] = None;
            }
            return u32::MAX;
        }

        let wake_at = crate::arch::hal::timer_current_ticks() + 1;
        crate::task::scheduler::sleep_until(wake_at);
        crate::net::poll();
    }
}

/// Active open over IPv6: connect to a remote host.
pub fn connect_v6(remote_ip: Ipv6Addr, remote_port: u16, timeout_ticks: u32) -> u32 {
    let socket_id = connect_start_v6(remote_ip, remote_port);
    if socket_id == u32::MAX {
        return u32::MAX;
    }
    let tid = crate::task::scheduler::current_tid();

    let start = crate::arch::hal::timer_current_ticks();
    crate::net::poll();

    loop {
        match connect_status(socket_id) {
            ConnectStatus::Established => return socket_id,
            ConnectStatus::Failed => return u32::MAX,
            ConnectStatus::InProgress => {}
        }

        {
            let mut conns = TCP_CONNECTIONS.lock();
            let table = match conns.as_mut() {
                Some(t) => t,
                None => return u32::MAX,
            };
            if let Some(tcb) = table.get_mut(socket_id as usize).and_then(Option::as_mut) {
                tcb.waiting_tid = tid;
            } else {
                return u32::MAX;
            }
        }

        let now = crate::arch::hal::timer_current_ticks();
        if now.wrapping_sub(start) >= timeout_ticks {
            let mut conns = TCP_CONNECTIONS.lock();
            if let Some(table) = conns.as_mut() {
                remove_slot_hash(table, socket_id as usize);
                table[socket_id as usize] = None;
            }
            return u32::MAX;
        }
        let wake_at = crate::arch::hal::timer_current_ticks() + 1;
        crate::task::scheduler::sleep_until(wake_at);
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
                crate::serial_verbose_println!(
                    "TCP: port {} already listening (slot {} owner_tid={})",
                    port,
                    i,
                    tcb.owner_tid
                );
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
            crate::serial_verbose_println!("TCP: listening on port {} (socket {})", port, id);
            id as u32
        }
        None => {
            crate::serial_verbose_println!("TCP: no free slots for listen");
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
            let listen_valid = table[lid]
                .as_ref()
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
                let ready = table[i]
                    .as_ref()
                    .map(|tcb| {
                        !tcb.is_ipv6
                            && tcb.parent_listener == Some(lid as u16)
                            && tcb.state == TcpState::Established
                            && !tcb.accepted
                    })
                    .unwrap_or(false);

                if ready {
                    let tcb = match table[i].as_mut() {
                        Some(t) => t,
                        None => continue,
                    };
                    tcb.accepted = true;
                    tcb.parent_listener = None;
                    tcb.owner_tid = crate::task::scheduler::current_tid();
                    let rip = tcb.remote_ip;
                    let rport = tcb.remote_port;
                    if let Some(listener) = table[lid].as_mut() {
                        listener.waiting_tid = 0;
                    }
                    crate::serial_verbose_println!(
                        "TCP: accepted socket {} from {}:{}",
                        i,
                        rip,
                        rport
                    );
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
        crate::net::poll();
    }
}

/// Accept an IPv6 connection from a listening socket.
pub fn accept_v6(listener_id: u32, timeout_ticks: u32) -> (u32, Ipv6Addr, u16) {
    let lid = listener_id as usize;
    if lid >= MAX_CONNECTIONS {
        return (u32::MAX, Ipv6Addr::UNSPECIFIED, 0);
    }

    let tid = crate::task::scheduler::current_tid();
    let start = crate::arch::hal::timer_current_ticks();

    crate::net::poll();

    loop {
        {
            let mut conns = TCP_CONNECTIONS.lock();
            let table = match conns.as_mut() {
                Some(t) => t,
                None => return (u32::MAX, Ipv6Addr::UNSPECIFIED, 0),
            };

            let listen_valid = table[lid]
                .as_ref()
                .map(|t| t.state == TcpState::Listen)
                .unwrap_or(false);
            if !listen_valid {
                if let Some(tcb) = table[lid].as_mut() {
                    tcb.waiting_tid = 0;
                }
                return (u32::MAX, Ipv6Addr::UNSPECIFIED, 0);
            }

            for i in 0..table.len() {
                let ready = table[i]
                    .as_ref()
                    .map(|tcb| {
                        tcb.is_ipv6
                            && tcb.parent_listener == Some(lid as u16)
                            && tcb.state == TcpState::Established
                            && !tcb.accepted
                    })
                    .unwrap_or(false);

                if ready {
                    let tcb = match table[i].as_mut() {
                        Some(t) => t,
                        None => continue,
                    };
                    tcb.accepted = true;
                    tcb.parent_listener = None;
                    tcb.owner_tid = crate::task::scheduler::current_tid();
                    let rip = tcb.remote_ip6;
                    let rport = tcb.remote_port;
                    if let Some(listener) = table[lid].as_mut() {
                        listener.waiting_tid = 0;
                    }
                    TCP_PASSIVE_OPENS.fetch_add(1, Ordering::Relaxed);
                    return (i as u32, rip, rport);
                }
            }

            let now = crate::arch::hal::timer_current_ticks();
            if now.wrapping_sub(start) >= timeout_ticks {
                if let Some(tcb) = table[lid].as_mut() {
                    tcb.waiting_tid = 0;
                }
                return (u32::MAX, Ipv6Addr::UNSPECIFIED, 0);
            }

            if let Some(tcb) = table[lid].as_mut() {
                tcb.waiting_tid = tid;
            }
        }

        let wake_at = crate::arch::hal::timer_current_ticks() + 1;
        crate::task::scheduler::sleep_until(wake_at);
        crate::net::poll();
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

    let is_listener = table[id]
        .as_ref()
        .map(|t| t.state == TcpState::Listen)
        .unwrap_or(false);

    if !is_listener {
        return u32::MAX;
    }

    // Clean up pending connections
    for i in 0..table.len() {
        let is_pending = table[i]
            .as_ref()
            .map(|tcb| tcb.parent_listener == Some(id as u16) && !tcb.accepted)
            .unwrap_or(false);
        if is_pending {
            remove_slot_hash(table, i);
            table[i] = None;
        }
    }

    remove_slot_hash(table, id);
    table[id] = None;
    crate::serial_verbose_println!("TCP: listener socket {} closed", id);
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
    enum CloseInfo {
        V4(Ipv4Addr, u16, Ipv4Addr, u16, u32, u32, u16),
        V6(Ipv6Addr, u16, Ipv6Addr, u16, u32, u32, u16),
    }
    let send_info: Option<CloseInfo> = {
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
            TcpState::Established | TcpState::CloseWait => {
                tcb.state = if tcb.state == TcpState::Established {
                    TcpState::FinWait1
                } else {
                    TcpState::LastAck
                };
                let seq = tcb.snd_nxt;
                let ack_num = tcb.rcv_nxt;
                let win = tcb.advertised_window();
                tcb.last_send_tick = crate::arch::hal::timer_current_ticks();
                tcb.retransmit_count = 0;
                tcb.snd_nxt = tcb.snd_nxt.wrapping_add(1);
                if tcb.is_ipv6 {
                    Some(CloseInfo::V6(
                        tcb.local_ip6,
                        tcb.local_port,
                        tcb.remote_ip6,
                        tcb.remote_port,
                        seq,
                        ack_num,
                        win,
                    ))
                } else {
                    Some(CloseInfo::V4(
                        tcb.local_ip,
                        tcb.local_port,
                        tcb.remote_ip,
                        tcb.remote_port,
                        seq,
                        ack_num,
                        win,
                    ))
                }
            }
            TcpState::Closed => {
                remove_slot_hash(table, id);
                table[id] = None;
                return 0;
            }
            _ => {
                remove_slot_hash(table, id);
                table[id] = None;
                return 0;
            }
        }
    };

    match send_info {
        Some(CloseInfo::V4(lip, lp, rip, rp, seq, ack_num, win)) => {
            send_segment(lip, lp, rip, rp, seq, ack_num, FIN | ACK, win, &[]);
        }
        Some(CloseInfo::V6(lip6, lp, rip6, rp, seq, ack_num, win)) => {
            send_segment_v6(lip6, lp, rip6, rp, seq, ack_num, FIN | ACK, win, &[]);
        }
        None => {}
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
                            remove_slot_hash(table, id);
                            table[id] = None;
                        }
                        return 0;
                    }
                    _ => {}
                }
                if tcb.reset_received {
                    remove_slot_hash(table, id);
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
            enum RstInfo {
                V4(Ipv4Addr, u16, Ipv4Addr, u16, u32, u32),
                V6(Ipv6Addr, u16, Ipv6Addr, u16, u32, u32),
            }
            let rst_info = {
                let mut conns = TCP_CONNECTIONS.lock();
                let table = match conns.as_mut() {
                    Some(t) => t,
                    None => return 0,
                };
                let info = table[id].as_ref().map(|tcb| {
                    if tcb.is_ipv6 {
                        RstInfo::V6(
                            tcb.local_ip6,
                            tcb.local_port,
                            tcb.remote_ip6,
                            tcb.remote_port,
                            tcb.snd_nxt,
                            tcb.rcv_nxt,
                        )
                    } else {
                        RstInfo::V4(
                            tcb.local_ip,
                            tcb.local_port,
                            tcb.remote_ip,
                            tcb.remote_port,
                            tcb.snd_nxt,
                            tcb.rcv_nxt,
                        )
                    }
                });
                remove_slot_hash(table, id);
                table[id] = None;
                info
            };
            match rst_info {
                Some(RstInfo::V4(lip, lp, rip, rp, sn, rn)) => {
                    send_segment(lip, lp, rip, rp, sn, rn, RST, 0, &[]);
                }
                Some(RstInfo::V6(lip6, lp, rip6, rp, sn, rn)) => {
                    send_segment_v6(lip6, lp, rip6, rp, sn, rn, RST, 0, &[]);
                }
                None => {}
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

    enum ShutInfo {
        V4(Ipv4Addr, u16, Ipv4Addr, u16, u32, u32, u16),
        V6(Ipv6Addr, u16, Ipv6Addr, u16, u32, u32, u16),
    }
    let send_info: Option<ShutInfo> = {
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
                let seq = tcb.snd_nxt;
                let ack_num = tcb.rcv_nxt;
                let win = tcb.advertised_window();
                tcb.last_send_tick = crate::arch::hal::timer_current_ticks();
                tcb.retransmit_count = 0;
                tcb.snd_nxt = tcb.snd_nxt.wrapping_add(1);
                if tcb.is_ipv6 {
                    Some(ShutInfo::V6(
                        tcb.local_ip6,
                        tcb.local_port,
                        tcb.remote_ip6,
                        tcb.remote_port,
                        seq,
                        ack_num,
                        win,
                    ))
                } else {
                    Some(ShutInfo::V4(
                        tcb.local_ip,
                        tcb.local_port,
                        tcb.remote_ip,
                        tcb.remote_port,
                        seq,
                        ack_num,
                        win,
                    ))
                }
            }
            _ => None,
        }
    };

    match send_info {
        Some(ShutInfo::V4(lip, lp, rip, rp, seq, ack_num, win)) => {
            send_segment(lip, lp, rip, rp, seq, ack_num, FIN | ACK, win, &[]);
        }
        Some(ShutInfo::V6(lip6, lp, rip6, rp, seq, ack_num, win)) => {
            send_segment_v6(lip6, lp, rip6, rp, seq, ack_num, FIN | ACK, win, &[]);
        }
        None => {}
    }

    0
}
