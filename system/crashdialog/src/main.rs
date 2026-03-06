#![no_std]
#![no_main]

use anyos_std::format;
use anyos_std::println;
use libanyui_client as anyui;

anyos_std::entry!(main);

const DIALOG_W: u32 = 420;
const DIALOG_H: u32 = 180;

fn signal_name(sig: u32) -> &'static str {
    match sig {
        132 => "SIGILL (Invalid opcode)",
        135 => "SIGBUS (Device not available)",
        136 => "SIGFPE (Floating-point exception)",
        139 => "SIGSEGV (Segmentation fault)",
        _ => "Unknown signal",
    }
}

/// Parse a hex string (with optional 0x prefix) into u64.
fn parse_hex(s: &str) -> u64 {
    let s = s.strip_prefix("0x").unwrap_or(s);
    let mut val: u64 = 0;
    for b in s.bytes() {
        let digit = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            _ => break,
        };
        val = val.wrapping_shl(4) | digit as u64;
    }
    val
}

/// Parse a decimal string into u32.
fn parse_u32(s: &str) -> u32 {
    let mut val: u32 = 0;
    for b in s.bytes() {
        match b {
            b'0'..=b'9' => val = val.wrapping_mul(10).wrapping_add((b - b'0') as u32),
            _ => break,
        }
    }
    val
}

fn main() {
    // Parse args: <tid> <signal> <rip_hex> <thread_name>
    let mut arg_buf = [0u8; 256];
    let args = anyos_std::process::args(&mut arg_buf);

    let mut parts = args.splitn(4, ' ');
    let tid_str = match parts.next() {
        Some(s) if !s.is_empty() => s,
        _ => {
            println!("crashdialog: usage: crashdialog <tid> <signal> <rip_hex> <thread_name>");
            return;
        }
    };
    let sig_str = parts.next().unwrap_or("0");
    let rip_str = parts.next().unwrap_or("0");
    let thread_name = parts.next().unwrap_or("unknown");

    let tid = parse_u32(tid_str);
    let signal = parse_u32(sig_str);
    let rip = parse_hex(rip_str);

    // Initialize anyui
    if !anyui::init() {
        println!("crashdialog: failed to init anyui");
        return;
    }

    // Center dialog on screen
    let (screen_w, screen_h) = anyui::screen_size();
    let wx = (screen_w.saturating_sub(DIALOG_W) / 2) as i32;
    let wy = (screen_h.saturating_sub(DIALOG_H) / 2) as i32;

    let win = anyui::Window::new("Crash", wx, wy, DIALOG_W, DIALOG_H);

    // Main layout: vertical stack with padding
    let panel = anyui::StackPanel::vertical();
    panel.set_dock(anyui::DOCK_FILL);
    panel.set_margin(20, 16, 20, 16);
    win.add(&panel);

    // Title: "Application Crashed"
    let title = anyui::Label::new("Application Crashed");
    title.set_font(1); // bold
    title.set_font_size(15);
    title.set_color(0xFFFF3B30); // red
    panel.add(&title);

    // Thread info
    let info_text = format!("{} (TID {})", thread_name, tid);
    let info = anyui::Label::new(&info_text);
    info.set_font_size(13);
    info.set_margin(0, 8, 0, 0);
    panel.add(&info);

    // Signal name
    let sig_label = anyui::Label::new(signal_name(signal));
    sig_label.set_font_size(13);
    sig_label.set_color(0xFF969696);
    sig_label.set_margin(0, 4, 0, 0);
    panel.add(&sig_label);

    // RIP address
    let rip_text = format!("at {:#018x}", rip);
    let rip_label = anyui::Label::new(&rip_text);
    rip_label.set_font_size(11);
    rip_label.set_color(0xFF969696);
    rip_label.set_margin(0, 4, 0, 0);
    panel.add(&rip_label);

    // OK button
    let btn_row = anyui::StackPanel::horizontal();
    btn_row.set_margin(0, 16, 0, 0);
    panel.add(&btn_row);

    let ok_btn = anyui::Button::new("OK");
    ok_btn.set_size(80, 28);
    ok_btn.on_click(|_| {
        anyui::quit();
    });
    btn_row.add(&ok_btn);

    win.on_close(|_| {
        anyui::quit();
    });

    anyui::run();
}
