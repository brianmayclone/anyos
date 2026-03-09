use std::env;
use std::collections::VecDeque;
use std::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void, CString};
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
    corevm_get_cr, corevm_get_gpr, corevm_get_last_error_rip, corevm_get_mode, corevm_get_msr, corevm_get_rflags,
    corevm_get_rip, corevm_get_segment_base, corevm_get_segment_selector, corevm_jit_cache_stats,
    corevm_cache_stats,
    corevm_jit_enable, corevm_jit_stats, corevm_lapic_diag_state, corevm_irq_pending_word,
    corevm_ioapic_redir_entry,
    corevm_pic_diag_state, corevm_read_linear_u8, corevm_read_phys_u8, corevm_read_phys_u32,
    corevm_fw_cfg_add_file, corevm_ide_attach_disk, corevm_ide_attach_slave, corevm_ide_clear_irq, corevm_ide_irq_raised,
    corevm_load_binary, corevm_load_rom, corevm_pic_raise_irq,
    corevm_ps2_key_press, corevm_ps2_key_release, corevm_set_rip,
    corevm_reset, corevm_run, corevm_serial_take_output,
    corevm_setup_ide, corevm_setup_pci_bus, corevm_setup_standard_devices, corevm_debug_take_output,
    corevm_vga_get_framebuffer, corevm_vga_get_text_buffer,
    corevm_debugger_enable, corevm_debugger_break,
    corevm_dump_exception_ring,
};

// ── X11 FFI bindings (conditional display) ──

const X11_KEY_PRESS: c_int = 2;
const X11_KEY_RELEASE: c_int = 3;
const X11_CONFIGURE_NOTIFY: c_int = 22;
const X11_KEY_PRESS_MASK: c_long = 1 << 0;
const X11_KEY_RELEASE_MASK: c_long = 1 << 1;
const X11_EXPOSURE_MASK: c_long = 1 << 15;
const X11_STRUCTURE_NOTIFY_MASK: c_long = 1 << 17;
const X11_ZPIXMAP: c_int = 2;
const X11_RENDER_MIN_INTERVAL: Duration = Duration::from_millis(16);
const X11_OVERLAY_UPDATE_INTERVAL: Duration = Duration::from_millis(250);
const X11_CHAR_W: usize = 8;
const X11_CHAR_H: usize = 16;
const X11_OVERLAY_PAD: usize = 8;

const XK_BACKSPACE: c_ulong = 0xFF08;
const XK_TAB: c_ulong = 0xFF09;
const XK_RETURN: c_ulong = 0xFF0D;
const XK_ESCAPE: c_ulong = 0xFF1B;
const XK_LEFT: c_ulong = 0xFF51;
const XK_UP: c_ulong = 0xFF52;
const XK_RIGHT: c_ulong = 0xFF53;
const XK_DOWN: c_ulong = 0xFF54;

#[repr(C)]
struct X11Display(c_void);
#[repr(C)]
struct X11Visual(c_void);
#[repr(C)]
struct X11GCData(c_void);
type X11GC = *mut X11GCData;

#[repr(C)]
struct XEvent {
    pad: [c_long; 24],
}

#[repr(C)]
struct XAnyEvent {
    type_: c_int,
    _serial: c_ulong,
    _send_event: c_int,
    _display: *mut X11Display,
    _window: c_ulong,
}

#[repr(C)]
struct XKeyEvent {
    type_: c_int,
    _serial: c_ulong,
    _send_event: c_int,
    _display: *mut X11Display,
    _window: c_ulong,
    _root: c_ulong,
    _subwindow: c_ulong,
    _time: c_ulong,
    _x: c_int,
    _y: c_int,
    _x_root: c_int,
    _y_root: c_int,
    _state: c_uint,
    _keycode: c_uint,
    _same_screen: c_int,
}

#[repr(C)]
struct XConfigureEvent {
    _type: c_int,
    _serial: c_ulong,
    _send_event: c_int,
    _display: *mut X11Display,
    _event: c_ulong,
    _window: c_ulong,
    _x: c_int,
    _y: c_int,
    width: c_int,
    height: c_int,
    _border_width: c_int,
    _above: c_ulong,
    _override_redirect: c_int,
}

#[repr(C)]
struct XImage {
    width: c_int,
    height: c_int,
    xoffset: c_int,
    format: c_int,
    data: *mut c_char,
    byte_order: c_int,
    bitmap_unit: c_int,
    bitmap_bit_order: c_int,
    bitmap_pad: c_int,
    depth: c_int,
    bytes_per_line: c_int,
    bits_per_pixel: c_int,
    red_mask: c_ulong,
    green_mask: c_ulong,
    blue_mask: c_ulong,
    obdata: *mut c_char,
    f: [usize; 8],
}

#[link(name = "X11")]
unsafe extern "C" {
    fn XOpenDisplay(name: *const c_char) -> *mut X11Display;
    fn XCloseDisplay(display: *mut X11Display) -> c_int;
    fn XDefaultScreen(display: *mut X11Display) -> c_int;
    fn XRootWindow(display: *mut X11Display, screen: c_int) -> c_ulong;
    fn XBlackPixel(display: *mut X11Display, screen: c_int) -> c_ulong;
    fn XWhitePixel(display: *mut X11Display, screen: c_int) -> c_ulong;
    fn XDefaultVisual(display: *mut X11Display, screen: c_int) -> *mut X11Visual;
    fn XDefaultDepth(display: *mut X11Display, screen: c_int) -> c_int;
    fn XCreateSimpleWindow(
        display: *mut X11Display, parent: c_ulong,
        x: c_int, y: c_int, width: c_uint, height: c_uint,
        border_width: c_uint, border: c_ulong, background: c_ulong,
    ) -> c_ulong;
    fn XStoreName(display: *mut X11Display, window: c_ulong, name: *const c_char) -> c_int;
    fn XSelectInput(display: *mut X11Display, window: c_ulong, mask: c_long) -> c_int;
    fn XMapWindow(display: *mut X11Display, window: c_ulong) -> c_int;
    fn XCreateGC(display: *mut X11Display, drawable: c_ulong, v: c_ulong, values: *mut c_void) -> X11GC;
    fn XSetForeground(display: *mut X11Display, gc: X11GC, color: c_ulong) -> c_int;
    fn XFillRectangle(
        display: *mut X11Display, drawable: c_ulong, gc: X11GC,
        x: c_int, y: c_int, width: c_uint, height: c_uint,
    ) -> c_int;
    fn XDrawString(
        display: *mut X11Display, drawable: c_ulong, gc: X11GC,
        x: c_int, y: c_int, text: *const c_char, len: c_int,
    ) -> c_int;
    fn XPending(display: *mut X11Display) -> c_int;
    fn XNextEvent(display: *mut X11Display, event: *mut XEvent) -> c_int;
    fn XLookupKeysym(event: *mut XKeyEvent, index: c_int) -> c_ulong;
    fn XCreateImage(
        display: *mut X11Display, visual: *mut X11Visual, depth: c_uint,
        format: c_int, offset: c_int, data: *mut c_char,
        width: c_uint, height: c_uint, bitmap_pad: c_int, bytes_per_line: c_int,
    ) -> *mut XImage;
    fn XPutImage(
        display: *mut X11Display, drawable: c_ulong, gc: X11GC, image: *mut XImage,
        src_x: c_int, src_y: c_int, dst_x: c_int, dst_y: c_int,
        width: c_uint, height: c_uint,
    ) -> c_int;
    fn XFlush(display: *mut X11Display) -> c_int;
}

struct X11Window {
    display: *mut X11Display,
    window: c_ulong,
    gc: X11GC,
    visual: *mut X11Visual,
    depth: c_int,
    win_w: usize,
    win_h: usize,
    blit_buf: Vec<u32>,
    ximage: *mut XImage,
    last_render: Instant,
    last_overlay_update: Instant,
    last_text_hash: u64,
    diag_lines: Vec<String>,
    last_ic: u64,
    last_calls: u64,
    last_cache_hits: u64,
    last_cache_misses: u64,
    run_calls: u64,
}

