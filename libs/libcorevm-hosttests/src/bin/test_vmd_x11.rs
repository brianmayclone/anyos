use std::env;
use std::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use libcorevm::{
    corevm_create_ex, corevm_debug_take_output, corevm_destroy, corevm_get_last_error,
    corevm_get_last_error_rip, corevm_get_mode, corevm_ide_attach_slave, corevm_load_rom,
    corevm_pic_raise_irq, corevm_pit_advance, corevm_ps2_key_press, corevm_ps2_key_release,
    corevm_run, corevm_serial_take_output, corevm_setup_ide, corevm_setup_pci_bus,
    corevm_setup_standard_devices, corevm_vga_get_framebuffer, corevm_vga_get_text_buffer,
};

const PIT_TICKS_PER_MS: u32 = 1193;
const PIT_MAX_ADVANCE_MS: u64 = 10;

const KEY_PRESS: c_int = 2;
const KEY_RELEASE: c_int = 3;
const CONFIGURE_NOTIFY: c_int = 22;
const KEY_PRESS_MASK: c_long = 1 << 0;
const KEY_RELEASE_MASK: c_long = 1 << 1;
const EXPOSURE_MASK: c_long = 1 << 15;
const STRUCTURE_NOTIFY_MASK: c_long = 1 << 17;
const ZPIXMAP: c_int = 2;

const XK_BACKSPACE: c_ulong = 0xFF08;
const XK_TAB: c_ulong = 0xFF09;
const XK_RETURN: c_ulong = 0xFF0D;
const XK_ESCAPE: c_ulong = 0xFF1B;
const XK_LEFT: c_ulong = 0xFF51;
const XK_UP: c_ulong = 0xFF52;
const XK_RIGHT: c_ulong = 0xFF53;
const XK_DOWN: c_ulong = 0xFF54;

#[repr(C)]
struct Display(c_void);
#[repr(C)]
struct Visual(c_void);
#[repr(C)]
struct _XGC(c_void);
type GC = *mut _XGC;

#[repr(C)]
struct XEvent {
    pad: [c_long; 24],
}

#[repr(C)]
struct XAnyEvent {
    type_: c_int,
    serial: c_ulong,
    send_event: c_int,
    display: *mut Display,
    window: c_ulong,
}

#[repr(C)]
struct XKeyEvent {
    type_: c_int,
    serial: c_ulong,
    send_event: c_int,
    display: *mut Display,
    window: c_ulong,
    root: c_ulong,
    subwindow: c_ulong,
    time: c_ulong,
    x: c_int,
    y: c_int,
    x_root: c_int,
    y_root: c_int,
    state: c_uint,
    keycode: c_uint,
    same_screen: c_int,
}

