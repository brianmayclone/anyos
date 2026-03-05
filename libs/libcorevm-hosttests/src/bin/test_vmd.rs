use std::env;
use std::collections::VecDeque;
use std::fs;
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use libcorevm::{
    corevm_create_ex, corevm_destroy, corevm_get_instruction_count, corevm_get_last_error,
    corevm_get_gpr, corevm_get_last_error_rip, corevm_get_mode, corevm_get_rflags, corevm_get_rip,
    corevm_get_segment_selector, corevm_pic_diag_state, corevm_read_linear_u8, corevm_read_phys_u8,
    corevm_ide_attach_slave, corevm_load_rom, corevm_ps2_key_press, corevm_ps2_key_release,
    corevm_pic_raise_irq, corevm_pit_advance, corevm_run, corevm_serial_take_output,
    corevm_setup_ide, corevm_setup_pci_bus, corevm_setup_standard_devices, corevm_debug_take_output,
    corevm_vga_get_framebuffer, corevm_vga_get_text_buffer,
};

struct VmHandle(u64);

impl Drop for VmHandle {
    fn drop(&mut self) {
        if self.0 != 0 {
            corevm_destroy(self.0);
        }
    }
}

static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);
const SIGINT: i32 = 2;

unsafe extern "C" {
    fn signal(sig: i32, handler: usize) -> usize;
}

extern "C" fn on_sigint(_sig: i32) {
    STOP_REQUESTED.store(true, Ordering::SeqCst);
}

#[derive(Clone, Debug)]
struct Config {
    bios: PathBuf,
    bios_base: u64,
    iso: PathBuf,
    ram_mb: u32,
    cores: u32,
    batch: u64,
    max_seconds: u64,
    max_instructions: u64,
    stdin_keyboard: bool,
    show_vga_text: bool,
    plain: bool,
    auto_enter_ms: u64,
}

struct SttyGuard {
    saved_state: Option<String>,
}

impl SttyGuard {
    fn enable_raw() -> Self {
        let saved_state = Command::new("stty")
            .arg("-g")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());

        if saved_state.is_some() {
            let _ = Command::new("stty").args(["raw", "-echo"]).status();
        }

        Self { saved_state }
    }
}

impl Drop for SttyGuard {
    fn drop(&mut self) {
        if let Some(state) = &self.saved_state {
            let _ = Command::new("stty").arg(state).status();
        }
    }
}

fn default_bios_path() -> PathBuf {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("libcorevm")
        .join("bios");
    let bin = base.join("bios.bin");
    if bin.exists() {
        bin
    } else {
        base.join("bios")
    }
}

fn parse_u64(s: &str) -> Option<u64> {
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16).ok()
    } else {
        s.parse::<u64>().ok()
    }
}

