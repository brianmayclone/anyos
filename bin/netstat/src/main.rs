#![no_std]
#![no_main]

anyos_std::entry!(main);

/// TCP state names matching kernel TcpState enum.
fn state_name(s: u8) -> &'static str {
    match s {
        0 => "CLOSED",
        1 => "SYN_SENT",
        2 => "ESTABLISHED",
        3 => "FIN_WAIT1",
        4 => "FIN_WAIT2",
        5 => "TIME_WAIT",
        6 => "CLOSE_WAIT",
        7 => "LAST_ACK",
        8 => "LISTEN",
        9 => "SYN_RECV",
        _ => "UNKNOWN",
    }
}

fn main() {
    let conns = anyos_std::net::tcp_list();

    if conns.is_empty() {
        anyos_std::println!("No active TCP connections.");
        return;
    }

    anyos_std::println!("Proto  Local Address            Foreign Address          State        TID  RecvQ");
    anyos_std::println!("-----  -----------------------  -----------------------  -----------  ---  -----");

    for c in &conns {
        let local = alloc::format!(
            "{}.{}.{}.{}:{}",
            c.local_ip[0], c.local_ip[1], c.local_ip[2], c.local_ip[3],
            c.local_port
        );
        let remote = if c.state == 8 {
            // LISTEN — no remote
            alloc::format!("*:*")
        } else {
            alloc::format!(
                "{}.{}.{}.{}:{}",
                c.remote_ip[0], c.remote_ip[1], c.remote_ip[2], c.remote_ip[3],
                c.remote_port
            )
        };

        anyos_std::println!(
            "tcp    {:<23}  {:<23}  {:<11}  {:>3}  {:>5}",
            local, remote, state_name(c.state), c.owner_tid, c.recv_buf_len
        );
    }

    anyos_std::println!("\n{} connections", conns.len());
}
