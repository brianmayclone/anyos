#![no_std]
#![no_main]

anyos_std::entry!(main);

use anyos_std::net;
use anyos_std::println;
use anyos_std::process;

fn main() {
    let mut args_buf = [0u8; 256];
    let args = process::args(&mut args_buf);

    let port: u16 = if args.is_empty() {
        8080
    } else {
        parse_u16(args.trim()).unwrap_or(8080)
    };

    println!("echoserver: listening on port {}...", port);

    let listener = net::tcp_listen(port, 5);
    if listener == u32::MAX {
        println!("echoserver: failed to listen on port {}", port);
        return;
    }

    loop {
        println!("echoserver: waiting for connection...");
        let (sock, ip, rport) = net::tcp_accept(listener);
        if sock == u32::MAX {
            println!("echoserver: accept timeout, retrying...");
            continue;
        }

        println!("echoserver: accepted connection from {}.{}.{}.{}:{} (socket {})",
            ip[0], ip[1], ip[2], ip[3], rport, sock);

        // Echo loop
        let mut buf = [0u8; 2048];
        loop {
            let n = net::tcp_recv(sock, &mut buf);
            if n == 0 {
                println!("echoserver: client disconnected");
                break;
            }
            if n == u32::MAX {
                println!("echoserver: recv error/timeout");
                break;
            }
            // Echo back
            let sent = net::tcp_send(sock, &buf[..n as usize]);
            if sent == u32::MAX {
                println!("echoserver: send error");
                break;
            }
        }

        net::tcp_close(sock);
        println!("echoserver: connection closed, waiting for next...");
    }
}

fn parse_u16(s: &str) -> Option<u16> {
    let mut val: u32 = 0;
    for b in s.bytes() {
        if b < b'0' || b > b'9' { return None; }
        val = val * 10 + (b - b'0') as u32;
        if val > 65535 { return None; }
    }
    Some(val as u16)
}