fn parse_args() -> Result<Config, String> {
    let mut cfg = Config {
        bios: default_bios_path(),
        bios_base: 0xF0000,
        iso: PathBuf::new(),
        ram_mb: 256,
        cores: 1,
        batch: 100_000,
        max_seconds: 120,
        max_instructions: 0,
        stdin_keyboard: false,
        show_vga_text: true,
        plain: false,
        auto_enter_ms: 0,
    };

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--bios" => cfg.bios = PathBuf::from(args.next().ok_or("missing value for --bios")?),
            "--bios-base" => {
                let val = args.next().ok_or("missing value for --bios-base")?;
                cfg.bios_base = parse_u64(&val).ok_or("invalid --bios-base")?;
            }
            "--iso" => cfg.iso = PathBuf::from(args.next().ok_or("missing value for --iso")?),
            "--ram-mb" => {
                cfg.ram_mb = args
                    .next()
                    .ok_or("missing value for --ram-mb")?
                    .parse::<u32>()
                    .map_err(|_| "invalid --ram-mb")?;
            }
            "--cores" => {
                cfg.cores = args
                    .next()
                    .ok_or("missing value for --cores")?
                    .parse::<u32>()
                    .map_err(|_| "invalid --cores")?
                    .clamp(1, 64);
            }
            "--batch" => {
                cfg.batch = args
                    .next()
                    .ok_or("missing value for --batch")?
                    .parse::<u64>()
                    .map_err(|_| "invalid --batch")?
                    .max(1);
            }
            "--max-seconds" => {
                cfg.max_seconds = args
                    .next()
                    .ok_or("missing value for --max-seconds")?
                    .parse::<u64>()
                    .map_err(|_| "invalid --max-seconds")?
                    .max(1);
            }
            "--max-instructions" => {
                cfg.max_instructions = args
                    .next()
                    .ok_or("missing value for --max-instructions")?
                    .parse::<u64>()
                    .map_err(|_| "invalid --max-instructions")?;
            }
            "--stdin-kbd" => cfg.stdin_keyboard = true,
            "--no-vga-text" => cfg.show_vga_text = false,
            "--plain" => cfg.plain = true,
            "--auto-enter-ms" => {
                cfg.auto_enter_ms = args
                    .next()
                    .ok_or("missing value for --auto-enter-ms")?
                    .parse::<u64>()
                    .map_err(|_| "invalid --auto-enter-ms")?;
            }
            "--help" | "-h" => {
                return Err(String::new());
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    if cfg.iso.as_os_str().is_empty() {
        return Err("missing required --iso <path>".to_string());
    }
    Ok(cfg)
}

fn usage(program: &str) {
    eprintln!(
        "Usage: {program} --iso <path> [--bios <path>] [--bios-base <addr>] [--ram-mb <mb>] [--cores <n>] [--batch <n>] [--max-seconds <n>] [--max-instructions <n>] [--stdin-kbd] [--no-vga-text] [--plain] [--auto-enter-ms <ms>]"
    );
}

fn take_text_output(handle: u64) -> String {
    let mut out = String::new();
    let mut buf = [0u8; 4096];

    loop {
        let n = corevm_debug_take_output(handle, buf.as_mut_ptr(), buf.len() as u32);
        if n == 0 {
            break;
        }
        out.push_str(&String::from_utf8_lossy(&buf[..n as usize]));
    }
    loop {
        let n = corevm_serial_take_output(handle, buf.as_mut_ptr(), buf.len() as u32);
        if n == 0 {
            break;
        }
        out.push_str(&String::from_utf8_lossy(&buf[..n as usize]));
    }
    out
}

fn mode_name(mode: u32) -> &'static str {
    match mode {
        0 => "RealMode",
        1 => "ProtectedMode",
        2 => "LongMode",
        _ => "Unknown",
    }
}

fn dump_cpu_probe(handle: u64, mode: u32, cs: u16, rip: u64) {
    let rflags = corevm_get_rflags(handle);
    let pic = corevm_pic_diag_state(handle);
    let m_irr = (pic & 0xFF) as u8;
    let m_isr = ((pic >> 8) & 0xFF) as u8;
    let m_imr = ((pic >> 16) & 0xFF) as u8;
    let s_irr = ((pic >> 24) & 0xFF) as u8;
    let s_isr = ((pic >> 32) & 0xFF) as u8;
    let s_imr = ((pic >> 40) & 0xFF) as u8;
    let ax = corevm_get_gpr(handle, 0) as u32;
    let cx = corevm_get_gpr(handle, 1) as u32;
    let dx = corevm_get_gpr(handle, 2) as u32;
    let bx = corevm_get_gpr(handle, 3) as u32;
    let sp = corevm_get_gpr(handle, 4) as u32;
    let bp = corevm_get_gpr(handle, 5) as u32;
    let si = corevm_get_gpr(handle, 6) as u32;
    let di = corevm_get_gpr(handle, 7) as u32;
    let ip16 = (rip as u16) as u64;
    let probe_addr = if mode == 0 {
        ((cs as u64) << 4).wrapping_add(ip16)
    } else {
        rip
    };
    let mut bytes = [0u8; 8];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = if mode == 0 {
            corevm_read_phys_u8(handle, probe_addr.wrapping_add(i as u64))
        } else {
            corevm_read_linear_u8(handle, probe_addr.wrapping_add(i as u64))
        };
    }
    eprintln!(
        "[test-vmd] cpu probe: mode={} cs:ip={:04X}:{:04X} addr={:08X} AX={:04X} BX={:04X} CX={:04X} DX={:04X} SI={:04X} DI={:04X} BP={:04X} SP={:04X} FLAGS={:04X} IF={} ZF={} CF={} PIC[m:{:02X}/{:02X}/{:02X} s:{:02X}/{:02X}/{:02X}] bytes={:02X} {:02X} {:02X} {:02X} {:02X} {:02X} {:02X} {:02X}",
        mode_name(mode),
        cs,
        ip16 as u32,
        probe_addr as u32,
        ax as u16,
        bx as u16,
        cx as u16,
        dx as u16,
        si as u16,
        di as u16,
        bp as u16,
        sp as u16,
        (rflags & 0xFFFF) as u16,
        if (rflags & 0x0200) != 0 { 1 } else { 0 },
        if (rflags & 0x0040) != 0 { 1 } else { 0 },
        if (rflags & 0x0001) != 0 { 1 } else { 0 },
        m_irr,
        m_isr,
        m_imr,
        s_irr,
        s_isr,
        s_imr,
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7]
    );
}