#[repr(C)]
struct XConfigureEvent {
    type_: c_int,
    serial: c_ulong,
    send_event: c_int,
    display: *mut Display,
    event: c_ulong,
    window: c_ulong,
    x: c_int,
    y: c_int,
    width: c_int,
    height: c_int,
    border_width: c_int,
    above: c_ulong,
    override_redirect: c_int,
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
    fn XOpenDisplay(name: *const c_char) -> *mut Display;
    fn XCloseDisplay(display: *mut Display) -> c_int;
    fn XDefaultScreen(display: *mut Display) -> c_int;
    fn XRootWindow(display: *mut Display, screen: c_int) -> c_ulong;
    fn XBlackPixel(display: *mut Display, screen: c_int) -> c_ulong;
    fn XWhitePixel(display: *mut Display, screen: c_int) -> c_ulong;
    fn XDefaultVisual(display: *mut Display, screen: c_int) -> *mut Visual;
    fn XDefaultDepth(display: *mut Display, screen: c_int) -> c_int;
    fn XCreateSimpleWindow(
        display: *mut Display,
        parent: c_ulong,
        x: c_int,
        y: c_int,
        width: c_uint,
        height: c_uint,
        border_width: c_uint,
        border: c_ulong,
        background: c_ulong,
    ) -> c_ulong;
    fn XStoreName(display: *mut Display, window: c_ulong, name: *const c_char) -> c_int;
    fn XSelectInput(display: *mut Display, window: c_ulong, mask: c_long) -> c_int;
    fn XMapWindow(display: *mut Display, window: c_ulong) -> c_int;
    fn XCreateGC(display: *mut Display, drawable: c_ulong, v: c_ulong, values: *mut c_void) -> GC;
    fn XSetForeground(display: *mut Display, gc: GC, color: c_ulong) -> c_int;
    fn XFillRectangle(
        display: *mut Display,
        drawable: c_ulong,
        gc: GC,
        x: c_int,
        y: c_int,
        width: c_uint,
        height: c_uint,
    ) -> c_int;
    fn XDrawString(
        display: *mut Display,
        drawable: c_ulong,
        gc: GC,
        x: c_int,
        y: c_int,
        text: *const c_char,
        len: c_int,
    ) -> c_int;
    fn XPending(display: *mut Display) -> c_int;
    fn XNextEvent(display: *mut Display, event: *mut XEvent) -> c_int;
    fn XLookupKeysym(event: *mut XKeyEvent, index: c_int) -> c_ulong;
    fn XCreateImage(
        display: *mut Display,
        visual: *mut Visual,
        depth: c_uint,
        format: c_int,
        offset: c_int,
        data: *mut c_char,
        width: c_uint,
        height: c_uint,
        bitmap_pad: c_int,
        bytes_per_line: c_int,
    ) -> *mut XImage;
    fn XPutImage(
        display: *mut Display,
        drawable: c_ulong,
        gc: GC,
        image: *mut XImage,
        src_x: c_int,
        src_y: c_int,
        dst_x: c_int,
        dst_y: c_int,
        width: c_uint,
        height: c_uint,
    ) -> c_int;
    fn XFlush(display: *mut Display) -> c_int;
}

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
        ram_mb: 64,
        cores: 1,
        batch: 50_000,
        max_seconds: 300,
    };
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--bios" => cfg.bios = PathBuf::from(args.next().ok_or("missing value for --bios")?),
            "--bios-base" => {
                let v = args.next().ok_or("missing value for --bios-base")?;
                cfg.bios_base = parse_u64(&v).ok_or("invalid --bios-base")?;
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
            "--help" | "-h" => return Err(String::new()),
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    if cfg.iso.as_os_str().is_empty() {
        return Err("missing required --iso <path>".to_string());
    }
    Ok(cfg)
}

fn usage(program: &str) {
    eprintln!("Usage: {program} --iso <path> [--bios <path>] [--bios-base <addr>] [--ram-mb <mb>] [--cores <n>] [--batch <n>] [--max-seconds <n>]");
}