impl X11Window {
    fn open() -> Option<Self> {
        let display = unsafe { XOpenDisplay(std::ptr::null()) };
        if display.is_null() {
            return None;
        }
        let screen = unsafe { XDefaultScreen(display) };
        let root = unsafe { XRootWindow(display, screen) };
        let black = unsafe { XBlackPixel(display, screen) };
        let white = unsafe { XWhitePixel(display, screen) };
        let window = unsafe { XCreateSimpleWindow(display, root, 50, 50, 800, 600, 1, white, black) };
        unsafe {
            XSelectInput(display, window,
                X11_KEY_PRESS_MASK | X11_KEY_RELEASE_MASK | X11_EXPOSURE_MASK | X11_STRUCTURE_NOTIFY_MASK);
            let title = b"CoreVM Display\0";
            XStoreName(display, window, title.as_ptr() as *const c_char);
            XMapWindow(display, window);
        }
        let gc = unsafe { XCreateGC(display, window, 0, std::ptr::null_mut()) };
        let visual = unsafe { XDefaultVisual(display, screen) };
        let depth = unsafe { XDefaultDepth(display, screen) };
        let win_w = 800usize;
        let win_h = 600usize;
        let mut blit_buf = vec![0u32; win_w * win_h];
        let ximage = unsafe {
            XCreateImage(display, visual, depth as c_uint, X11_ZPIXMAP, 0,
                blit_buf.as_mut_ptr() as *mut c_char, win_w as c_uint, win_h as c_uint, 32, 0)
        };
        Some(X11Window {
            display, window, gc, visual, depth, win_w, win_h, blit_buf, ximage,
            last_render: Instant::now() - X11_RENDER_MIN_INTERVAL,
            last_overlay_update: Instant::now() - X11_OVERLAY_UPDATE_INTERVAL,
            last_text_hash: 0,
            diag_lines: vec![String::new(); 4],
            last_ic: 0, last_calls: 0,
            last_cache_hits: 0, last_cache_misses: 0,
            run_calls: 0,
        })
    }

    fn process_events(&mut self, vm: u64) {
        while unsafe { XPending(self.display) } > 0 {
            let mut ev = XEvent { pad: [0; 24] };
            unsafe { XNextEvent(self.display, &mut ev) };
            let any = unsafe { &*(&ev as *const XEvent as *const XAnyEvent) };
            if any.type_ == X11_KEY_PRESS || any.type_ == X11_KEY_RELEASE {
                let kev = unsafe { &mut *(&mut ev as *mut XEvent as *mut XKeyEvent) };
                let ks = unsafe { XLookupKeysym(kev, 0) };
                x11_keysym_to_ps2(vm, ks, any.type_ == X11_KEY_PRESS);
            } else if any.type_ == X11_CONFIGURE_NOTIFY {
                let cev = unsafe { &*(&ev as *const XEvent as *const XConfigureEvent) };
                let nw = cev.width.max(1) as usize;
                let nh = cev.height.max(1) as usize;
                if nw != self.win_w || nh != self.win_h {
                    self.win_w = nw;
                    self.win_h = nh;
                    self.blit_buf = vec![0u32; nw * nh];
                    self.ximage = unsafe {
                        XCreateImage(self.display, self.visual, self.depth as c_uint, X11_ZPIXMAP, 0,
                            self.blit_buf.as_mut_ptr() as *mut c_char, nw as c_uint, nh as c_uint, 32, 0)
                    };
                }
            }
        }
    }

    fn update_overlay(&mut self, vm: u64, jit: bool) {
        let now = Instant::now();
        if now.duration_since(self.last_overlay_update) < X11_OVERLAY_UPDATE_INTERVAL {
            return;
        }
        self.run_calls = self.run_calls.saturating_add(1);
        let ic = corevm_get_instruction_count(vm);
        let dt = now.duration_since(self.last_overlay_update).as_secs_f64().max(1e-6);
        let dic = ic.saturating_sub(self.last_ic);
        let dcalls = self.run_calls.saturating_sub(self.last_calls);
        let mode = corevm_get_mode(vm);
        let cs = corevm_get_segment_selector(vm, 1);
        let rip = corevm_get_rip(vm);
        let ipc = if dcalls > 0 { dic as f64 / dcalls as f64 } else { 0.0 };
        let mips = (dic as f64 / dt) / 1_000_000.0;
        self.diag_lines[0] = format!("mode={} cs:ip={:04X}:{:08X}", mode_name(mode), cs, rip as u32);
        self.diag_lines[1] = format!("IPC={:.1} MIPS={:.2} ic={}", ipc, mips, ic);
        let mut ch = 0u64; let mut cm = 0u64; let mut ce = 0u64;
        corevm_cache_stats(vm, &mut ch, &mut cm, &mut ce);
        let dch = ch.saturating_sub(self.last_cache_hits);
        let dcm = cm.saturating_sub(self.last_cache_misses);
        let hit_rate = if dch + dcm > 0 { dch as f64 / (dch + dcm) as f64 * 100.0 } else { 0.0 };
        self.last_cache_hits = ch;
        self.last_cache_misses = cm;
        self.diag_lines[2] = format!("cache: {:.1}% hit ({}/{}) entries={}", hit_rate, dch, dcm, ce);
        let mut jb = 0u64; let mut jn = 0u64; let mut jf = 0u64; let mut jc = 0u32;
        corevm_jit_stats(vm, &mut jb, &mut jn, &mut jf, &mut jc);
        self.diag_lines[3] = if jit { format!("JIT b={} n={} f={}", jb, jn, jf) } else { "JIT off".to_string() };
        self.last_ic = ic;
        self.last_calls = self.run_calls;
        self.last_overlay_update = now;
    }

    fn render(&mut self, vm: u64) {
        let now = Instant::now();
        if now.duration_since(self.last_render) < X11_RENDER_MIN_INTERVAL {
            return;
        }
        let mut text_count = 0u32;
        let text_ptr = corevm_vga_get_text_buffer(vm, &mut text_count);
        if !text_ptr.is_null() && text_count >= 2000 {
            let cells = unsafe { std::slice::from_raw_parts(text_ptr, text_count as usize) };
            let hash = x11_text_sig(cells);
            if hash == self.last_text_hash {
                return;
            }
            self.last_text_hash = hash;
            for row in 0..25usize {
                for col in 0..80usize {
                    let cell = cells[row * 80 + col];
                    let ch = (cell & 0xFF) as u8;
                    let attr = (cell >> 8) as u8;
                    let fg = x11_color16(attr & 0x0F);
                    let bg = x11_color16((attr >> 4) & 0x0F);
                    let x = (col * 8) as c_int;
                    let y = (row * 16) as c_int;
                    unsafe {
                        XSetForeground(self.display, self.gc, bg);
                        XFillRectangle(self.display, self.window, self.gc, x, y, 8, 16);
                        if ch.is_ascii_graphic() || ch == b' ' {
                            XSetForeground(self.display, self.gc, fg);
                            let txt = [ch as c_char];
                            XDrawString(self.display, self.window, self.gc, x + 1, y + 13, txt.as_ptr(), 1);
                        }
                    }
                }
            }
        } else {
            let mut fb_w = 0u32; let mut fb_h = 0u32; let mut fb_bpp = 0u8;
            let fb_ptr = corevm_vga_get_framebuffer(vm, &mut fb_w, &mut fb_h, &mut fb_bpp);
            if fb_ptr.is_null() || fb_w == 0 || fb_h == 0 || self.ximage.is_null() {
                return;
            }
            let spp = (fb_bpp as usize).max(8).div_ceil(8);
            let src_len = (fb_w as usize).saturating_mul(fb_h as usize).saturating_mul(spp);
            let src = unsafe { std::slice::from_raw_parts(fb_ptr, src_len) };
            x11_blit_fb_to_bgra(&mut self.blit_buf, src, fb_w as usize, fb_h as usize, fb_bpp, self.win_w, self.win_h);
            unsafe {
                (*self.ximage).data = self.blit_buf.as_mut_ptr() as *mut c_char;
                XPutImage(self.display, self.window, self.gc, self.ximage,
                    0, 0, 0, 0, self.win_w as c_uint, self.win_h as c_uint);
            }
        }
        x11_draw_diag_overlay(self.display, self.window, self.gc, self.win_w, self.win_h, &self.diag_lines);
        unsafe { XFlush(self.display); }
        self.last_render = now;
    }
}