fn last_error(handle: u64) -> String {
    let mut buf = [0u8; 1024];
    let n = corevm_get_last_error(handle, buf.as_mut_ptr(), buf.len() as u32);
    if n == 0 {
        String::new()
    } else {
        String::from_utf8_lossy(&buf[..n as usize]).to_string()
    }
}

fn ensure_exists(path: &Path, what: &str) -> Result<(), String> {
    if path.exists() {
        Ok(())
    } else {
        Err(format!("{what} not found: {}", path.display()))
    }
}

const VGA_TEXT_ROWS: usize = 25;
const VGA_TEXT_COLS: usize = 80;
const UI_LOG_TAIL: usize = 16;
const FB_RAMP: &[u8] = b" .:-=+*#%@";
const PIT_TICKS_PER_MS: u32 = 1193;
const PIT_MAX_ADVANCE_MS: u64 = 10;

#[derive(Default)]
struct DisplayState {
    text_cells: Vec<u16>,
    fb_bytes: Vec<u8>,
    fb_width: u32,
    fb_height: u32,
    fb_bpp: u8,
    in_text_mode: bool,
}

fn display_signature(state: &DisplayState) -> u64 {
    if state.in_text_mode {
        let mut sig = 0u64;
        let n = state.text_cells.len().min(64);
        for i in 0..n {
            sig = sig.wrapping_mul(131).wrapping_add(state.text_cells[i] as u64);
        }
        sig ^ 0x54585400u64
    } else {
        let mut sig = 0u64;
        let n = state.fb_bytes.len().min(256);
        for i in 0..n {
            sig = sig.wrapping_mul(131).wrapping_add(state.fb_bytes[i] as u64);
        }
        sig ^ ((state.fb_width as u64) << 32) ^ ((state.fb_height as u64) << 8) ^ state.fb_bpp as u64
    }
}

fn dump_text_screen(cells: &[u16]) {
    let cols = 80usize;
    let rows = 25usize;
    eprintln!("[test-vmd] final text screen dump:");
    for r in 0..rows {
        let mut line = String::with_capacity(cols);
        for c in 0..cols {
            let idx = r * cols + c;
            let ch = if idx < cells.len() {
                (cells[idx] & 0xFF) as u8
            } else {
                b' '
            };
            let out = if ch.is_ascii_graphic() || ch == b' ' {
                ch as char
            } else {
                '.'
            };
            line.push(out);
        }
        eprintln!("{:02}: {}", r, line);
    }
}

fn update_display_state(handle: u64, state: &mut DisplayState) {
    let mut count: u32 = 0;
    let text_ptr = corevm_vga_get_text_buffer(handle, &mut count as *mut u32);
    if !text_ptr.is_null() && count > 0 {
        state.in_text_mode = true;
        let cells = unsafe { std::slice::from_raw_parts(text_ptr, count as usize) };
        if state.text_cells.as_slice() != cells {
            state.text_cells.clear();
            state.text_cells.extend_from_slice(cells);
        }
        return;
    }

    let mut width = 0u32;
    let mut height = 0u32;
    let mut bpp = 0u8;
    let fb_ptr = corevm_vga_get_framebuffer(
        handle,
        &mut width as *mut u32,
        &mut height as *mut u32,
        &mut bpp as *mut u8,
    );
    state.in_text_mode = false;
    state.fb_width = width;
    state.fb_height = height;
    state.fb_bpp = bpp;
    if fb_ptr.is_null() || width == 0 || height == 0 {
        state.fb_bytes.clear();
        return;
    }

    let bytes_per_pixel = (bpp as usize).max(8).div_ceil(8);
    let total = (width as usize)
        .saturating_mul(height as usize)
        .saturating_mul(bytes_per_pixel);
    if total == 0 {
        state.fb_bytes.clear();
        return;
    }
    let fb = unsafe { std::slice::from_raw_parts(fb_ptr, total) };
    state.fb_bytes.clear();
    state.fb_bytes.extend_from_slice(fb);
}