fn ensure_exists(path: &Path, what: &str) -> Result<(), String> {
    if path.exists() {
        Ok(())
    } else {
        Err(format!("{what} not found: {}", path.display()))
    }
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

fn last_error(handle: u64) -> String {
    let mut buf = [0u8; 1024];
    let n = corevm_get_last_error(handle, buf.as_mut_ptr(), buf.len() as u32);
    if n == 0 {
        String::new()
    } else {
        String::from_utf8_lossy(&buf[..n as usize]).to_string()
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

fn color16(idx: u8) -> c_ulong {
    let (r, g, b) = match idx & 0x0F {
        0x0 => (0x00, 0x00, 0x00),
        0x1 => (0x00, 0x00, 0xAA),
        0x2 => (0x00, 0xAA, 0x00),
        0x3 => (0x00, 0xAA, 0xAA),
        0x4 => (0xAA, 0x00, 0x00),
        0x5 => (0xAA, 0x00, 0xAA),
        0x6 => (0xAA, 0x55, 0x00),
        0x7 => (0xAA, 0xAA, 0xAA),
        0x8 => (0x55, 0x55, 0x55),
        0x9 => (0x55, 0x55, 0xFF),
        0xA => (0x55, 0xFF, 0x55),
        0xB => (0x55, 0xFF, 0xFF),
        0xC => (0xFF, 0x55, 0x55),
        0xD => (0xFF, 0x55, 0xFF),
        0xE => (0xFF, 0xFF, 0x55),
        _ => (0xFF, 0xFF, 0xFF),
    };
    ((r as c_ulong) << 16) | ((g as c_ulong) << 8) | (b as c_ulong)
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

fn key_press_ascii(vm: u64, ch: u8) {
    if let Some((shift, code)) = scancode_for_ascii(ch) {
        if shift {
            corevm_ps2_key_press(vm, 0x2A);
        }
        corevm_ps2_key_press(vm, code);
        corevm_ps2_key_release(vm, code);
        if shift {
            corevm_ps2_key_release(vm, 0x2A);
        }
    }
}

fn key_press_extended(vm: u64, code: u8, release: bool) {
    corevm_ps2_key_press(vm, 0xE0);
    if release {
        corevm_ps2_key_release(vm, code);
    } else {
        corevm_ps2_key_press(vm, code);
    }
}

fn keysym_to_ps2(vm: u64, keysym: c_ulong, press: bool) {
    match keysym {
        XK_RETURN => {
            if press {
                key_press_ascii(vm, b'\n');
            }
        }
        XK_TAB => {
            if press {
                key_press_ascii(vm, b'\t');
            }
        }
        XK_BACKSPACE => {
            if press {
                key_press_ascii(vm, 0x08);
            }
        }
        XK_ESCAPE => {
            if press {
                corevm_ps2_key_press(vm, 0x01);
                corevm_ps2_key_release(vm, 0x01);
            }
        }
        XK_UP => key_press_extended(vm, 0x48, !press),
        XK_DOWN => key_press_extended(vm, 0x50, !press),
        XK_LEFT => key_press_extended(vm, 0x4B, !press),
        XK_RIGHT => key_press_extended(vm, 0x4D, !press),
        ks if (0x20..=0x7E).contains(&ks) => {
            if press {
                key_press_ascii(vm, ks as u8);
            }
        }
        _ => {}
    }
}

fn blit_fb_to_bgra(dst: &mut [u32], src: &[u8], src_w: usize, src_h: usize, src_bpp: u8, dst_w: usize, dst_h: usize) {
    let src_bytes_per_pixel = (src_bpp as usize).max(8).div_ceil(8);
    for y in 0..dst_h {
        let sy = (y * src_h / dst_h).min(src_h.saturating_sub(1));
        for x in 0..dst_w {
            let sx = (x * src_w / dst_w).min(src_w.saturating_sub(1));
            let sidx = (sy * src_w + sx).saturating_mul(src_bytes_per_pixel);
            let didx = y * dst_w + x;
            let (r, g, b) = match src_bytes_per_pixel {
                1 => {
                    let v = *src.get(sidx).unwrap_or(&0);
                    if src_bpp == 4 {
                        // VGA 16-color indexed mode: low nibble = palette index.
                        match v & 0x0F {
                            0x0 => (0x00, 0x00, 0x00),
                            0x1 => (0x00, 0x00, 0xAA),
                            0x2 => (0x00, 0xAA, 0x00),
                            0x3 => (0x00, 0xAA, 0xAA),
                            0x4 => (0xAA, 0x00, 0x00),
                            0x5 => (0xAA, 0x00, 0xAA),
                            0x6 => (0xAA, 0x55, 0x00),
                            0x7 => (0xAA, 0xAA, 0xAA),
                            0x8 => (0x55, 0x55, 0x55),
                            0x9 => (0x55, 0x55, 0xFF),
                            0xA => (0x55, 0xFF, 0x55),
                            0xB => (0x55, 0xFF, 0xFF),
                            0xC => (0xFF, 0x55, 0x55),
                            0xD => (0xFF, 0x55, 0xFF),
                            0xE => (0xFF, 0xFF, 0x55),
                            _ => (0xFF, 0xFF, 0xFF),
                        }
                    } else {
                        (v, v, v)
                    }
                }
                2 => {
                    let lo = *src.get(sidx).unwrap_or(&0);
                    let hi = *src.get(sidx + 1).unwrap_or(&0);
                    let p = u16::from_le_bytes([lo, hi]);
                    let r = (((p >> 11) & 0x1F) as u32 * 255 / 31) as u8;
                    let g = (((p >> 5) & 0x3F) as u32 * 255 / 63) as u8;
                    let b = (((p) & 0x1F) as u32 * 255 / 31) as u8;
                    (r, g, b)
                }
                3 | 4 => {
                    let b = *src.get(sidx).unwrap_or(&0);
                    let g = *src.get(sidx + 1).unwrap_or(&0);
                    let r = *src.get(sidx + 2).unwrap_or(&0);
                    (r, g, b)
                }
                _ => (0, 0, 0),
            };
            dst[didx] = (b as u32) | ((g as u32) << 8) | ((r as u32) << 16);
        }
    }
}

fn main() {
    let program = env::args().next().unwrap_or_else(|| "test_vmd_x11".to_string());
    let cfg = match parse_args() {
        Ok(c) => c,
        Err(e) => {
            usage(&program);
            if !e.is_empty() {
                eprintln!("error: {e}");
            }
            std::process::exit(2);
        }
    };
    if let Err(e) = ensure_exists(&cfg.bios, "bios") {
        eprintln!("{e}");
        std::process::exit(2);
    }
    if let Err(e) = ensure_exists(&cfg.iso, "iso") {
        eprintln!("{e}");
        std::process::exit(2);
    }

    let bios = fs::read(&cfg.bios).unwrap_or_else(|e| {
        eprintln!("failed to read bios {}: {e}", cfg.bios.display());
        std::process::exit(2);
    });
    let iso = fs::read(&cfg.iso).unwrap_or_else(|e| {
        eprintln!("failed to read iso {}: {e}", cfg.iso.display());
        std::process::exit(2);
    });

    let vm = VmHandle(corevm_create_ex(cfg.ram_mb, cfg.cores));
    if vm.0 == 0 {
        eprintln!("corevm_create_ex failed");
        std::process::exit(1);
    }
    if corevm_load_rom(vm.0, cfg.bios_base, bios.as_ptr(), bios.len() as u32) != 0 {
        eprintln!("corevm_load_rom failed");
        std::process::exit(1);
    }
    corevm_setup_standard_devices(vm.0);
    corevm_setup_pci_bus(vm.0);
    corevm_setup_ide(vm.0);
    corevm_ide_attach_slave(vm.0, iso.as_ptr(), iso.len() as u32);

    unsafe {
        signal(SIGINT, on_sigint as *const () as usize);
    }

    let display = unsafe { XOpenDisplay(std::ptr::null()) };
    if display.is_null() {
        eprintln!("XOpenDisplay failed (DISPLAY not set?)");
        std::process::exit(1);
    }
    let screen = unsafe { XDefaultScreen(display) };
    let root = unsafe { XRootWindow(display, screen) };
    let black = unsafe { XBlackPixel(display, screen) };
    let white = unsafe { XWhitePixel(display, screen) };
    let window = unsafe { XCreateSimpleWindow(display, root, 50, 50, 800, 600, 1, white, black) };
    unsafe {
        XSelectInput(
            display,
            window,
            KEY_PRESS_MASK | KEY_RELEASE_MASK | EXPOSURE_MASK | STRUCTURE_NOTIFY_MASK,
        );
    }
    let title = b"CoreVM X11 Test\0";
    unsafe {
        XStoreName(display, window, title.as_ptr() as *const c_char);
        XMapWindow(display, window);
    }
    let gc = unsafe { XCreateGC(display, window, 0, std::ptr::null_mut()) };
    let visual = unsafe { XDefaultVisual(display, screen) };
    let depth = unsafe { XDefaultDepth(display, screen) };

    let mut win_w: usize = 800;
    let mut win_h: usize = 600;
    let mut blit_buf = vec![0u32; win_w * win_h];
    let mut ximage = unsafe {
        XCreateImage(
            display,
            visual,
            depth as c_uint,
            ZPIXMAP,
            0,
            blit_buf.as_mut_ptr() as *mut c_char,
            win_w as c_uint,
            win_h as c_uint,
            32,
            0,
        )
    };

    let start = Instant::now();
    let mut last_pit_tick = Instant::now();
    let max_duration = Duration::from_secs(cfg.max_seconds);

    while !STOP_REQUESTED.load(Ordering::SeqCst) {
        advance_pit_realtime(vm.0, &mut last_pit_tick);
        let exit_code = corevm_run(vm.0, cfg.batch);

        let text = take_text_output(vm.0);
        if !text.is_empty() {
            print!("{text}");
        }

        while unsafe { XPending(display) } > 0 {
            let mut ev = XEvent { pad: [0; 24] };
            unsafe { XNextEvent(display, &mut ev as *mut XEvent) };
            let any = unsafe { &*(&ev as *const XEvent as *const XAnyEvent) };
            if any.type_ == KEY_PRESS || any.type_ == KEY_RELEASE {
                let kev = unsafe { &mut *(&mut ev as *mut XEvent as *mut XKeyEvent) };
                let ks = unsafe { XLookupKeysym(kev, 0) };
                keysym_to_ps2(vm.0, ks, any.type_ == KEY_PRESS);
            } else if any.type_ == CONFIGURE_NOTIFY {
                let cev = unsafe { &*(&ev as *const XEvent as *const XConfigureEvent) };
                let nw = (cev.width.max(1)) as usize;
                let nh = (cev.height.max(1)) as usize;
                if nw != win_w || nh != win_h {
                    win_w = nw;
                    win_h = nh;
                    blit_buf = vec![0u32; win_w * win_h];
                    ximage = unsafe {
                        XCreateImage(
                            display,
                            visual,
                            depth as c_uint,
                            ZPIXMAP,
                            0,
                            blit_buf.as_mut_ptr() as *mut c_char,
                            win_w as c_uint,
                            win_h as c_uint,
                            32,
                            0,
                        )
                    };
                }
            }
        }

        let mut text_count = 0u32;
        let text_ptr = corevm_vga_get_text_buffer(vm.0, &mut text_count as *mut u32);
        if !text_ptr.is_null() && text_count >= 2000 {
            let cells = unsafe { std::slice::from_raw_parts(text_ptr, text_count as usize) };
            for row in 0..25usize {
                for col in 0..80usize {
                    let cell = cells[row * 80 + col];
                    let ch = (cell & 0x00FF) as u8;
                    let attr = (cell >> 8) as u8;
                    let fg = color16(attr & 0x0F);
                    let bg = color16((attr >> 4) & 0x0F);
                    let x = (col * 8) as c_int;
                    let y = (row * 16) as c_int;
                    unsafe {
                        XSetForeground(display, gc, bg);
                        XFillRectangle(display, window, gc, x, y, 8, 16);
                        if ch.is_ascii_graphic() || ch == b' ' {
                            XSetForeground(display, gc, fg);
                            let txt = [ch as c_char];
                            XDrawString(display, window, gc, x + 1, y + 13, txt.as_ptr(), 1);
                        }
                    }
                }
            }
            unsafe {
                XFlush(display);
            }
        } else {
            let mut fb_w = 0u32;
            let mut fb_h = 0u32;
            let mut fb_bpp = 0u8;
            let fb_ptr = corevm_vga_get_framebuffer(
                vm.0,
                &mut fb_w as *mut u32,
                &mut fb_h as *mut u32,
                &mut fb_bpp as *mut u8,
            );
            if !fb_ptr.is_null() && fb_w > 0 && fb_h > 0 && !ximage.is_null() {
                let spp = (fb_bpp as usize).max(8).div_ceil(8);
                let src_len = (fb_w as usize)
                    .saturating_mul(fb_h as usize)
                    .saturating_mul(spp);
                let src = unsafe { std::slice::from_raw_parts(fb_ptr, src_len) };
                blit_fb_to_bgra(
                    &mut blit_buf,
                    src,
                    fb_w as usize,
                    fb_h as usize,
                    fb_bpp,
                    win_w,
                    win_h,
                );
                unsafe {
                    (*ximage).data = blit_buf.as_mut_ptr() as *mut c_char;
                    XPutImage(
                        display,
                        window,
                        gc,
                        ximage,
                        0,
                        0,
                        0,
                        0,
                        win_w as c_uint,
                        win_h as c_uint,
                    );
                    XFlush(display);
                }
            }
        }

        match exit_code {
            0 => {
                thread::sleep(Duration::from_micros(500));
                advance_pit_realtime(vm.0, &mut last_pit_tick);
            }
            1 => {
                let err = last_error(vm.0);
                let rip = corevm_get_last_error_rip(vm.0);
                eprintln!("\n[test-vmd-x11] exception at rip=0x{rip:X}: {err}");
                break;
            }
            2 => {}
            3 | 4 => break,
            _ => break,
        }

        if start.elapsed() >= max_duration {
            eprintln!("\n[test-vmd-x11] timeout after {} seconds", cfg.max_seconds);
            break;
        }
    }

    eprintln!(
        "\n[test-vmd-x11] stopped, mode={} uptime={}s",
        corevm_get_mode(vm.0),
        start.elapsed().as_secs()
    );
    unsafe {
        XCloseDisplay(display);
    }
}