impl Drop for X11Window {
    fn drop(&mut self) {
        unsafe { XCloseDisplay(self.display); }
    }
}

fn x11_color16(idx: u8) -> c_ulong {
    let (r, g, b) = match idx & 0x0F {
        0x0 => (0x00u32, 0x00u32, 0x00u32), 0x1 => (0x00, 0x00, 0xAA), 0x2 => (0x00, 0xAA, 0x00),
        0x3 => (0x00, 0xAA, 0xAA), 0x4 => (0xAA, 0x00, 0x00), 0x5 => (0xAA, 0x00, 0xAA),
        0x6 => (0xAA, 0x55, 0x00), 0x7 => (0xAA, 0xAA, 0xAA), 0x8 => (0x55, 0x55, 0x55),
        0x9 => (0x55, 0x55, 0xFF), 0xA => (0x55, 0xFF, 0x55), 0xB => (0x55, 0xFF, 0xFF),
        0xC => (0xFF, 0x55, 0x55), 0xD => (0xFF, 0x55, 0xFF), 0xE => (0xFF, 0xFF, 0x55),
        _ => (0xFF, 0xFF, 0xFF),
    };
    ((r as c_ulong) << 16) | ((g as c_ulong) << 8) | (b as c_ulong)
}

fn x11_text_sig(cells: &[u16]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for &v in cells.iter().take(2000) {
        h ^= v as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn x11_draw_diag_overlay(display: *mut X11Display, window: c_ulong, gc: X11GC, win_w: usize, win_h: usize, lines: &[String]) {
    if lines.is_empty() { return; }
    let max_chars = lines.iter().map(|l| l.len()).max().unwrap_or(0);
    let box_w = max_chars * X11_CHAR_W + X11_OVERLAY_PAD * 2;
    let box_h = lines.len() * X11_CHAR_H + X11_OVERLAY_PAD * 2;
    let x = win_w.saturating_sub(box_w + X11_OVERLAY_PAD) as c_int;
    let y = win_h.saturating_sub(box_h + X11_OVERLAY_PAD) as c_int;
    unsafe {
        XSetForeground(display, gc, 0x000000);
        XFillRectangle(display, window, gc, x, y, box_w as c_uint, box_h as c_uint);
        XSetForeground(display, gc, 0x00FF88);
        for (i, line) in lines.iter().enumerate() {
            let tx = x + X11_OVERLAY_PAD as c_int;
            let ty = y + X11_OVERLAY_PAD as c_int + (i * X11_CHAR_H + 13) as c_int;
            XDrawString(display, window, gc, tx, ty, line.as_ptr() as *const c_char, line.len() as c_int);
        }
    }
}

fn x11_scancode_for_ascii(ch: u8) -> Option<(bool, u8)> {
    let lower = ch.to_ascii_lowercase();
    let shift = ch.is_ascii_uppercase();
    let code = match lower {
        b'1' => 0x02, b'2' => 0x03, b'3' => 0x04, b'4' => 0x05, b'5' => 0x06,
        b'6' => 0x07, b'7' => 0x08, b'8' => 0x09, b'9' => 0x0A, b'0' => 0x0B,
        b'q' => 0x10, b'w' => 0x11, b'e' => 0x12, b'r' => 0x13, b't' => 0x14,
        b'y' => 0x15, b'u' => 0x16, b'i' => 0x17, b'o' => 0x18, b'p' => 0x19,
        b'a' => 0x1E, b's' => 0x1F, b'd' => 0x20, b'f' => 0x21, b'g' => 0x22,
        b'h' => 0x23, b'j' => 0x24, b'k' => 0x25, b'l' => 0x26,
        b'z' => 0x2C, b'x' => 0x2D, b'c' => 0x2E, b'v' => 0x2F,
        b'b' => 0x30, b'n' => 0x31, b'm' => 0x32,
        b' ' => 0x39, b'\n' | b'\r' => 0x1C, b'\t' => 0x0F, 0x08 | 0x7F => 0x0E,
        _ => return None,
    };
    Some((shift, code))
}

fn x11_key_press_ascii(vm: u64, ch: u8) {
    if let Some((shift, code)) = x11_scancode_for_ascii(ch) {
        if shift { corevm_ps2_key_press(vm, 0x2A); }
        corevm_ps2_key_press(vm, code);
        corevm_ps2_key_release(vm, code);
        if shift { corevm_ps2_key_release(vm, 0x2A); }
    }
}

fn x11_key_press_extended(vm: u64, code: u8, release: bool) {
    corevm_ps2_key_press(vm, 0xE0);
    if release { corevm_ps2_key_release(vm, code); } else { corevm_ps2_key_press(vm, code); }
}

fn x11_keysym_to_ps2(vm: u64, keysym: c_ulong, press: bool) {
    match keysym {
        XK_RETURN => { if press { x11_key_press_ascii(vm, b'\n'); } }
        XK_TAB => { if press { x11_key_press_ascii(vm, b'\t'); } }
        XK_BACKSPACE => { if press { x11_key_press_ascii(vm, 0x08); } }
        XK_ESCAPE => { if press { corevm_ps2_key_press(vm, 0x01); corevm_ps2_key_release(vm, 0x01); } }
        XK_UP => x11_key_press_extended(vm, 0x48, !press),
        XK_DOWN => x11_key_press_extended(vm, 0x50, !press),
        XK_LEFT => x11_key_press_extended(vm, 0x4B, !press),
        XK_RIGHT => x11_key_press_extended(vm, 0x4D, !press),
        ks if (0x20..=0x7E).contains(&ks) => { if press { x11_key_press_ascii(vm, ks as u8); } }
        _ => {}
    }
}

fn x11_blit_fb_to_bgra(dst: &mut [u32], src: &[u8], src_w: usize, src_h: usize, src_bpp: u8, dst_w: usize, dst_h: usize) {
    let spp = (src_bpp as usize).max(8).div_ceil(8);
    for y in 0..dst_h {
        let sy = (y * src_h / dst_h).min(src_h.saturating_sub(1));
        for x in 0..dst_w {
            let sx = (x * src_w / dst_w).min(src_w.saturating_sub(1));
            let sidx = (sy * src_w + sx) * spp;
            let didx = y * dst_w + x;
            let (r, g, b) = match spp {
                1 => {
                    let v = *src.get(sidx).unwrap_or(&0);
                    if src_bpp == 4 {
                        let c = x11_color16(v & 0x0F);
                        (((c >> 16) & 0xFF) as u8, ((c >> 8) & 0xFF) as u8, (c & 0xFF) as u8)
                    } else { (v, v, v) }
                }
                2 => {
                    let lo = *src.get(sidx).unwrap_or(&0);
                    let hi = *src.get(sidx + 1).unwrap_or(&0);
                    let p = u16::from_le_bytes([lo, hi]);
                    let r = (((p >> 11) & 0x1F) as u32 * 255 / 31) as u8;
                    let g = (((p >> 5) & 0x3F) as u32 * 255 / 63) as u8;
                    let b = ((p & 0x1F) as u32 * 255 / 31) as u8;
                    (r, g, b)
                }
                _ => {
                    let b = *src.get(sidx).unwrap_or(&0);
                    let g = *src.get(sidx + 1).unwrap_or(&0);
                    let r = *src.get(sidx + 2).unwrap_or(&0);
                    (r, g, b)
                }
            };
            dst[didx] = (b as u32) | ((g as u32) << 8) | ((r as u32) << 16);
        }
    }
}

// ── End X11 ──

struct VmHandle(u64);

impl Drop for VmHandle {
    fn drop(&mut self) {
        if self.0 != 0 {
            corevm_destroy(self.0);
        }
    }
}

static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);
static KERNEL_LOOP_DUMPED: AtomicBool = AtomicBool::new(false);
static KERNEL_SPIN2_DUMPED: AtomicBool = AtomicBool::new(false);
static KERNEL_ENTRY2_DUMPED: AtomicBool = AtomicBool::new(false);
static CD_BOOT_DUMPED: AtomicBool = AtomicBool::new(false);
static LOWMEM_STAGE_DUMPED: AtomicBool = AtomicBool::new(false);
static WINDOWS_WAIT_DUMPED: AtomicBool = AtomicBool::new(false);
static WINDOWS_WAIT_CODE_DUMPED: AtomicBool = AtomicBool::new(false);
const IDE_IRQ_POLL_QUANTUM: u64 = 1_024;
const SIGINT: i32 = 2;

unsafe extern "C" {
    fn signal(sig: i32, handler: usize) -> usize;
}

static DEBUGGER_ACTIVE: AtomicBool = AtomicBool::new(false);

extern "C" fn on_sigint(_sig: i32) {
    if DEBUGGER_ACTIVE.load(Ordering::SeqCst) {
        // In debugger mode, Ctrl-C breaks into debugger instead of stopping
        corevm_debugger_break();
        return;
    }
    STOP_REQUESTED.store(true, Ordering::SeqCst);
}

fn run_batch_with_irq_poll(vm: u64, batch: u64) -> u32 {
    let mut remaining = batch.max(1);
    let mut exit_code = 2;

    while remaining > 0 {
        let chunk = remaining.min(IDE_IRQ_POLL_QUANTUM);
        exit_code = corevm_run(vm, chunk);
        if corevm_ide_irq_raised(vm) != 0 {
            corevm_pic_raise_irq(vm, 14);
            corevm_ide_clear_irq(vm);
        }
        if exit_code != 2 {
            break;
        }
        remaining -= chunk;
    }

    exit_code
}

#[derive(Clone, Debug)]
enum BiosKind {
    SeaBios,
    CoreVm,
}

#[derive(Clone, Debug)]
struct Config {
    bios_kind: BiosKind,
    bios: PathBuf,
    vgabios: PathBuf,
    bios_base: u64,
    iso: PathBuf,
    disk: PathBuf,
    ram_mb: u32,
    cores: u32,
    batch: u64,
    max_seconds: u64,
    max_instructions: u64,
    stdin_keyboard: bool,
    show_vga_text: bool,
    plain: bool,
    auto_enter_ms: u64,
    jit: bool,
    debugger: bool,
    no_display: bool,
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

fn first_existing_path(candidates: &[&str]) -> Option<PathBuf> {
    candidates
        .iter()
        .map(PathBuf::from)
        .find(|path| path.exists())
}

fn default_corevm_bios_path() -> PathBuf {
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

fn default_seabios_path() -> PathBuf {
    first_existing_path(&[
        "/mnt/c/Program Files/qemu/share/bios-256k.bin",
        "/mnt/c/Program Files/qemu/share/bios.bin",
    ])
    .unwrap_or_else(default_corevm_bios_path)
}

fn default_vgabios_path() -> PathBuf {
    first_existing_path(&[
        "/mnt/c/Program Files/qemu/share/vgabios.bin",
    ])
    .unwrap_or_else(|| PathBuf::from("/mnt/c/Program Files/qemu/share/vgabios.bin"))
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
        bios_kind: BiosKind::SeaBios,
        bios: default_seabios_path(),
        vgabios: default_vgabios_path(),
        bios_base: 0xC0000,
        iso: PathBuf::new(),
        disk: PathBuf::new(),
        ram_mb: 256,
        cores: 1,
        batch: 1_000_000,
        max_seconds: 120,
        max_instructions: 0,
        stdin_keyboard: false,
        show_vga_text: true,
        plain: false,
        auto_enter_ms: 0,
        jit: false,
        debugger: false,
        no_display: false,
    };

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--bios" => cfg.bios = PathBuf::from(args.next().ok_or("missing value for --bios")?),
            "--vgabios" => {
                cfg.vgabios = PathBuf::from(args.next().ok_or("missing value for --vgabios")?)
            }
            "--bios-base" => {
                let val = args.next().ok_or("missing value for --bios-base")?;
                cfg.bios_base = parse_u64(&val).ok_or("invalid --bios-base")?;
            }
            "--seabios" => {
                cfg.bios_kind = BiosKind::SeaBios;
                if cfg.bios == default_corevm_bios_path() {
                    cfg.bios = default_seabios_path();
                }
                cfg.bios_base = 0xC0000;
            }
            "--corevm-bios" => {
                cfg.bios_kind = BiosKind::CoreVm;
                cfg.bios = default_corevm_bios_path();
                cfg.bios_base = 0xF0000;
            }
            "--iso" => cfg.iso = PathBuf::from(args.next().ok_or("missing value for --iso")?),
            "--disk" => cfg.disk = PathBuf::from(args.next().ok_or("missing value for --disk")?),
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
            "--jit" => cfg.jit = true,
            "--debugger" | "--dbg" => cfg.debugger = true,
            "--no-display" => cfg.no_display = true,
            "--help" | "-h" => {
                return Err(String::new());
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    if cfg.iso.as_os_str().is_empty() && cfg.disk.as_os_str().is_empty() {
        return Err("missing required --iso <path> or --disk <path>".to_string());
    }
    Ok(cfg)
}

fn usage(program: &str) {
    eprintln!(
        "Usage: {program} [--iso <path>] [--disk <path>] [--seabios|--corevm-bios] [--bios <path>] [--vgabios <path>] [--bios-base <addr>] [--ram-mb <mb>] [--cores <n>] [--batch <n>] [--max-seconds <n>] [--max-instructions <n>] [--stdin-kbd] [--no-vga-text] [--plain] [--auto-enter-ms <ms>] [--jit] [--no-display]"
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
    let irq_p0 = corevm_irq_pending_word(handle, 0);
    let irq_p1 = corevm_irq_pending_word(handle, 1);
    let ioapic_r0 = corevm_ioapic_redir_entry(handle, 0);
    let ioapic_r1 = corevm_ioapic_redir_entry(handle, 1);
    let ioapic_r8 = corevm_ioapic_redir_entry(handle, 8);
    let ioapic_r14 = corevm_ioapic_redir_entry(handle, 14);
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
    let ss = corevm_get_segment_selector(handle, 2);
    let ss_base = corevm_get_segment_base(handle, 2);
    let cs_base = corevm_get_segment_base(handle, 1);
    let fs = corevm_get_segment_selector(handle, 4);
    let fs_base = corevm_get_segment_base(handle, 4);
    let cr0 = corevm_get_cr(handle, 0);
    let cr3 = corevm_get_cr(handle, 3);
    let apic_base_msr = corevm_get_msr(handle, 0x1B);
    let efer_msr = corevm_get_msr(handle, 0xC000_0080);
    let last_err_rip = corevm_get_last_error_rip(handle);
    let last_err = last_error(handle);
    let cpl = (cs & 0x3) as u8;
    let probe_addr = if mode == 0 {
        ((cs as u64) << 4).wrapping_add(ip16)
    } else {
        rip
    };
    let readb = |a: u64| -> u8 {
        if mode == 0 {
            corevm_read_phys_u8(handle, a)
        } else {
            corevm_read_linear_u8(handle, a)
        }
    };
    let mut bytes = [0u8; 8];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = readb(probe_addr.wrapping_add(i as u64));
    }
    let stack_linear = if mode == 0 {
        ((ss as u64) << 4).wrapping_add((sp as u16) as u64)
    } else {
        ss_base.wrapping_add(sp as u64)
    };
    let mut stack = [0u32; 4];
    for (i, w) in stack.iter_mut().enumerate() {
        let a = stack_linear.wrapping_add((i * 4) as u64);
        let b0 = readb(a) as u32;
        let b1 = readb(a.wrapping_add(1)) as u32;
        let b2 = readb(a.wrapping_add(2)) as u32;
        let b3 = readb(a.wrapping_add(3)) as u32;
        *w = b0 | (b1 << 8) | (b2 << 16) | (b3 << 24);
    }
    let ret = stack[1] as u64;
    let mut ret_bytes = [0u8; 8];
    for (i, b) in ret_bytes.iter_mut().enumerate() {
        *b = readb(ret.wrapping_add(i as u64));
    }
    let jiffies = if mode == 1 {
        let a = 0xC0B3_DC70u64;
        (readb(a) as u32)
            | ((readb(a.wrapping_add(1)) as u32) << 8)
            | ((readb(a.wrapping_add(2)) as u32) << 16)
            | ((readb(a.wrapping_add(3)) as u32) << 24)
    } else {
        0
    };
    let lpj_seed = if mode == 1 {
        let a = 0xC0B2_3100u64;
        (readb(a) as u32)
            | ((readb(a.wrapping_add(1)) as u32) << 8)
            | ((readb(a.wrapping_add(2)) as u32) << 16)
            | ((readb(a.wrapping_add(3)) as u32) << 24)
    } else {
        0
    };
    let fs_cal = if mode == 1 {
        let a = fs_base.wrapping_add(0xC0CC_7184u64);
        (readb(a) as u32)
            | ((readb(a.wrapping_add(1)) as u32) << 8)
            | ((readb(a.wrapping_add(2)) as u32) << 16)
            | ((readb(a.wrapping_add(3)) as u32) << 24)
    } else {
        0
    };
    let pv_ptr = if mode == 1 {
        let a = 0xC0CE_9960u64;
        (readb(a) as u32)
            | ((readb(a.wrapping_add(1)) as u32) << 8)
            | ((readb(a.wrapping_add(2)) as u32) << 16)
            | ((readb(a.wrapping_add(3)) as u32) << 24)
    } else {
        0
    };
    let max_pfn_mapped = if mode == 1 {
        let a = 0xC0CE_1860u64;
        (readb(a) as u32)
            | ((readb(a.wrapping_add(1)) as u32) << 8)
            | ((readb(a.wrapping_add(2)) as u32) << 16)
            | ((readb(a.wrapping_add(3)) as u32) << 24)
    } else {
        0
    };
    let relocated_initrd_start = if mode == 1 {
        let a = 0xC0CD_F054u64;
        (readb(a) as u32)
            | ((readb(a.wrapping_add(1)) as u32) << 8)
            | ((readb(a.wrapping_add(2)) as u32) << 16)
            | ((readb(a.wrapping_add(3)) as u32) << 24)
    } else {
        0
    };
    let relocated_initrd_end = if mode == 1 {
        let a = 0xC0CD_F058u64;
        (readb(a) as u32)
            | ((readb(a.wrapping_add(1)) as u32) << 8)
            | ((readb(a.wrapping_add(2)) as u32) << 16)
            | ((readb(a.wrapping_add(3)) as u32) << 24)
    } else {
        0
    };
    let mut pv_bytes = [0u8; 4];
    if mode == 1 && pv_ptr != 0 {
        for (i, b) in pv_bytes.iter_mut().enumerate() {
            *b = readb((pv_ptr as u64).wrapping_add(i as u64));
        }
    }
    let mut lapic_init = 0u32;
    let mut lapic_cur = 0u32;
    let mut lapic_div = 0u32;
    let lapic = corevm_lapic_diag_state(
        handle,
        &mut lapic_init as *mut u32,
        &mut lapic_cur as *mut u32,
        &mut lapic_div as *mut u32,
    );
    let lapic_svr = (lapic & 0xFFFF_FFFF) as u32;
    let lapic_lvt_timer = (lapic >> 32) as u32;
    let lapic_base = apic_base_msr & 0xFFFF_F000;
    let lapic_tpr = corevm_read_phys_u32(handle, lapic_base.wrapping_add(0x080));
    let lapic_ppr = corevm_read_phys_u32(handle, lapic_base.wrapping_add(0x0A0));
    let lapic_isr6 = corevm_read_phys_u32(handle, lapic_base.wrapping_add(0x160));
    let lapic_isr7 = corevm_read_phys_u32(handle, lapic_base.wrapping_add(0x170));
    let lapic_tmr6 = corevm_read_phys_u32(handle, lapic_base.wrapping_add(0x1E0));
    let lapic_tmr7 = corevm_read_phys_u32(handle, lapic_base.wrapping_add(0x1F0));
    let lapic_irr6 = corevm_read_phys_u32(handle, lapic_base.wrapping_add(0x260));
    let lapic_irr7 = corevm_read_phys_u32(handle, lapic_base.wrapping_add(0x270));
    let scan_boot_params = |hint: u64| -> Option<u64> {
        let rd32_phys = |base: u64| -> u32 {
            (corevm_read_phys_u8(handle, base) as u32)
                | ((corevm_read_phys_u8(handle, base.wrapping_add(1)) as u32) << 8)
                | ((corevm_read_phys_u8(handle, base.wrapping_add(2)) as u32) << 16)
                | ((corevm_read_phys_u8(handle, base.wrapping_add(3)) as u32) << 24)
        };
        let is_boot_params = |base: u64| -> bool {
            rd32_phys(base.wrapping_add(0x202)) == 0x5372_6448
        };
        if hint != 0 && hint < 0x10_0000 && is_boot_params(hint) {
            return Some(hint);
        }
        for cand in (0x1000u64..0x10_0000u64).step_by(0x10) {
            if is_boot_params(cand) {
                return Some(cand);
            }
        }
        None
    };
    if mode == 1
        && (rip & 0xFFFF_FFF0) == 0xC08D_F670
        && !KERNEL_LOOP_DUMPED.swap(true, Ordering::SeqCst)
    {
        let boot_params = scan_boot_params(0x0009_0000).unwrap_or(0x0009_0000);
        let e820_entries = corevm_read_phys_u8(handle, boot_params.wrapping_add(0x1E8));
        eprintln!(
            "[test-vmd] zeropage@0x{:08X} e820_entries={}",
            boot_params as u32,
            e820_entries
        );
        if e820_entries == 0 {
            let rd32 = |base: u64, off: u64| -> u32 {
                (corevm_read_phys_u8(handle, base.wrapping_add(off)) as u32)
                    | ((corevm_read_phys_u8(handle, base.wrapping_add(off + 1)) as u32) << 8)
                    | ((corevm_read_phys_u8(handle, base.wrapping_add(off + 2)) as u32) << 16)
                    | ((corevm_read_phys_u8(handle, base.wrapping_add(off + 3)) as u32) << 24)
            };
            for cand in (0x1000u64..0x10_0000u64).step_by(0x10) {
                let count = corevm_read_phys_u8(handle, cand.wrapping_add(0x1E8));
                if !(1..=8).contains(&count) {
                    continue;
                }
                if rd32(cand, 0x2D0) == 0
                    && rd32(cand, 0x2D4) == 0
                    && rd32(cand, 0x2D8) == 0x0009_FC00
                    && rd32(cand, 0x2DC) == 0
                    && rd32(cand, 0x2E0) == 1
                {
                    eprintln!(
                        "[test-vmd] zeropage candidate @0x{:08X} e820_entries={}",
                        cand as u32,
                        count
                    );
                    break;
                }
            }
        }
        for idx in 0..usize::from(e820_entries.min(8)) {
            let base = boot_params.wrapping_add(0x2D0).wrapping_add((idx as u64) * 20);
            let rd32 = |off: u64| -> u32 {
                (corevm_read_phys_u8(handle, base.wrapping_add(off)) as u32)
                    | ((corevm_read_phys_u8(handle, base.wrapping_add(off + 1)) as u32) << 8)
                    | ((corevm_read_phys_u8(handle, base.wrapping_add(off + 2)) as u32) << 16)
                    | ((corevm_read_phys_u8(handle, base.wrapping_add(off + 3)) as u32) << 24)
            };
            eprintln!(
                "[test-vmd] zp_e820[{idx}] base={:08X}:{:08X} len={:08X}:{:08X} type={:08X}",
                rd32(4),
                rd32(0),
                rd32(12),
                rd32(8),
                rd32(16)
            );
        }
        let base = 0xC08D_F650u64;
        let mut line = String::new();
        for i in 0..256u64 {
            if i % 16 == 0 {
                if !line.is_empty() {
                    eprintln!("{line}");
                    line.clear();
                }
                line.push_str(&format!(
                    "[test-vmd] kernblk {:08X}:",
                    base.wrapping_add(i) as u32
                ));
            }
            line.push_str(&format!(" {:02X}", readb(base.wrapping_add(i))));
        }
        if !line.is_empty() {
            eprintln!("{line}");
        }
        for base in [0xC010_2B20u64, 0xC010_2BC0u64] {
            let mut chunk = String::new();
            for i in 0..64u64 {
                if i % 16 == 0 {
                    if !chunk.is_empty() {
                        eprintln!("{chunk}");
                        chunk.clear();
                    }
                    chunk.push_str(&format!(
                        "[test-vmd] kernref {:08X}:",
                        base.wrapping_add(i) as u32
                    ));
                }
                chunk.push_str(&format!(" {:02X}", readb(base.wrapping_add(i))));
            }
            if !chunk.is_empty() {
                eprintln!("{chunk}");
            }
        }
        {
            let base = 0xC010_2AE0u64;
            let mut chunk = String::new();
            for i in 0..320u64 {
                if i % 16 == 0 {
                    if !chunk.is_empty() {
                        eprintln!("{chunk}");
                        chunk.clear();
                    }
                    chunk.push_str(&format!(
                        "[test-vmd] kernbig {:08X}:",
                        base.wrapping_add(i) as u32
                    ));
                }
                chunk.push_str(&format!(" {:02X}", readb(base.wrapping_add(i))));
            }
            if !chunk.is_empty() {
                eprintln!("{chunk}");
            }
        }
        {
            let base = 0xC08E_B7E0u64;
            let mut chunk = String::new();
            for i in 0..256u64 {
                if i % 16 == 0 {
                    if !chunk.is_empty() {
                        eprintln!("{chunk}");
                        chunk.clear();
                    }
                    chunk.push_str(&format!(
                        "[test-vmd] kernhot {:08X}:",
                        base.wrapping_add(i) as u32
                    ));
                }
                chunk.push_str(&format!(" {:02X}", readb(base.wrapping_add(i))));
            }
            if !chunk.is_empty() {
                eprintln!("{chunk}");
            }
        }
        let mut chain = String::from("[test-vmd] stack chain:");
        for i in 0..16u64 {
            let a = stack_linear.wrapping_add(i * 4);
            let v = (readb(a) as u32)
                | ((readb(a.wrapping_add(1)) as u32) << 8)
                | ((readb(a.wrapping_add(2)) as u32) << 16)
                | ((readb(a.wrapping_add(3)) as u32) << 24);
            chain.push_str(&format!(" {:08X}", v));
        }
        eprintln!("{chain}");
    }
    if mode == 1
        && (rip & 0xFFFF_FFF0) == 0xC0C0_74F0
        && !KERNEL_SPIN2_DUMPED.swap(true, Ordering::SeqCst)
    {
        let base = 0xC0C0_74C0u64;
        let mut line = String::new();
        for i in 0..192u64 {
            if i % 16 == 0 {
                if !line.is_empty() {
                    eprintln!("{line}");
                    line.clear();
                }
                line.push_str(&format!(
                    "[test-vmd] spinblk {:08X}:",
                    base.wrapping_add(i) as u32
                ));
            }
            line.push_str(&format!(" {:02X}", readb(base.wrapping_add(i))));
        }
        if !line.is_empty() {
            eprintln!("{line}");
        }
        // Dump the first 0x30 vectors from IDT to see whether #BP/#UD/#PF/IRQ0
        // point to expected handlers.
        let idtr_base = if mode == 1 {
            let ptr = 0x0000_0000u64; // unused here; keep local scope explicit
            let _ = ptr;
            0
        } else {
            0
        };
        let _ = idtr_base;
    }
    if mode == 1
        && rip >= 0xC000_0000
        && !KERNEL_ENTRY2_DUMPED.swap(true, Ordering::SeqCst)
    {
        let base = (rip & !0xFF) as u64;
        let mut line = String::new();
        for i in 0..256u64 {
            if i % 16 == 0 {
                if !line.is_empty() {
                    eprintln!("{line}");
                    line.clear();
                }
                line.push_str(&format!(
                    "[test-vmd] ent2blk {:08X}:",
                    base.wrapping_add(i) as u32
                ));
            }
            line.push_str(&format!(" {:02X}", readb(base.wrapping_add(i))));
        }
        if !line.is_empty() {
            eprintln!("{line}");
        }
        let boot_params = scan_boot_params(si as u64).unwrap_or(si as u64);
        if boot_params != 0 && boot_params < 0x20_0000 {
            let e820_entries = corevm_read_phys_u8(handle, boot_params.wrapping_add(0x1E8));
            eprintln!(
                "[test-vmd] boot_params=0x{:08X} e820_entries={}",
                boot_params as u32,
                e820_entries
            );
            for idx in 0..usize::from(e820_entries.min(8)) {
                let base = boot_params.wrapping_add(0x2D0).wrapping_add((idx as u64) * 20);
                let rd32 = |off: u64| -> u32 {
                    (corevm_read_phys_u8(handle, base.wrapping_add(off)) as u32)
                        | ((corevm_read_phys_u8(handle, base.wrapping_add(off + 1)) as u32) << 8)
                        | ((corevm_read_phys_u8(handle, base.wrapping_add(off + 2)) as u32) << 16)
                        | ((corevm_read_phys_u8(handle, base.wrapping_add(off + 3)) as u32) << 24)
                };
                eprintln!(
                    "[test-vmd] e820[{idx}] base={:08X}:{:08X} len={:08X}:{:08X} type={:08X}",
                    rd32(4),
                    rd32(0),
                    rd32(12),
                    rd32(8),
                    rd32(16)
                );
            }
        }
        let mut chain = String::from("[test-vmd] ent2 stack chain:");
        for i in 0..16u64 {
            let a = stack_linear.wrapping_add(i * 4);
            let v = (readb(a) as u32)
                | ((readb(a.wrapping_add(1)) as u32) << 8)
                | ((readb(a.wrapping_add(2)) as u32) << 16)
                | ((readb(a.wrapping_add(3)) as u32) << 24);
            chain.push_str(&format!(" {:08X}", v));
        }
        eprintln!("{chain}");
    }
    if mode == 0 && rip <= 1 && dx == 0xE0 && !CD_BOOT_DUMPED.swap(true, Ordering::SeqCst) {
        for base in [0x7C00u64, 0x7C40u64, 0x7C6Cu64, 0x7E00u64] {
            let mut line = String::new();
            for i in 0..64u64 {
                if i % 16 == 0 {
                    if !line.is_empty() {
                        eprintln!("{line}");
                        line.clear();
                    }
                    line.push_str(&format!(
                        "[test-vmd] bootmem {:08X}:",
                        base.wrapping_add(i) as u32
                    ));
                }
                line.push_str(&format!(
                    " {:02X}",
                    corevm_read_phys_u8(handle, base.wrapping_add(i))
                ));
            }
            if !line.is_empty() {
                eprintln!("{line}");
            }
        }
    }
    if mode == 0 && rip < 0x100 && !LOWMEM_STAGE_DUMPED.swap(true, Ordering::SeqCst) {
        for base in [0x80A0u64, 0x9770u64, 0x97A0u64, 0xAAD0u64] {
            let mut line = String::new();
            for i in 0..64u64 {
                if i % 16 == 0 {
                    if !line.is_empty() {
                        eprintln!("{line}");
                        line.clear();
                    }
                    line.push_str(&format!(
                        "[test-vmd] stagedump {:08X}:",
                        base.wrapping_add(i) as u32
                    ));
                }
                line.push_str(&format!(
                    " {:02X}",
                    corevm_read_phys_u8(handle, base.wrapping_add(i))
                ));
            }
            if !line.is_empty() {
                eprintln!("{line}");
            }
        }
    }
    if mode == 1
        && ((rip & 0xFFFF_FFF0) == 0x8080_9BF0 || (rip & 0xFFFF_FFF0) == 0x8081_4850)
        && !WINDOWS_WAIT_DUMPED.swap(true, Ordering::SeqCst)
    {
        eprintln!(
            "[test-vmd] windows-wait rip={:08X} lapic[svr={:08X} tpr={:08X} ppr={:08X} lvt={:08X} init={:08X} cur={:08X} div={:08X} isr6={:08X} isr7={:08X} irr6={:08X} irr7={:08X} tmr6={:08X} tmr7={:08X}] irqp[0={:016X} 1={:016X}] ioapic[r8={:016X}]",
            rip as u32,
            lapic_svr,
            lapic_tpr,
            lapic_ppr,
            lapic_lvt_timer,
            lapic_init,
            lapic_cur,
            lapic_div,
            lapic_isr6,
            lapic_isr7,
            lapic_irr6,
            lapic_irr7,
            lapic_tmr6,
            lapic_tmr7,
            irq_p0,
            irq_p1,
            ioapic_r8,
        );
    }
    if mode == 1
        && (0x8082_8800..0x8082_8900).contains(&rip)
        && !WINDOWS_WAIT_CODE_DUMPED.swap(true, Ordering::SeqCst)
    {
        for base in [0x8081_4820u64, 0x8082_8800u64] {
            let mut line = String::new();
            for i in 0..96u64 {
                if i % 16 == 0 {
                    if !line.is_empty() {
                        eprintln!("{line}");
                        line.clear();
                    }
                    line.push_str(&format!("[test-vmd] wincode {:08X}:", base.wrapping_add(i) as u32));
                }
                line.push_str(&format!(" {:02X}", readb(base.wrapping_add(i))));
            }
            if !line.is_empty() {
                eprintln!("{line}");
            }
        }
    }
    eprintln!(
        "[test-vmd] cpu probe: mode={} cpl={} cs:ip={:04X}:{:04X} cs_base={:08X} addr={:08X} ss={:04X} ss_base={:08X} fs={:04X} fs_base={:08X} EAX={:08X} EBX={:08X} ECX={:08X} EDX={:08X} ESI={:08X} EDI={:08X} EBP={:08X} ESP={:08X} FLAGS={:04X} IF={} ZF={} CF={} CR0={:08X} CR3={:08X} APIC_BASE={:08X} EFER={:08X} JIFF={:08X} LPJ={:08X} FSCAL={:08X} MAXPFN={:08X} MAXBYTES={:08X} PVOP={:08X}[{:02X} {:02X} {:02X} {:02X}] INITRD_DST=[{:08X},{:08X}) LAPIC[svr={:08X} lvt={:08X} init={:08X} cur={:08X} div={:08X} tpr={:08X} ppr={:08X} isr6={:08X} isr7={:08X} irr6={:08X} irr7={:08X} tmr6={:08X} tmr7={:08X}] IRQP[0={:016X} 1={:016X}] IOAPIC[r0={:016X} r1={:016X} r8={:016X} r14={:016X}] STK={:08X}@{:08X} {:08X} {:08X} {:08X} RET={:08X} rbytes={:02X} {:02X} {:02X} {:02X} {:02X} {:02X} {:02X} {:02X} PIC[m:{:02X}/{:02X}/{:02X} s:{:02X}/{:02X}/{:02X}] ERR_RIP={:08X} ERR='{}' bytes={:02X} {:02X} {:02X} {:02X} {:02X} {:02X} {:02X} {:02X}",
        mode_name(mode),
        cpl,
        cs,
        ip16 as u32,
        cs_base as u32,
        probe_addr as u32,
        ss,
        ss_base as u32,
        fs,
        fs_base as u32,
        ax as u32,
        bx as u32,
        cx as u32,
        dx as u32,
        si as u32,
        di as u32,
        bp as u32,
        sp as u32,
        (rflags & 0xFFFF) as u16,
        if (rflags & 0x0200) != 0 { 1 } else { 0 },
        if (rflags & 0x0040) != 0 { 1 } else { 0 },
        if (rflags & 0x0001) != 0 { 1 } else { 0 },
        cr0 as u32,
        cr3 as u32,
        apic_base_msr as u32,
        efer_msr as u32,
        jiffies,
        lpj_seed,
        fs_cal,
        max_pfn_mapped,
        max_pfn_mapped << 12,
        pv_ptr,
        pv_bytes[0],
        pv_bytes[1],
        pv_bytes[2],
        pv_bytes[3],
        relocated_initrd_start,
        relocated_initrd_end,
        lapic_svr,
        lapic_lvt_timer,
        lapic_init,
        lapic_cur,
        lapic_div,
        lapic_tpr,
        lapic_ppr,
        lapic_isr6,
        lapic_isr7,
        lapic_irr6,
        lapic_irr7,
        lapic_tmr6,
        lapic_tmr7,
        irq_p0,
        irq_p1,
        ioapic_r0,
        ioapic_r1,
        ioapic_r8,
        ioapic_r14,
        stack_linear as u32,
        stack[0],
        stack[1],
        stack[2],
        stack[3],
        ret as u32,
        ret_bytes[0],
        ret_bytes[1],
        ret_bytes[2],
        ret_bytes[3],
        ret_bytes[4],
        ret_bytes[5],
        ret_bytes[6],
        ret_bytes[7],
        m_irr,
        m_isr,
        m_imr,
        s_irr,
        s_isr,
        s_imr,
        last_err_rip as u32,
        last_err,
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

fn load_guest_bios(vm: u64, cfg: &Config, bios: &[u8], vgabios: Option<&[u8]>) -> Result<(), String> {
    match cfg.bios_kind {
        BiosKind::CoreVm => {
            let rc = corevm_load_rom(vm, cfg.bios_base, bios.as_ptr(), bios.len() as u32);
            if rc != 0 {
                return Err(format!("corevm_load_rom failed (rc={rc})"));
            }
        }
        BiosKind::SeaBios => {
            let rc = corevm_load_binary(vm, cfg.bios_base, bios.as_ptr(), bios.len() as u32);
            if rc != 0 {
                return Err(format!("corevm_load_binary failed (rc={rc})"));
            }
            let rc = corevm_load_rom(vm, 0xFFFC_0000, bios.as_ptr(), bios.len() as u32);
            if rc != 0 {
                return Err(format!("corevm_load_rom overlay failed (rc={rc})"));
            }
            let vgabios = vgabios.ok_or("missing vgabios for SeaBIOS")?;
            let name = CString::new("vgaroms/vgabios.bin").unwrap();
            let rc = corevm_fw_cfg_add_file(
                vm,
                name.as_ptr() as *const u8,
                vgabios.as_ptr(),
                vgabios.len() as u32,
            );
            if rc != 0 {
                return Err(format!("corevm_fw_cfg_add_file failed (rc={rc})"));
            }
            corevm_set_rip(vm, 0xFFF0);
        }
    }
    Ok(())
}

const VGA_TEXT_ROWS: usize = 25;
const VGA_TEXT_COLS: usize = 80;
const UI_LOG_TAIL: usize = 16;
const FB_RAMP: &[u8] = b" .:-=+*#%@";
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
    if matches!(cfg.bios_kind, BiosKind::SeaBios) {
        if let Err(e) = ensure_exists(&cfg.vgabios, "vgabios") {
            eprintln!("{e}");
            std::process::exit(2);
        }
    }
    if !cfg.iso.as_os_str().is_empty() {
        if let Err(e) = ensure_exists(&cfg.iso, "iso") {
            eprintln!("{e}");
            std::process::exit(2);
        }
    }

    let bios = match fs::read(&cfg.bios) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("failed to read bios {}: {e}", cfg.bios.display());
            std::process::exit(2);
        }
    };
    let vgabios = if matches!(cfg.bios_kind, BiosKind::SeaBios) {
        Some(fs::read(&cfg.vgabios).unwrap_or_else(|e| {
            eprintln!("failed to read vgabios {}: {e}", cfg.vgabios.display());
            std::process::exit(2);
        }))
    } else {
        None
    };
    let iso = if !cfg.iso.as_os_str().is_empty() {
        match fs::read(&cfg.iso) {
            Ok(i) => i,
            Err(e) => {
                eprintln!("failed to read iso {}: {e}", cfg.iso.display());
                std::process::exit(2);
            }
        }
    } else {
        Vec::new()
    };
    let disk = if !cfg.disk.as_os_str().is_empty() {
        match fs::read(&cfg.disk) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("failed to read disk {}: {e}", cfg.disk.display());
                std::process::exit(2);
            }
        }
    } else {
        Vec::new()
    };

    eprintln!(
        "[test-vmd] bios_kind={:?} bios={} ({} bytes) iso={} ({} bytes) disk={} ({} bytes) ram={}MiB cores={} stdin_kbd={} vga_text={}",
        cfg.bios_kind,
        cfg.bios.display(),
        bios.len(),
        cfg.iso.display(),
        iso.len(),
        cfg.disk.display(),
        disk.len(),
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
    corevm_setup_standard_devices(vm.0);
    corevm_setup_pci_bus(vm.0);
    corevm_setup_ide(vm.0);
    if let Err(e) = load_guest_bios(vm.0, &cfg, &bios, vgabios.as_deref()) {
        eprintln!("{e}");
        std::process::exit(1);
    }
    if !disk.is_empty() {
        corevm_ide_attach_disk(vm.0, disk.as_ptr(), disk.len() as u32);
    }
    if !iso.is_empty() {
        corevm_ide_attach_slave(vm.0, iso.as_ptr(), iso.len() as u32);
    }
    if cfg.jit {
        corevm_jit_enable(vm.0, 1);
    }
    if cfg.debugger {
        corevm_debugger_enable();
        DEBUGGER_ACTIVE.store(true, Ordering::SeqCst);
    }

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

    let mut x11 = if !cfg.no_display && env::var("DISPLAY").is_ok() {
        match X11Window::open() {
            Some(w) => {
                eprintln!("[test-vmd] X11 display opened");
                Some(w)
            }
            None => {
                eprintln!("[test-vmd] X11 display unavailable, continuing without");
                None
            }
        }
    } else {
        None
    };

    loop {
        if STOP_REQUESTED.load(Ordering::SeqCst) {
            break;
        }

        while let Ok(ch) = kbd_rx.try_recv() {
            inject_ascii_key(vm.0, ch);
        }

        let exit_code = run_batch_with_irq_poll(vm.0, cfg.batch);

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
        if let Some(ref mut xw) = x11 {
            xw.process_events(vm.0);
            xw.run_calls = xw.run_calls.saturating_add(1);
            xw.update_overlay(vm.0, cfg.jit);
            xw.render(vm.0);
        }
        let mode = corevm_get_mode(vm.0);
        let cs = corevm_get_segment_selector(vm.0, 1);
        let rip = corevm_get_rip(vm.0);
        if mode == 1 && rip >= 0xC000_0000 && !KERNEL_ENTRY2_DUMPED.load(Ordering::SeqCst) {
            dump_cpu_probe(vm.0, mode, cs, rip);
        }
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
                // Dump exception ring once when stuck in kernel space
                if rip >= 0x8000_0000 && !KERNEL_LOOP_DUMPED.swap(true, Ordering::SeqCst) {
                    corevm_dump_exception_ring(vm.0);
                    // Dump stack
                    let esp = corevm_get_gpr(vm.0, 4);
                    eprintln!("[bsod-stack] ESP={:08X} dumping 64 dwords:", esp);
                    for i in 0..64u64 {
                        let addr = esp.wrapping_add(i * 4);
                        let b0 = corevm_read_linear_u8(vm.0, addr) as u32;
                        let b1 = corevm_read_linear_u8(vm.0, addr + 1) as u32;
                        let b2 = corevm_read_linear_u8(vm.0, addr + 2) as u32;
                        let b3 = corevm_read_linear_u8(vm.0, addr + 3) as u32;
                        let dw = b0 | (b1 << 8) | (b2 << 16) | (b3 << 24);
                        if i % 8 == 0 { eprint!("\n  {:08X}:", addr); }
                        eprint!(" {:08X}", dw);
                    }
                    eprintln!();
                    // Dump bytes at last #UD and #GP addresses from exception ring
                    // Dump stack at EBP chain
                    let mut ebp = corevm_get_gpr(vm.0, 5); // EBP
                    for frame in 0..8 {
                        let ret_addr_loc = ebp.wrapping_add(4);
                        let ret_b0 = corevm_read_linear_u8(vm.0, ret_addr_loc) as u32;
                        let ret_b1 = corevm_read_linear_u8(vm.0, ret_addr_loc + 1) as u32;
                        let ret_b2 = corevm_read_linear_u8(vm.0, ret_addr_loc + 2) as u32;
                        let ret_b3 = corevm_read_linear_u8(vm.0, ret_addr_loc + 3) as u32;
                        let ret = ret_b0 | (ret_b1 << 8) | (ret_b2 << 16) | (ret_b3 << 24);
                        let next_b0 = corevm_read_linear_u8(vm.0, ebp) as u64;
                        let next_b1 = corevm_read_linear_u8(vm.0, ebp + 1) as u64;
                        let next_b2 = corevm_read_linear_u8(vm.0, ebp + 2) as u64;
                        let next_b3 = corevm_read_linear_u8(vm.0, ebp + 3) as u64;
                        let next_ebp = next_b0 | (next_b1 << 8) | (next_b2 << 16) | (next_b3 << 24);
                        // Dump 8 dwords of args after ret addr
                        eprint!("[frame{}] EBP={:08X} RET={:08X} args:", frame, ebp, ret);
                        for a in 0..8u64 {
                            let addr = ebp.wrapping_add(8 + a * 4);
                            let ab0 = corevm_read_linear_u8(vm.0, addr) as u32;
                            let ab1 = corevm_read_linear_u8(vm.0, addr + 1) as u32;
                            let ab2 = corevm_read_linear_u8(vm.0, addr + 2) as u32;
                            let ab3 = corevm_read_linear_u8(vm.0, addr + 3) as u32;
                            eprint!(" {:08X}", ab0 | (ab1 << 8) | (ab2 << 16) | (ab3 << 24));
                        }
                        eprintln!();
                        if next_ebp == 0 || next_ebp < 0x80000000 { break; }
                        ebp = next_ebp;
                    }
                    for dump_addr in [0x8019B90Bu64, 0x8019BDA1u64, 0x80806F9Eu64, rip] {
                        eprint!("[bsod-code] {:08X}:", dump_addr);
                        for j in 0..16u64 {
                            let b = corevm_read_linear_u8(vm.0, dump_addr + j);
                            eprint!(" {:02X}", b);
                        }
                        eprintln!();
                    }
                }
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
            }
            1 => {
                let err = last_error(vm.0);
                let rip = corevm_get_last_error_rip(vm.0);
                eprintln!("[test-vmd] fatal exception at rip=0x{rip:X}: {err}");
                break;
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
            5 => {
                eprintln!("[test-vmd] PS/2 system reset — resetting VM");
                corevm_reset(vm.0);
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
    let elapsed = start.elapsed().as_secs_f64().max(0.001);
    let total_ic = corevm_get_instruction_count(vm.0);
    let mips = (total_ic as f64) / elapsed / 1_000_000.0;
    let mut blocks_compiled = 0u64;
    let mut native_count = 0u64;
    let mut fallback_count = 0u64;
    let mut code_used = 0u32;
    corevm_jit_stats(
        vm.0,
        &mut blocks_compiled as *mut u64,
        &mut native_count as *mut u64,
        &mut fallback_count as *mut u64,
        &mut code_used as *mut u32,
    );
    let mut cache_blocks = 0u32;
    let mut cache_hits = 0u64;
    let mut cache_misses = 0u64;
    corevm_jit_cache_stats(
        vm.0,
        &mut cache_blocks as *mut u32,
        &mut cache_hits as *mut u64,
        &mut cache_misses as *mut u64,
    );
    eprintln!(
        "[test-vmd] perf: elapsed={:.2}s ic={} ({:.2} MIPS) jit={} blocks={} native={} fallback={} code={}B cache_blocks={} hits={} misses={}",
        elapsed,
        total_ic,
        mips,
        if cfg.jit { 1 } else { 0 },
        blocks_compiled,
        native_count,
        fallback_count,
        code_used,
        cache_blocks,
        cache_hits,
        cache_misses
    );
    if !interactive_ui && display.in_text_mode {
        dump_text_screen(&display.text_cells);
    }

    // Restore terminal state on exit.
    if interactive_ui {
        println!("\x1B[?25h\x1B[?1049l");
    }
}