fn append_log_text(log_lines: &mut VecDeque<String>, pending: &mut String, text: &str) {
    pending.push_str(text);
    loop {
        let Some(pos) = pending.find('\n') else {
            break;
        };
        let mut line = pending[..pos].to_string();
        if line.ends_with('\r') {
            line.pop();
        }
        log_lines.push_back(line);
        if log_lines.len() > 5000 {
            log_lines.pop_front();
        }
        pending.drain(..=pos);
    }
}

fn fb_intensity(state: &DisplayState, x: usize, y: usize) -> u8 {
    if state.fb_width == 0 || state.fb_height == 0 || state.fb_bpp == 0 {
        return 0;
    }
    let width = state.fb_width as usize;
    let bpp = state.fb_bpp as usize;
    let bytes_per_pixel = bpp.max(8).div_ceil(8);
    let idx = (y * width + x).saturating_mul(bytes_per_pixel);
    if idx >= state.fb_bytes.len() {
        return 0;
    }
    match bytes_per_pixel {
        1 => state.fb_bytes[idx],
        2 => {
            if idx + 1 >= state.fb_bytes.len() {
                return 0;
            }
            let pix = u16::from_le_bytes([state.fb_bytes[idx], state.fb_bytes[idx + 1]]);
            let r = ((pix >> 11) & 0x1F) as u32 * 255 / 31;
            let g = ((pix >> 5) & 0x3F) as u32 * 255 / 63;
            let b = (pix & 0x1F) as u32 * 255 / 31;
            ((r * 30 + g * 59 + b * 11) / 100) as u8
        }
        3 | 4 => {
            if idx + 2 >= state.fb_bytes.len() {
                return 0;
            }
            let b = state.fb_bytes[idx] as u32;
            let g = state.fb_bytes[idx + 1] as u32;
            let r = state.fb_bytes[idx + 2] as u32;
            ((r * 30 + g * 59 + b * 11) / 100) as u8
        }
        _ => state.fb_bytes[idx],
    }
}

fn vga_lines(state: &DisplayState, allow_text: bool) -> Vec<String> {
    if allow_text && state.in_text_mode {
        let mut out = Vec::with_capacity(VGA_TEXT_ROWS);
        for row in 0..VGA_TEXT_ROWS {
            let mut line = String::with_capacity(VGA_TEXT_COLS);
            for col in 0..VGA_TEXT_COLS {
                let idx = row * VGA_TEXT_COLS + col;
                if idx >= state.text_cells.len() {
                    line.push(' ');
                    continue;
                }
                let ch = (state.text_cells[idx] & 0x00FF) as u8;
                if ch.is_ascii_graphic() || ch == b' ' {
                    line.push(ch as char);
                } else {
                    line.push(' ');
                }
            }
            out.push(line);
        }
        return out;
    }

    let mut out = Vec::with_capacity(VGA_TEXT_ROWS);
    if state.fb_width == 0 || state.fb_height == 0 || state.fb_bytes.is_empty() {
        out.push("(no VGA output yet)".to_string());
        while out.len() < VGA_TEXT_ROWS {
            out.push(String::new());
        }
        return out;
    }
    let w = state.fb_width as usize;
    let h = state.fb_height as usize;
    for row in 0..VGA_TEXT_ROWS {
        let mut line = String::with_capacity(VGA_TEXT_COLS);
        for col in 0..VGA_TEXT_COLS {
            let sx = ((col * w) / VGA_TEXT_COLS).min(w.saturating_sub(1));
            let sy = ((row * h) / VGA_TEXT_ROWS).min(h.saturating_sub(1));
            let i = fb_intensity(state, sx, sy) as usize;
            let ridx = (i * (FB_RAMP.len() - 1)) / 255;
            line.push(FB_RAMP[ridx] as char);
        }
        out.push(line);
    }
    out
}

fn trim_to_width(mut s: String, width: usize) -> String {
    if s.len() > width {
        s.truncate(width);
        return s;
    }
    if s.len() < width {
        s.push_str(&" ".repeat(width - s.len()));
    }
    s
}

fn build_ui_lines(
    cfg: &Config,
    vm_handle: u64,
    start: Instant,
    display: &DisplayState,
    log_lines: &VecDeque<String>,
) -> Vec<String> {
    let ic = corevm_get_instruction_count(vm_handle);
    let mode = corevm_get_mode(vm_handle);
    let cs = corevm_get_segment_selector(vm_handle, 1);
    let rip = corevm_get_rip(vm_handle);

    let mut lines = Vec::new();
    lines.push(format!(
        "test_vmd | t={}s ic={} mode={} cs={:04X} rip={:08X} | Ctrl+C beendet",
        start.elapsed().as_secs(),
        ic,
        mode_name(mode),
        cs,
        rip as u32
    ));
    lines.push(format!(
        "ISO={} RAM={}MiB Cores={} Batch={} stdin_kbd={} text_pref={} render={}",
        cfg.iso.display(),
        cfg.ram_mb,
        cfg.cores,
        cfg.batch,
        cfg.stdin_keyboard,
        cfg.show_vga_text,
        if display.in_text_mode {
            "VGA text 80x25"
        } else if display.fb_width > 0 && display.fb_height > 0 {
            "VGA framebuffer"
        } else {
            "none"
        }
    ));
    lines.push("-".repeat(VGA_TEXT_COLS));
    lines.extend(vga_lines(display, cfg.show_vga_text));
    lines.push("-".repeat(VGA_TEXT_COLS));
    lines.push("Debug/Serial output (letzte Zeilen):".to_string());
    let start_idx = log_lines.len().saturating_sub(UI_LOG_TAIL);
    for line in log_lines.iter().skip(start_idx) {
        lines.push(line.clone());
    }
    while lines.len() < (3 + VGA_TEXT_ROWS + 2 + UI_LOG_TAIL) {
        lines.push(String::new());
    }
    lines
}

fn render_ui(lines: &[String], prev_lines: &mut Vec<String>) {
    let mut out = String::new();
    let width = VGA_TEXT_COLS;
    for (i, line) in lines.iter().enumerate() {
        let line = trim_to_width(line.clone(), width);
        if prev_lines.get(i) != Some(&line) {
            out.push_str(&format!("\x1B[{};1H{}", i + 1, line));
            if i >= prev_lines.len() {
                prev_lines.push(line);
            } else {
                prev_lines[i] = line;
            }
        }
    }
    if !out.is_empty() {
        let _ = io::stdout().write_all(out.as_bytes());
        let _ = io::stdout().flush();
    }
}

fn advance_pit_realtime(vm_handle: u64, last_pit_tick: &mut Instant) {
    let now = Instant::now();
    let elapsed_ms = now
        .duration_since(*last_pit_tick)
        .as_millis()
        .min(PIT_MAX_ADVANCE_MS as u128) as u64;
    let ticks = if elapsed_ms > 0 {
        (elapsed_ms as u32) * PIT_TICKS_PER_MS
    } else {
        PIT_TICKS_PER_MS
    };
    let fires = corevm_pit_advance(vm_handle, ticks);
    if fires > 0 {
        corevm_pic_raise_irq(vm_handle, 0);
    }
    *last_pit_tick = now;
}

fn scancode_for_ascii(ch: u8) -> Option<(bool, u8)> {
    let lower = ch.to_ascii_lowercase();
    let shift = ch.is_ascii_uppercase();
    let code = match lower {
        b'1' => 0x02,
        b'2' => 0x03,
        b'3' => 0x04,
        b'4' => 0x05,
        b'5' => 0x06,
        b'6' => 0x07,
        b'7' => 0x08,
        b'8' => 0x09,
        b'9' => 0x0A,
        b'0' => 0x0B,
        b'q' => 0x10,
        b'w' => 0x11,
        b'e' => 0x12,
        b'r' => 0x13,
        b't' => 0x14,
        b'y' => 0x15,
        b'u' => 0x16,
        b'i' => 0x17,
        b'o' => 0x18,
        b'p' => 0x19,
        b'a' => 0x1E,
        b's' => 0x1F,
        b'd' => 0x20,
        b'f' => 0x21,
        b'g' => 0x22,
        b'h' => 0x23,
        b'j' => 0x24,
        b'k' => 0x25,
        b'l' => 0x26,
        b'z' => 0x2C,
        b'x' => 0x2D,
        b'c' => 0x2E,
        b'v' => 0x2F,
        b'b' => 0x30,
        b'n' => 0x31,
        b'm' => 0x32,
        b' ' => 0x39,
        b'\n' | b'\r' => 0x1C,
        b'\t' => 0x0F,
        0x08 | 0x7F => 0x0E,
        _ => return None,
    };
    Some((shift, code))
}

fn inject_ascii_key(handle: u64, ch: u8) {
    if let Some((shift, code)) = scancode_for_ascii(ch) {
        if shift {
            corevm_ps2_key_press(handle, 0x2A);
        }
        corevm_ps2_key_press(handle, code);
        corevm_ps2_key_release(handle, code);
        if shift {
            corevm_ps2_key_release(handle, 0x2A);
        }
    }
}

fn main() {
    let program = env::args()
        .next()
        .unwrap_or_else(|| "test_vmd".to_string());
    let mut cfg = match parse_args() {
        Ok(c) => c,
        Err(e) => {
            usage(&program);
            if !e.is_empty() {
                eprintln!("error: {e}");
            }
            std::process::exit(2);
        }
    };
    if cfg.plain && !cfg.stdin_keyboard && cfg.auto_enter_ms == 0 {
        cfg.auto_enter_ms = 1500;
    }

    if let Err(e) = ensure_exists(&cfg.bios, "bios") {
        eprintln!("{e}");
        std::process::exit(2);
    }
    if let Err(e) = ensure_exists(&cfg.iso, "iso") {
        eprintln!("{e}");
        std::process::exit(2);
    }

    let bios = match fs::read(&cfg.bios) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("failed to read bios {}: {e}", cfg.bios.display());
            std::process::exit(2);
        }
    };
    let iso = match fs::read(&cfg.iso) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("failed to read iso {}: {e}", cfg.iso.display());
            std::process::exit(2);
        }
    };

    eprintln!(
        "[test-vmd] bios={} ({} bytes) iso={} ({} bytes) ram={}MiB cores={} stdin_kbd={} vga_text={}",
        cfg.bios.display(),
        bios.len(),
        cfg.iso.display(),
        iso.len(),
        cfg.ram_mb,
        cfg.cores,
        cfg.stdin_keyboard,
        cfg.show_vga_text
    );

    let vm = VmHandle(corevm_create_ex(cfg.ram_mb, cfg.cores));
    if vm.0 == 0 {
        eprintln!("corevm_create_ex failed");
        std::process::exit(1);
    }

    let rc = corevm_load_rom(vm.0, cfg.bios_base, bios.as_ptr(), bios.len() as u32);
    if rc != 0 {
        eprintln!("corevm_load_rom failed (rc={rc})");
        std::process::exit(1);
    }

    corevm_setup_standard_devices(vm.0);
    corevm_setup_pci_bus(vm.0);
    corevm_setup_ide(vm.0);
    corevm_ide_attach_slave(vm.0, iso.as_ptr(), iso.len() as u32);

    let (kbd_tx, kbd_rx) = mpsc::channel::<u8>();
    if cfg.stdin_keyboard {
        thread::spawn(move || {
            let mut stdin = io::stdin().lock();
            let mut buf = [0u8; 1];
            loop {
                match stdin.read(&mut buf) {
                    Ok(0) => break,
                    Ok(_) => {
                        if kbd_tx.send(buf[0]).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
    }

    let start = Instant::now();
    let max_duration = Duration::from_secs(cfg.max_seconds);
    let interactive_ui = !cfg.plain && io::stdout().is_terminal();
    let mut display = DisplayState::default();
    let mut last_display_meta = String::new();
    let mut last_display_sig = 0u64;
    let mut last_plain_diag = Instant::now();
    let mut log_lines: VecDeque<String> = VecDeque::new();
    let mut log_pending = String::new();
    let mut last_render = Instant::now();
    let mut prev_ui_lines: Vec<String> = Vec::new();
    let mut last_pit_tick = Instant::now();
    let mut last_auto_enter = Instant::now();
    let mut saw_booting_kernel = false;
    let _raw_guard = if cfg.stdin_keyboard {
        Some(SttyGuard::enable_raw())
    } else {
        None
    };

    unsafe {
        signal(SIGINT, on_sigint as *const () as usize);
    }
    if interactive_ui {
        print!("\x1B[?1049h\x1B[?25l\x1B[2J\x1B[H");
        let _ = io::stdout().flush();
    }

    loop {
        if STOP_REQUESTED.load(Ordering::SeqCst) {
            break;
        }

        while let Ok(ch) = kbd_rx.try_recv() {
            inject_ascii_key(vm.0, ch);
        }

        advance_pit_realtime(vm.0, &mut last_pit_tick);
        let exit_code = corevm_run(vm.0, cfg.batch);

        let text = take_text_output(vm.0);
        if !text.is_empty() {
            append_log_text(&mut log_lines, &mut log_pending, &text);
            if text.contains("Booting the kernel") {
                saw_booting_kernel = true;
            }
            if !interactive_ui {
                print!("{text}");
                let _ = io::stdout().flush();
            }
        }

        if cfg.auto_enter_ms > 0
            && !saw_booting_kernel
            && last_auto_enter.elapsed() >= Duration::from_millis(cfg.auto_enter_ms)
        {
            inject_ascii_key(vm.0, b'\n');
            last_auto_enter = Instant::now();
        }
        update_display_state(vm.0, &mut display);
        if !interactive_ui {
            let meta = if display.in_text_mode {
                format!("text cells={}", display.text_cells.len())
            } else {
                format!(
                    "gfx {}x{}x{} bytes={}",
                    display.fb_width, display.fb_height, display.fb_bpp, display.fb_bytes.len()
                )
            };
            let sig = display_signature(&display);
            if meta != last_display_meta || sig != last_display_sig {
                eprintln!("[test-vmd] display: {meta} sig=0x{sig:016X}");
                last_display_meta = meta;
                last_display_sig = sig;
            } else if last_plain_diag.elapsed() >= Duration::from_secs(5) {
                let mode = corevm_get_mode(vm.0);
                let cs = corevm_get_segment_selector(vm.0, 1);
                let rip = corevm_get_rip(vm.0);
                eprintln!(
                    "[test-vmd] display steady: {} sig=0x{:016X} ic={} mode={} cs={:04X} rip={:08X}",
                    last_display_meta,
                    last_display_sig,
                    corevm_get_instruction_count(vm.0),
                    mode_name(mode),
                    cs,
                    rip as u32
                );
                dump_cpu_probe(vm.0, mode, cs, rip);
                last_plain_diag = Instant::now();
            }
        }

        if interactive_ui && last_render.elapsed() >= Duration::from_millis(100) {
            let lines = build_ui_lines(&cfg, vm.0, start, &display, &log_lines);
            render_ui(&lines, &mut prev_ui_lines);
            last_render = Instant::now();
        }

        if cfg.max_instructions > 0 && corevm_get_instruction_count(vm.0) >= cfg.max_instructions {
            eprintln!("[test-vmd] reached max instructions {}", cfg.max_instructions);
            break;
        }

        match exit_code {
            0 => {
                thread::sleep(Duration::from_micros(500));
                advance_pit_realtime(vm.0, &mut last_pit_tick);
            }
            1 => {
                let err = last_error(vm.0);
                let rip = corevm_get_last_error_rip(vm.0);
                eprintln!("[test-vmd] exception at rip=0x{rip:X}: {err}");
                std::process::exit(1);
            }
            2 => {}
            3 => {
                eprintln!("[test-vmd] breakpoint exit");
                break;
            }
            4 => {
                eprintln!("[test-vmd] stop-requested exit");
                break;
            }
            _ => {
                eprintln!("[test-vmd] unexpected exit code {exit_code}");
                break;
            }
        }

        if start.elapsed() >= max_duration {
            eprintln!("[test-vmd] timeout after {} seconds", cfg.max_seconds);
            break;
        }
    }

    if saw_booting_kernel {
        eprintln!("[test-vmd] marker reached: Booting the kernel");
    }
    if !interactive_ui && display.in_text_mode {
        dump_text_screen(&display.text_cells);
    }

    // Restore terminal state on exit.
    if interactive_ui {
        println!("\x1B[?25h\x1B[?1049l");
    }
}
