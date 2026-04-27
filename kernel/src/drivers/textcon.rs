//! Kernel text console for the no-GUI (nogui) boot mode.
//!
//! Renders text directly to the VESA/VirtIO framebuffer using the same 8×16
//! bitmap font used by the boot splash / error mode.  Provides a scrollable
//! full-screen terminal surface: characters are written left-to-right, new
//! lines scroll the entire console up by one font row.
//!
//! Used exclusively by `SYS_CON_WRITE` and `SYS_CON_READ`.

use crate::graphics::font::{FONT_HEIGHT, FONT_WIDTH};
use crate::sync::spinlock::Spinlock;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

// ─── Static state ────────────────────────────────────────────────────────────

static READY: AtomicBool = AtomicBool::new(false);

/// Cell width in pixels  = fb_width  / CONSOLE_COLS  (computed at init).
static CELL_W: AtomicU32 = AtomicU32::new(8);
/// Cell height in pixels = fb_height / CONSOLE_ROWS  (computed at init).
static CELL_H: AtomicU32 = AtomicU32::new(16);

/// Target console dimensions.
const CONSOLE_COLS: u32 = 80;
const CONSOLE_ROWS: u32 = 25;

#[inline(always)]
fn cell_w() -> u32 {
    CELL_W.load(Ordering::Relaxed)
}
#[inline(always)]
fn cell_h() -> u32 {
    CELL_H.load(Ordering::Relaxed)
}

// ─── Console mode flags ───────────────────────────────────────────────────────
/// Bit 0: cursor hidden  Bit 1: auto-scroll disabled
static CON_MODE_FLAGS: AtomicU32 = AtomicU32::new(0);

// ─── Scroll-back buffer ───────────────────────────────────────────────────────
//
// We maintain a ring buffer of text-cell rows (character + fg + bg per cell).
// MAX_COLS covers the widest supported mode (mode 3: 160 cols).
// SCROLLBACK_ROWS is the number of off-screen rows kept in history.
// The "live" screen rows are stored in a separate shadow cell buffer so we can
// re-render any viewport position without re-running ANSI sequences.

const MAX_COLS: usize = 160;
/// Number of off-screen history rows available for scrolling back.
const SCROLLBACK_ROWS: usize = 200;
/// Max visible rows covered by the shadow buffer (mode 3: 50 rows max).
const MAX_VISIBLE_ROWS: usize = 50;

/// Character byte for each cell.  0 = space.
static mut SCRBUF_CH: [[u8; MAX_COLS]; SCROLLBACK_ROWS] = [[b' '; MAX_COLS]; SCROLLBACK_ROWS];
/// Foreground color for each cell (ARGB, 0 = COLOR_FG default).
static mut SCRBUF_FG: [[u32; MAX_COLS]; SCROLLBACK_ROWS] = [[0u32; MAX_COLS]; SCROLLBACK_ROWS];
/// Background color for each cell (ARGB, 0 = COLOR_BG default).
static mut SCRBUF_BG: [[u32; MAX_COLS]; SCROLLBACK_ROWS] = [[0u32; MAX_COLS]; SCROLLBACK_ROWS];

/// Shadow buffer for the live visible screen (up to MAX_VISIBLE_ROWS rows).
static mut SHADOW_CH: [[u8; MAX_COLS]; MAX_VISIBLE_ROWS] = [[b' '; MAX_COLS]; MAX_VISIBLE_ROWS];
static mut SHADOW_FG: [[u32; MAX_COLS]; MAX_VISIBLE_ROWS] = [[0u32; MAX_COLS]; MAX_VISIBLE_ROWS];
static mut SHADOW_BG: [[u32; MAX_COLS]; MAX_VISIBLE_ROWS] = [[0u32; MAX_COLS]; MAX_VISIBLE_ROWS];

/// Ring-buffer head: index of the *oldest* row in SCRBUF (next slot to overwrite).
static SCRBUF_HEAD: AtomicU32 = AtomicU32::new(0);
/// Number of valid rows stored in the scroll-back buffer (≤ SCROLLBACK_ROWS).
static SCRBUF_COUNT: AtomicU32 = AtomicU32::new(0);
/// How many rows the viewport is scrolled back from the live view.  0 = live.
static SCROLL_OFFSET: AtomicU32 = AtomicU32::new(0);
static FB_ADDR: AtomicU64 = AtomicU64::new(0);
static FB_PITCH: AtomicU32 = AtomicU32::new(0);
static FB_WIDTH: AtomicU32 = AtomicU32::new(0);
static FB_HEIGHT: AtomicU32 = AtomicU32::new(0);

/// Current cursor column in pixels.
static CUR_X: AtomicU32 = AtomicU32::new(0);
/// Current cursor row in pixels (top of current glyph row).
static CUR_Y: AtomicU32 = AtomicU32::new(0);

/// Whether the cursor block is currently drawn.
static CURSOR_VISIBLE: AtomicBool = AtomicBool::new(false);
/// Blink phase: true = cursor shown, false = cursor hidden. Toggled every BLINK_HALF_PERIOD ticks.
static CURSOR_BLINK_ON: AtomicBool = AtomicBool::new(true);
/// Counts PIT ticks for cursor blink timing.
static BLINK_COUNTER: AtomicU32 = AtomicU32::new(0);
/// Half-period in PIT ticks (1000 Hz → 500 ms on, 500 ms off).
const BLINK_HALF_PERIOD: u32 = 500;

// ─── ANSI escape sequence parser state ───────────────────────────────────────

/// ANSI 8-color palette (normal intensity) — classic VGA/xterm colors.
const ANSI_COLORS: [u32; 8] = [
    0xFF000000, // 0 black
    0xFFAA0000, // 1 red
    0xFF00AA00, // 2 green
    0xFFAA5500, // 3 yellow (dark)
    0xFF0000AA, // 4 blue
    0xFFAA00AA, // 5 magenta
    0xFF00AAAA, // 6 cyan
    0xFFAAAAAA, // 7 white (light gray)
];
/// ANSI 8-color palette (bright/bold intensity) — classic VGA/xterm bright colors.
const ANSI_BRIGHT: [u32; 8] = [
    0xFF555555, // 0 bright black (dark gray)
    0xFFFF5555, // 1 bright red
    0xFF55FF55, // 2 bright green
    0xFFFFFF55, // 3 bright yellow
    0xFF5555FF, // 4 bright blue
    0xFFFF55FF, // 5 bright magenta
    0xFF55FFFF, // 6 bright cyan
    0xFFFFFFFF, // 7 bright white
];

/// Convert a 256-color index to ARGB (compatible with color_256 in Terminal.app).
fn color_256(idx: u32) -> u32 {
    if idx < 8 {
        return ANSI_COLORS[idx as usize];
    }
    if idx < 16 {
        return ANSI_BRIGHT[(idx - 8) as usize];
    }
    if idx < 232 {
        // 6×6×6 color cube
        let i = idx - 16;
        let b = i % 6;
        let g = (i / 6) % 6;
        let r = i / 36;
        fn c(v: u32) -> u32 {
            if v == 0 {
                0
            } else {
                v * 40 + 55
            }
        }
        return 0xFF000000 | (c(r) << 16) | (c(g) << 8) | c(b);
    }
    // 24 grayscale ramp
    let v = 8 + (idx - 232) * 10;
    0xFF000000 | (v << 16) | (v << 8) | v
}

/// SGR attribute flags (bold, underline).
const ATTR_BOLD: u8 = 1;
const ATTR_UNDERLINE: u8 = 4;
const ATTR_REVERSE: u8 = 8;

/// Parser state machine for VT100/ANSI escape sequences.
struct EscState {
    /// 0 = normal, 1 = seen ESC, 2 = seen CSI, 3 = OSC, 4 = DEC private (CSI ?)
    mode: u8,
    /// Accumulated parameter bytes for CSI sequences (e.g. "1;23")
    params: [u8; 64],
    params_len: usize,
    /// Current SGR foreground color (ARGB).  0 = use COLOR_FG default.
    cur_fg: u32,
    /// Current SGR background color (ARGB).  0 = use COLOR_BG default.
    cur_bg: u32,
    /// SGR attribute bitfield.
    attr: u8,
}

impl EscState {
    const fn new() -> Self {
        Self {
            mode: 0,
            params: [0u8; 64],
            params_len: 0,
            cur_fg: 0,
            cur_bg: 0,
            attr: 0,
        }
    }
    fn reset(&mut self) {
        self.mode = 0;
        self.params_len = 0;
    }
    fn fg(&self) -> u32 {
        if self.cur_fg != 0 {
            self.cur_fg
        } else {
            COLOR_FG
        }
    }
    fn bg(&self) -> u32 {
        if self.cur_bg != 0 {
            self.cur_bg
        } else {
            COLOR_BG
        }
    }
}

static ESC: Spinlock<EscState> = Spinlock::new(EscState::new());

// Default foreground / background colors.
const COLOR_FG: u32 = 0xFFAAAAAA; // light gray (matches ANSI white)
const COLOR_BG: u32 = 0xFF000000; // black
const COLOR_CURSOR: u32 = 0xFFAAAAAA;

/// Font data (8×16 bitmap, same as boot_console / error mode).
static FONT_DATA: &[u8] = include_bytes!("../graphics/font_8x16.bin");

// ─── Initialisation ──────────────────────────────────────────────────────────

/// Initialise the textcon from the kernel framebuffer.
/// Must be called after `drivers::framebuffer::init()`.
pub fn init() {
    if let Some(fb) = crate::drivers::framebuffer::info() {
        FB_ADDR.store(fb.addr, Ordering::Relaxed);
        FB_PITCH.store(fb.pitch, Ordering::Relaxed);
        FB_WIDTH.store(fb.width, Ordering::Relaxed);
        FB_HEIGHT.store(fb.height, Ordering::Relaxed);

        // Compute cell size so we always get exactly CONSOLE_COLS × CONSOLE_ROWS cells,
        // regardless of framebuffer resolution.  No integer-scale restriction.
        let cw = (fb.width / CONSOLE_COLS).max(FONT_WIDTH);
        let ch = (fb.height / CONSOLE_ROWS).max(FONT_HEIGHT);
        CELL_W.store(cw, Ordering::Relaxed);
        CELL_H.store(ch, Ordering::Relaxed);

        let cols = fb.width / cw;
        let rows = fb.height / ch;
        crate::serial_println!(
            "[OK] textcon: {}x{} fb, cell {}x{}, console {}x{}",
            fb.width,
            fb.height,
            cw,
            ch,
            cols,
            rows
        );

        CUR_X.store(0, Ordering::Relaxed);
        CUR_Y.store(0, Ordering::Relaxed);
        clear_screen_bg();
        READY.store(true, Ordering::Relaxed);
    } else {
        crate::serial_println!("[WARN] textcon: no framebuffer, falling back to VGA text mode");
    }
}

pub fn is_ready() -> bool {
    READY.load(Ordering::Relaxed)
}

// ─── Internal pixel helpers ──────────────────────────────────────────────────

#[inline(always)]
fn put_pixel(x: u32, y: u32, color: u32) {
    let addr = FB_ADDR.load(Ordering::Relaxed) as *mut u32;
    let pitch = FB_PITCH.load(Ordering::Relaxed) as usize / 4; // pitch in pixels
    let w = FB_WIDTH.load(Ordering::Relaxed) as usize;
    let h = FB_HEIGHT.load(Ordering::Relaxed) as usize;
    if (x as usize) < w && (y as usize) < h {
        unsafe {
            *addr.add(y as usize * pitch + x as usize) = color;
        }
    }
}

fn fill_rect(x: u32, y: u32, w: u32, h: u32, color: u32) {
    for row in 0..h {
        for col in 0..w {
            put_pixel(x + col, y + row, color);
        }
    }
}

/// Copy a block of rows upward by `rows` glyph rows.
fn scroll_up(rows: u32) {
    // Save displaced rows into scroll-back buffer before pixel-shifting
    for _ in 0..rows {
        shadow_scroll_up();
    }
    let addr = FB_ADDR.load(Ordering::Relaxed) as *mut u32;
    let pitch = FB_PITCH.load(Ordering::Relaxed) as usize / 4;
    let w = FB_WIDTH.load(Ordering::Relaxed) as usize;
    let h = FB_HEIGHT.load(Ordering::Relaxed) as usize;
    let dy = (rows * cell_h()) as usize;
    if dy >= h {
        return;
    }
    let copy_rows = h - dy;
    // memmove rows upward
    for row in 0..copy_rows {
        let src_base = (row + dy) * pitch;
        let dst_base = row * pitch;
        for col in 0..w {
            unsafe {
                let v = *addr.add(src_base + col);
                *addr.add(dst_base + col) = v;
            }
        }
    }
    // blank the newly exposed rows at the bottom
    for row in copy_rows..h {
        for col in 0..w {
            unsafe {
                *addr.add(row * pitch + col) = COLOR_BG;
            }
        }
    }
}

fn clear_screen_bg() {
    let addr = FB_ADDR.load(Ordering::Relaxed) as *mut u32;
    let pitch = FB_PITCH.load(Ordering::Relaxed) as usize / 4;
    let w = FB_WIDTH.load(Ordering::Relaxed) as usize;
    let h = FB_HEIGHT.load(Ordering::Relaxed) as usize;
    for row in 0..h {
        for col in 0..w {
            unsafe {
                *addr.add(row * pitch + col) = COLOR_BG;
            }
        }
    }
    // Also reset shadow buffer and scroll state
    shadow_clear_all();
    SCRBUF_HEAD.store(0, Ordering::Relaxed);
    SCRBUF_COUNT.store(0, Ordering::Relaxed);
    SCROLL_OFFSET.store(0, Ordering::Relaxed);
}

fn flush_rect(x: u32, y: u32, w: u32, h: u32) {
    if let Some(_fb) = crate::drivers::framebuffer::info() {
        crate::drivers::gpu::with_gpu(|g| {
            g.transfer_rect(x, y, w, h);
            g.flush_display(x, y, w, h);
        });
    }
}

/// IRQ-safe flush: uses try_lock so it never blocks or yields.
/// Called exclusively from tick_blink() (PIT interrupt context).
/// If the GPU mutex is held by another thread, the blink simply
/// skips this cycle — the cursor will update on the next tick.
fn flush_rect_irq(x: u32, y: u32, w: u32, h: u32) {
    if crate::drivers::framebuffer::info().is_none() {
        return;
    }
    if let Some(mut gpu_guard) = crate::drivers::gpu::try_lock_gpu() {
        if let Some(g) = gpu_guard.as_mut() {
            g.transfer_rect(x, y, w, h);
            g.flush_display(x, y, w, h);
        }
    }
    // None = GPU locked by another thread; skip this blink cycle silently.
}

// ─── Shadow cell buffer helpers ──────────────────────────────────────────────

/// Number of visible rows given current cell height.
#[inline]
fn vis_rows() -> usize {
    let h = FB_HEIGHT.load(Ordering::Relaxed);
    let ch = cell_h();
    if ch == 0 {
        return CONSOLE_ROWS as usize;
    }
    (h / ch).min(MAX_VISIBLE_ROWS as u32) as usize
}

/// Number of visible cols given current cell width.
#[inline]
fn vis_cols() -> usize {
    let w = FB_WIDTH.load(Ordering::Relaxed);
    let cw = cell_w();
    if cw == 0 {
        return CONSOLE_COLS as usize;
    }
    (w / cw).min(MAX_COLS as u32) as usize
}

/// Write a cell into the shadow buffer (live screen mirror).
#[inline]
fn shadow_set(row: usize, col: usize, ch: u8, fg: u32, bg: u32) {
    if row < MAX_VISIBLE_ROWS && col < MAX_COLS {
        unsafe {
            SHADOW_CH[row][col] = ch;
            SHADOW_FG[row][col] = fg;
            SHADOW_BG[row][col] = bg;
        }
    }
}

/// Clear the entire shadow buffer (used on screen clear / resize).
fn shadow_clear_all() {
    unsafe {
        for r in 0..MAX_VISIBLE_ROWS {
            for c in 0..MAX_COLS {
                SHADOW_CH[r][c] = b' ';
                SHADOW_FG[r][c] = 0;
                SHADOW_BG[r][c] = 0;
            }
        }
    }
}

/// Erase a range of cells in the shadow buffer (for EL/ED erase operations).
/// `row` is the cell row, `col_start..col_end` is the column range.
fn shadow_erase_cells(row: usize, col_start: usize, col_end: usize) {
    if row >= MAX_VISIBLE_ROWS {
        return;
    }
    let end = col_end.min(MAX_COLS);
    unsafe {
        for c in col_start..end {
            SHADOW_CH[row][c] = b' ';
            SHADOW_FG[row][c] = 0;
            SHADOW_BG[row][c] = 0;
        }
    }
}

/// Erase all cells from (row, 0) to end of screen in shadow buffer.
fn shadow_erase_from_row(start_row: usize) {
    let rows = vis_rows();
    let cols = vis_cols();
    unsafe {
        for r in start_row..rows.min(MAX_VISIBLE_ROWS) {
            for c in 0..cols {
                SHADOW_CH[r][c] = b' ';
                SHADOW_FG[r][c] = 0;
                SHADOW_BG[r][c] = 0;
            }
        }
    }
}

/// Erase all cells up to (row, col) from start of screen in shadow buffer.
fn shadow_erase_to_row(end_row: usize, end_col: usize) {
    unsafe {
        for r in 0..end_row.min(MAX_VISIBLE_ROWS) {
            let cols = vis_cols();
            for c in 0..cols {
                SHADOW_CH[r][c] = b' ';
                SHADOW_FG[r][c] = 0;
                SHADOW_BG[r][c] = 0;
            }
        }
        if end_row < MAX_VISIBLE_ROWS {
            for c in 0..end_col.min(MAX_COLS) {
                SHADOW_CH[end_row][c] = b' ';
                SHADOW_FG[end_row][c] = 0;
                SHADOW_BG[end_row][c] = 0;
            }
        }
    }
}

/// Push the top shadow row (row 0) into the scroll-back ring buffer,
/// then shift all shadow rows up by one.
fn shadow_scroll_up() {
    let cols = vis_cols();
    // Save row 0 into ring buffer
    let head = SCRBUF_HEAD.load(Ordering::Relaxed) as usize;
    unsafe {
        SCRBUF_CH[head][..cols].copy_from_slice(&SHADOW_CH[0][..cols]);
        SCRBUF_FG[head][..cols].copy_from_slice(&SHADOW_FG[0][..cols]);
        SCRBUF_BG[head][..cols].copy_from_slice(&SHADOW_BG[0][..cols]);
    }
    let next_head = (head + 1) % SCROLLBACK_ROWS;
    SCRBUF_HEAD.store(next_head as u32, Ordering::Relaxed);
    let count = SCRBUF_COUNT.load(Ordering::Relaxed) as usize;
    if count < SCROLLBACK_ROWS {
        SCRBUF_COUNT.store((count + 1) as u32, Ordering::Relaxed);
    }
    // Shift shadow rows up by one
    let rows = vis_rows();
    unsafe {
        for r in 0..rows.saturating_sub(1) {
            SHADOW_CH[r][..cols].copy_from_slice(&SHADOW_CH[r + 1][..cols]);
            SHADOW_FG[r][..cols].copy_from_slice(&SHADOW_FG[r + 1][..cols]);
            SHADOW_BG[r][..cols].copy_from_slice(&SHADOW_BG[r + 1][..cols]);
        }
        // Clear last row
        if rows > 0 {
            let last = rows - 1;
            for c in 0..cols {
                SHADOW_CH[last][c] = b' ';
                SHADOW_FG[last][c] = 0;
                SHADOW_BG[last][c] = 0;
            }
        }
    }
}

/// Render a single row from either the scroll-back buffer or the shadow buffer
/// onto the framebuffer at vertical pixel position `y_px`.
/// `row_idx` is the logical row index from the top of the current viewport.
/// When `offset > 0`, row 0 of the viewport maps to a row in the scroll-back buffer.
/// Scroll the viewport up (delta > 0) or down (delta < 0) by `|delta|` rows.
/// Called from `sys_con_poll_key` when Shift+Up / Shift+Down is pressed.
pub fn scroll_viewport(delta: i32) {
    if !is_ready() {
        return;
    }
    let count = SCRBUF_COUNT.load(Ordering::Relaxed) as i32;
    let rows = vis_rows() as i32;
    let max_offset = (count).min(SCROLLBACK_ROWS as i32 - rows).max(0);
    let cur = SCROLL_OFFSET.load(Ordering::Relaxed) as i32;
    let new_offset = (cur + delta).clamp(0, max_offset);
    if new_offset == cur {
        return;
    }
    SCROLL_OFFSET.store(new_offset as u32, Ordering::Relaxed);
    repaint_viewport();
}

/// Re-render the entire visible screen from shadow+scrollback at current offset.
fn repaint_viewport() {
    let offset = SCROLL_OFFSET.load(Ordering::Relaxed) as usize;
    let rows = vis_rows();
    let count = SCRBUF_COUNT.load(Ordering::Relaxed) as usize;
    let ch = cell_h();
    let fb_w = FB_WIDTH.load(Ordering::Relaxed);
    let fb_h = FB_HEIGHT.load(Ordering::Relaxed);

    // Each visible row:
    //   viewport row 0 → oldest in scrollback shown = scrbuf[(head - offset) % SCROLLBACK_ROWS]
    //   viewport row k  →
    //     if k < offset and k < count: from scroll-back
    //     else:                        from shadow[k - offset]
    let head = SCRBUF_HEAD.load(Ordering::Relaxed) as usize;

    for r in 0..rows {
        let y_px = r as u32 * ch;
        if r < offset && r < count {
            // from scroll-back ring buffer
            // oldest visible = (head - offset) mod SCROLLBACK_ROWS
            let scr_idx = (head + SCROLLBACK_ROWS - offset + r) % SCROLLBACK_ROWS;
            let cols = vis_cols();
            for col in 0..cols {
                let (cell_ch, fg, bg) = unsafe {
                    (
                        SCRBUF_CH[scr_idx][col],
                        SCRBUF_FG[scr_idx][col],
                        SCRBUF_BG[scr_idx][col],
                    )
                };
                let fg = if fg != 0 { fg } else { COLOR_FG };
                let bg = if bg != 0 { bg } else { COLOR_BG };
                let x_px = col as u32 * cell_w();
                draw_glyph_pixels(x_px, y_px, cell_ch, fg, bg);
            }
        } else {
            // from live shadow buffer
            let shadow_row = r.saturating_sub(offset);
            let cols = vis_cols();
            if shadow_row < rows {
                for col in 0..cols {
                    let (cell_ch, fg, bg) = unsafe {
                        (
                            SHADOW_CH[shadow_row][col],
                            SHADOW_FG[shadow_row][col],
                            SHADOW_BG[shadow_row][col],
                        )
                    };
                    let fg = if fg != 0 { fg } else { COLOR_FG };
                    let bg = if bg != 0 { bg } else { COLOR_BG };
                    let x_px = col as u32 * cell_w();
                    draw_glyph_pixels(x_px, y_px, cell_ch, fg, bg);
                }
                // blank any cols beyond vis_cols
                let cw = cell_w();
                let used_w = cols as u32 * cw;
                if used_w < fb_w {
                    fill_rect(used_w, y_px, fb_w - used_w, ch, COLOR_BG);
                }
            } else {
                fill_rect(0, y_px, fb_w, ch, COLOR_BG);
            }
        }
    }
    flush_rect(0, 0, fb_w, fb_h);
}

// ─── Cursor ──────────────────────────────────────────────────────────────────
// Cursor state is managed inside write_str — no separate flush per cursor op.

// ─── Text output ─────────────────────────────────────────────────────────────

/// Render glyph pixels only — no shadow update.  Used during viewport repaint.
fn draw_glyph_pixels(cx: u32, cy: u32, ch: u8, fg: u32, bg: u32) {
    let cw = cell_w();
    let ch_px = cell_h();
    let c = ch as u32;
    let idx = if c >= 32 && c <= 126 {
        (c - 32) as usize
    } else {
        0
    };
    let glyph_offset = idx * FONT_HEIGHT as usize;
    for py in 0..ch_px {
        let src_row = (py * FONT_HEIGHT) / ch_px;
        let bits = if glyph_offset + (src_row as usize) < FONT_DATA.len() {
            FONT_DATA[glyph_offset + src_row as usize]
        } else {
            0
        };
        for px in 0..cw {
            let src_col = (px * FONT_WIDTH) / cw;
            let color = if bits & (0x80 >> src_col) != 0 {
                fg
            } else {
                bg
            };
            put_pixel(cx + px, cy + py, color);
        }
    }
}

fn draw_glyph(cx: u32, cy: u32, ch: u8, fg: u32, bg: u32) {
    // Update shadow cell buffer (for scroll-back).
    let cw = cell_w();
    let ch_px = cell_h();
    if cw > 0 && ch_px > 0 {
        let col = (cx / cw) as usize;
        let row = (cy / ch_px) as usize;
        shadow_set(row, col, ch, fg, bg);
    }
    draw_glyph_pixels(cx, cy, ch, fg, bg);
}

fn advance_cursor() {
    let w = FB_WIDTH.load(Ordering::Relaxed);
    let cw = cell_w();
    let mut cx = CUR_X.load(Ordering::Relaxed) + cw;
    let mut cy = CUR_Y.load(Ordering::Relaxed);
    if cx + cw > w {
        cx = 0;
        newline_cursor(&mut cx, &mut cy);
    }
    CUR_X.store(cx, Ordering::Relaxed);
    CUR_Y.store(cy, Ordering::Relaxed);
}

/// Returns true if a scroll happened (caller must flush full screen).
fn newline_cursor(cx: &mut u32, cy: &mut u32) -> bool {
    let h = FB_HEIGHT.load(Ordering::Relaxed);
    let ch = cell_h();
    *cx = 0;
    *cy += ch;
    if *cy + ch > h {
        // Only scroll if auto-scroll is enabled (bit 1 clear)
        let mode = CON_MODE_FLAGS.load(Ordering::Relaxed);
        if mode & 0x02 == 0 {
            scroll_up(1);
            *cy = h - ch;
            return true;
        } else {
            *cy = h - ch;
            return false;
        }
    }
    false
}

/// Parse two semicolon-separated numbers from a CSI parameter buffer.
/// Returns (p1, p2) with defaults for missing values.
fn parse_csi_params(params: &[u8], def1: u32, def2: u32) -> (u32, u32) {
    let s = core::str::from_utf8(params).unwrap_or("");
    let mut it = s.split(';');
    let p1 = it
        .next()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(def1);
    let p2 = it
        .next()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(def2);
    (p1, p2)
}

fn parse_csi_single(params: &[u8], default: u32) -> u32 {
    let s = core::str::from_utf8(params).unwrap_or("");
    s.parse::<u32>().unwrap_or(default)
}

/// Execute a complete CSI sequence: ESC [ <params> <final_byte>
/// Returns true if a full-screen operation occurred (scroll/clear).
fn execute_csi(params: &[u8], final_byte: u8) -> bool {
    let fb_w = FB_WIDTH.load(Ordering::Relaxed);
    let fb_h = FB_HEIGHT.load(Ordering::Relaxed);
    let cw = cell_w();
    let ch = cell_h();
    let cols = fb_w / cw;
    let rows = fb_h / ch;

    match final_byte {
        // CUP — cursor position: ESC [ row ; col H  (1-based)
        b'H' | b'f' => {
            let (row, col) = parse_csi_params(params, 1, 1);
            let col = col.saturating_sub(1).min(cols.saturating_sub(1));
            let row = row.saturating_sub(1).min(rows.saturating_sub(1));
            CUR_X.store(col * cw, Ordering::Relaxed);
            CUR_Y.store(row * ch, Ordering::Relaxed);
            false
        }
        // CUU — cursor up N
        b'A' => {
            let n = parse_csi_single(params, 1);
            let cy = CUR_Y.load(Ordering::Relaxed);
            CUR_Y.store(cy.saturating_sub(n * ch), Ordering::Relaxed);
            false
        }
        // CUD — cursor down N
        b'B' => {
            let n = parse_csi_single(params, 1);
            let cy = CUR_Y.load(Ordering::Relaxed);
            CUR_Y.store(
                (cy + n * ch).min(fb_h.saturating_sub(ch)),
                Ordering::Relaxed,
            );
            false
        }
        // CUF — cursor forward (right) N
        b'C' => {
            let n = parse_csi_single(params, 1);
            let cx = CUR_X.load(Ordering::Relaxed);
            CUR_X.store(
                (cx + n * cw).min(fb_w.saturating_sub(cw)),
                Ordering::Relaxed,
            );
            false
        }
        // CUB — cursor back (left) N
        b'D' => {
            let n = parse_csi_single(params, 1);
            let cx = CUR_X.load(Ordering::Relaxed);
            CUR_X.store(cx.saturating_sub(n * cw), Ordering::Relaxed);
            false
        }
        // ED — erase display
        b'J' => {
            let n = parse_csi_single(params, 0);
            let erase_bg = ESC.lock().bg();
            match n {
                0 => {
                    // Erase from cursor to end of screen
                    let cx = CUR_X.load(Ordering::Relaxed);
                    let cy = CUR_Y.load(Ordering::Relaxed);
                    fill_rect(cx, cy, fb_w - cx, ch, erase_bg);
                    if cy + ch < fb_h {
                        fill_rect(0, cy + ch, fb_w, fb_h - cy - ch, erase_bg);
                    }
                    let row = (cy / ch) as usize;
                    let col = (cx / cw) as usize;
                    shadow_erase_cells(row, col, vis_cols());
                    shadow_erase_from_row(row + 1);
                }
                1 => {
                    // Erase from start to cursor
                    let cx = CUR_X.load(Ordering::Relaxed);
                    let cy = CUR_Y.load(Ordering::Relaxed);
                    if cy > 0 {
                        fill_rect(0, 0, fb_w, cy, erase_bg);
                    }
                    fill_rect(0, cy, cx + cw, ch, erase_bg);
                    let row = (cy / ch) as usize;
                    let col = ((cx + cw) / cw) as usize;
                    shadow_erase_to_row(row, col);
                }
                _ => {
                    // Erase entire display (ESC[2J)
                    fill_rect(0, 0, fb_w, fb_h, erase_bg);
                    shadow_clear_all();
                    // Note: cursor stays (apps send ESC[H separately to home)
                }
            }
            true
        }
        // EL — erase line
        b'K' => {
            let n = parse_csi_single(params, 0);
            let cx = CUR_X.load(Ordering::Relaxed);
            let cy = CUR_Y.load(Ordering::Relaxed);
            let erase_bg = ESC.lock().bg();
            let row = (cy / ch) as usize;
            match n {
                0 => {
                    fill_rect(cx, cy, fb_w - cx, ch, erase_bg);
                    shadow_erase_cells(row, (cx / cw) as usize, vis_cols());
                }
                1 => {
                    fill_rect(0, cy, cx + cw, ch, erase_bg);
                    shadow_erase_cells(row, 0, ((cx + cw) / cw) as usize);
                }
                _ => {
                    fill_rect(0, cy, fb_w, ch, erase_bg);
                    shadow_erase_cells(row, 0, vis_cols());
                }
            }
            false
        }
        // SGR — select graphic rendition: full color + bold support
        b'm' => {
            let s = core::str::from_utf8(params).unwrap_or("");
            // Parse semicolon-separated numbers
            let mut nums = [0u32; 16];
            let mut num_count = 0usize;
            if s.is_empty() {
                nums[0] = 0;
                num_count = 1;
            } else {
                for part in s.split(';') {
                    if num_count >= 16 {
                        break;
                    }
                    nums[num_count] = part.parse::<u32>().unwrap_or(0);
                    num_count += 1;
                }
            }

            let mut esc = ESC.lock();
            let mut idx = 0usize;
            while idx < num_count {
                match nums[idx] {
                    0 => {
                        esc.cur_fg = 0;
                        esc.cur_bg = 0;
                        esc.attr = 0;
                    }
                    1 => {
                        esc.attr |= ATTR_BOLD;
                    }
                    4 => {
                        esc.attr |= ATTR_UNDERLINE;
                    }
                    7 => {
                        esc.attr |= ATTR_REVERSE;
                    }
                    22 => {
                        esc.attr &= !ATTR_BOLD;
                    }
                    24 => {
                        esc.attr &= !ATTR_UNDERLINE;
                    }
                    27 => {
                        esc.attr &= !ATTR_REVERSE;
                    }
                    30..=37 => {
                        let c = (nums[idx] - 30) as usize;
                        esc.cur_fg = if esc.attr & ATTR_BOLD != 0 {
                            ANSI_BRIGHT[c]
                        } else {
                            ANSI_COLORS[c]
                        };
                    }
                    38 => {
                        if idx + 1 < num_count {
                            if nums[idx + 1] == 5 && idx + 2 < num_count {
                                esc.cur_fg = color_256(nums[idx + 2]);
                                idx += 2;
                            } else if nums[idx + 1] == 2 && idx + 4 < num_count {
                                let r = nums[idx + 2].min(255) as u8;
                                let g = nums[idx + 3].min(255) as u8;
                                let b = nums[idx + 4].min(255) as u8;
                                esc.cur_fg =
                                    0xFF000000 | (r as u32) << 16 | (g as u32) << 8 | b as u32;
                                idx += 4;
                            }
                        }
                    }
                    39 => {
                        esc.cur_fg = 0;
                    }
                    40..=47 => {
                        let c = (nums[idx] - 40) as usize;
                        esc.cur_bg = ANSI_COLORS[c];
                    }
                    48 => {
                        if idx + 1 < num_count {
                            if nums[idx + 1] == 5 && idx + 2 < num_count {
                                esc.cur_bg = color_256(nums[idx + 2]);
                                idx += 2;
                            } else if nums[idx + 1] == 2 && idx + 4 < num_count {
                                let r = nums[idx + 2].min(255) as u8;
                                let g = nums[idx + 3].min(255) as u8;
                                let b = nums[idx + 4].min(255) as u8;
                                esc.cur_bg =
                                    0xFF000000 | (r as u32) << 16 | (g as u32) << 8 | b as u32;
                                idx += 4;
                            }
                        }
                    }
                    49 => {
                        esc.cur_bg = 0;
                    }
                    90..=97 => {
                        let c = (nums[idx] - 90) as usize;
                        esc.cur_fg = ANSI_BRIGHT[c];
                    }
                    100..=107 => {
                        let c = (nums[idx] - 100) as usize;
                        esc.cur_bg = ANSI_BRIGHT[c];
                    }
                    _ => {}
                }
                idx += 1;
            }
            false
        }
        // Show/hide cursor: ESC[?25h / ESC[?25l — accept silently
        b'h' | b'l' => false,
        // SU — scroll up N lines
        b'S' => {
            let n = parse_csi_single(params, 1);
            scroll_up(n);
            true
        }
        _ => false,
    }
}

/// Write a single character into the framebuffer only — NO GPU flush.
/// Routes through ANSI escape sequence parser.
/// Returns true if a full-screen operation occurred (flush must cover whole screen).
fn write_char_raw(ch: u8) -> bool {
    // Fast path: normal printable ASCII when not in escape mode
    {
        let esc = ESC.lock();
        if esc.mode == 0 && ch >= 32 && ch != 127 {
            let cx = CUR_X.load(Ordering::Relaxed);
            let cy = CUR_Y.load(Ordering::Relaxed);
            let (fg, bg) = if esc.attr & ATTR_REVERSE != 0 {
                (esc.bg(), esc.fg())
            } else {
                (esc.fg(), esc.bg())
            };
            drop(esc);
            draw_glyph(cx, cy, ch, fg, bg);
            advance_cursor();
            return false;
        }
    }

    let mut esc = ESC.lock();

    match esc.mode {
        // Normal mode
        0 => match ch {
            0x1B => {
                esc.mode = 1; // ESC received
                false
            }
            b'\n' => {
                drop(esc);
                let mut cx = 0u32;
                let mut cy = CUR_Y.load(Ordering::Relaxed);
                let scrolled = newline_cursor(&mut cx, &mut cy);
                CUR_X.store(cx, Ordering::Relaxed);
                CUR_Y.store(cy, Ordering::Relaxed);
                scrolled
            }
            b'\r' => {
                CUR_X.store(0, Ordering::Relaxed);
                false
            }
            b'\x08' => {
                let cw = cell_w();
                let cx = CUR_X.load(Ordering::Relaxed);
                if cx >= cw {
                    let new_cx = cx - cw;
                    CUR_X.store(new_cx, Ordering::Relaxed);
                    let cy = CUR_Y.load(Ordering::Relaxed);
                    fill_rect(new_cx, cy, cw, cell_h(), COLOR_BG);
                }
                false
            }
            b'\t' => {
                // Tab: advance to next 8-column boundary
                let cw = cell_w();
                let cx = CUR_X.load(Ordering::Relaxed);
                let col = cx / cw;
                let next_col = ((col + 8) / 8) * 8;
                let fb_w = FB_WIDTH.load(Ordering::Relaxed);
                CUR_X.store((next_col * cw).min(fb_w - cw), Ordering::Relaxed);
                false
            }
            _ => false, // other control chars: ignore
        },

        // ESC seen — next byte decides sequence type
        1 => {
            match ch {
                b'[' => {
                    esc.mode = 2;
                    esc.params_len = 0;
                    false
                } // CSI
                b']' => {
                    esc.mode = 3;
                    esc.params_len = 0;
                    false
                } // OSC
                b'c' => {
                    // RIS — full reset: clear screen, home cursor
                    esc.reset();
                    drop(esc);
                    let fb_w = FB_WIDTH.load(Ordering::Relaxed);
                    let fb_h = FB_HEIGHT.load(Ordering::Relaxed);
                    fill_rect(0, 0, fb_w, fb_h, COLOR_BG);
                    CUR_X.store(0, Ordering::Relaxed);
                    CUR_Y.store(0, Ordering::Relaxed);
                    true
                }
                _ => {
                    esc.reset();
                    false
                } // unknown: ignore
            }
        }

        // CSI — accumulating parameters
        2 => {
            if ch >= 0x20 && ch < 0x40 {
                // Parameter / intermediate byte
                let idx = esc.params_len;
                if idx < esc.params.len() {
                    esc.params[idx] = ch;
                    esc.params_len = idx + 1;
                }
                false
            } else if ch >= 0x40 {
                // Final byte — execute
                let params_len = esc.params_len;
                let params_copy = esc.params;
                esc.reset();
                drop(esc);
                execute_csi(&params_copy[..params_len], ch)
            } else {
                // Control char inside CSI: abort
                esc.reset();
                false
            }
        }

        // OSC — consume until ST (ESC \) or BEL
        3 => {
            if ch == 0x07 || ch == 0x1B {
                esc.reset();
            }
            false
        }

        _ => {
            esc.reset();
            false
        }
    }
}

// ─── Public API ──────────────────────────────────────────────────────────────

/// Write a string to the framebuffer console.
/// Renders all characters first, then does a single GPU flush covering the
/// entire dirty region — no per-character flush.
pub fn write_str(s: &str) {
    if !is_ready() {
        crate::drivers::vga_text::put_str(s);
        return;
    }

    // If user was scrolled back, return to live view before writing new output.
    if SCROLL_OFFSET.load(Ordering::Relaxed) > 0 {
        SCROLL_OFFSET.store(0, Ordering::Relaxed);
        repaint_viewport();
    }

    let fb_w = FB_WIDTH.load(Ordering::Relaxed);
    let fb_h = FB_HEIGHT.load(Ordering::Relaxed);

    // Track the bounding box of pixels written so we can flush once at the end.
    // If a scroll happens, we must flush the entire screen.
    let mut dirty_x0 = CUR_X.load(Ordering::Relaxed);
    let mut dirty_y0 = CUR_Y.load(Ordering::Relaxed);
    let cw = cell_w();
    let ch = cell_h();
    let mut dirty_x1 = dirty_x0;
    let mut dirty_y1 = dirty_y0 + ch;
    let mut full_flush = false;

    // Hide cursor without flushing (we'll flush once at the end)
    let cursor_was_visible = CURSOR_VISIBLE.load(Ordering::Relaxed);
    if cursor_was_visible {
        // Erase cursor block in memory only (no flush yet)
        let cx = CUR_X.load(Ordering::Relaxed);
        let cy = CUR_Y.load(Ordering::Relaxed);
        let y = cy + ch - 2;
        for col in 0..cw {
            put_pixel(cx + col, y, COLOR_BG);
        }
        for col in 0..cw {
            put_pixel(cx + col, y + 1, COLOR_BG);
        }
        CURSOR_VISIBLE.store(false, Ordering::Relaxed);
    }

    for b in s.bytes() {
        // Expand dirty rect to include current cursor position before writing
        let cx = CUR_X.load(Ordering::Relaxed);
        let cy = CUR_Y.load(Ordering::Relaxed);
        if cx < dirty_x0 {
            dirty_x0 = cx;
        }
        if cy < dirty_y0 {
            dirty_y0 = cy;
        }
        if cx + cw > dirty_x1 {
            dirty_x1 = cx + cw;
        }
        if cy + ch > dirty_y1 {
            dirty_y1 = cy + ch;
        }

        if write_char_raw(b) {
            full_flush = true;
        }
    }

    // Expand dirty rect to also include new cursor position
    let cx = CUR_X.load(Ordering::Relaxed);
    let cy = CUR_Y.load(Ordering::Relaxed);
    if cx + cw > dirty_x1 {
        dirty_x1 = cx + cw;
    }
    if cy + ch > dirty_y1 {
        dirty_y1 = cy + ch;
    }

    // Redraw cursor in memory only if not hidden by mode flags.
    // Also reset blink phase so cursor stays solid for a full period after each write.
    if CON_MODE_FLAGS.load(Ordering::Relaxed) & 0x01 == 0 {
        let y = cy + ch - 2;
        for col in 0..cw {
            put_pixel(cx + col, y, COLOR_CURSOR);
        }
        for col in 0..cw {
            put_pixel(cx + col, y + 1, COLOR_CURSOR);
        }
        CURSOR_VISIBLE.store(true, Ordering::Relaxed);
        CURSOR_BLINK_ON.store(true, Ordering::Relaxed);
        BLINK_COUNTER.store(0, Ordering::Relaxed);
    }

    // Single GPU flush for the entire dirty region
    if full_flush {
        flush_rect(0, 0, fb_w, fb_h);
    } else {
        let fw = (dirty_x1).min(fb_w) - dirty_x0.min(fb_w);
        let fh = (dirty_y1).min(fb_h) - dirty_y0.min(fb_h);
        if fw > 0 && fh > 0 {
            flush_rect(dirty_x0, dirty_y0, fw, fh);
        }
    }
}

/// Return console size as `cols << 16 | rows` (both derived from FB + font).
/// Returns 0 if not yet initialised.
pub fn get_size_packed() -> u32 {
    if !is_ready() {
        return 0;
    }
    let cols = FB_WIDTH.load(Ordering::Relaxed) / cell_w();
    let rows = FB_HEIGHT.load(Ordering::Relaxed) / cell_h();
    (cols << 16) | rows
}

/// Resize the console to exactly `cols` columns and `rows` rows by recomputing
/// CELL_W / CELL_H from the current framebuffer dimensions.
/// Clears the screen and homes the cursor.  Returns new packed size, or 0 if not ready.
pub fn resize(cols: u32, rows: u32) -> u32 {
    if !is_ready() {
        return 0;
    }
    if cols == 0 || rows == 0 {
        return 0;
    }
    let fb_w = FB_WIDTH.load(Ordering::Relaxed);
    let fb_h = FB_HEIGHT.load(Ordering::Relaxed);
    let cw = (fb_w / cols).max(FONT_WIDTH);
    let ch = (fb_h / rows).max(FONT_HEIGHT);
    CELL_W.store(cw, Ordering::Relaxed);
    CELL_H.store(ch, Ordering::Relaxed);
    // Clear screen, home cursor, reset scroll-back state
    fill_rect(0, 0, fb_w, fb_h, COLOR_BG);
    shadow_clear_all();
    SCRBUF_HEAD.store(0, Ordering::Relaxed);
    SCRBUF_COUNT.store(0, Ordering::Relaxed);
    SCROLL_OFFSET.store(0, Ordering::Relaxed);
    CUR_X.store(0, Ordering::Relaxed);
    CUR_Y.store(0, Ordering::Relaxed);
    CURSOR_VISIBLE.store(false, Ordering::Relaxed);
    flush_rect(0, 0, fb_w, fb_h);
    get_size_packed()
}

/// Set console mode flags. Returns previous flags.
/// Bit 0 (0x01): 1 = hide cursor, 0 = show cursor.
/// Bit 1 (0x02): 1 = disable auto-scroll, 0 = enable auto-scroll.
pub fn set_mode(flags: u32) -> u32 {
    let prev = CON_MODE_FLAGS.swap(flags, Ordering::Relaxed);
    if !is_ready() {
        return prev;
    }

    // Any mode change (cursor visibility or scroll enable/disable) clears the
    // scroll-back buffer and resets the viewport — the screen context has changed.
    if flags != prev {
        shadow_clear_all();
        SCRBUF_HEAD.store(0, Ordering::Relaxed);
        SCRBUF_COUNT.store(0, Ordering::Relaxed);
        SCROLL_OFFSET.store(0, Ordering::Relaxed);
    }

    let cursor_now_hidden = flags & 0x01 != 0;
    let cursor_was_hidden = prev & 0x01 != 0;
    if cursor_now_hidden && !cursor_was_hidden {
        // Hide cursor immediately
        let cx = CUR_X.load(Ordering::Relaxed);
        let cy = CUR_Y.load(Ordering::Relaxed);
        if CURSOR_VISIBLE.load(Ordering::Relaxed) {
            let cw = cell_w();
            let ch = cell_h();
            let y = cy + ch - 2;
            for col in 0..cw {
                put_pixel(cx + col, y, COLOR_BG);
            }
            for col in 0..cw {
                put_pixel(cx + col, y + 1, COLOR_BG);
            }
            CURSOR_VISIBLE.store(false, Ordering::Relaxed);
            let fb_w = FB_WIDTH.load(Ordering::Relaxed);
            let fb_h = FB_HEIGHT.load(Ordering::Relaxed);
            flush_rect(cx, cy, cw.min(fb_w - cx), ch.min(fb_h - cy));
        }
    }
    prev
}

/// Return current console mode flags.
pub fn get_mode() -> u32 {
    CON_MODE_FLAGS.load(Ordering::Relaxed)
}

/// Called from the PIT IRQ handler every tick (1000 Hz).
/// Toggles the cursor block every BLINK_HALF_PERIOD ticks (500 ms).
/// No-op if the console is not ready, cursor is mode-hidden, or viewport is scrolled back.
pub fn tick_blink() {
    if !is_ready() {
        return;
    }
    // Don't blink when cursor is mode-hidden (bit 0) or when scrolled back.
    if CON_MODE_FLAGS.load(Ordering::Relaxed) & 0x01 != 0 {
        return;
    }
    if SCROLL_OFFSET.load(Ordering::Relaxed) > 0 {
        return;
    }

    let counter = BLINK_COUNTER.fetch_add(1, Ordering::Relaxed) + 1;
    if counter < BLINK_HALF_PERIOD {
        return;
    }
    BLINK_COUNTER.store(0, Ordering::Relaxed);

    let blink_on = !CURSOR_BLINK_ON.load(Ordering::Relaxed);
    CURSOR_BLINK_ON.store(blink_on, Ordering::Relaxed);

    let cx = CUR_X.load(Ordering::Relaxed);
    let cy = CUR_Y.load(Ordering::Relaxed);
    let cw = cell_w();
    let ch = cell_h();
    let fb_w = FB_WIDTH.load(Ordering::Relaxed);
    let fb_h = FB_HEIGHT.load(Ordering::Relaxed);
    if cw == 0 || ch == 0 || cx >= fb_w || cy + ch > fb_h {
        return;
    }

    // Draw or erase the cursor underline (bottom 2 pixel rows of the cell).
    let y = cy + ch - 2;
    if blink_on {
        for col in 0..cw {
            put_pixel(cx + col, y, COLOR_CURSOR);
        }
        for col in 0..cw {
            put_pixel(cx + col, y + 1, COLOR_CURSOR);
        }
        CURSOR_VISIBLE.store(true, Ordering::Relaxed);
    } else {
        // Restore cell background — read from shadow buffer.
        let col_idx = (cx / cw) as usize;
        let row_idx = (cy / ch) as usize;
        let bg = if row_idx < MAX_VISIBLE_ROWS && col_idx < MAX_COLS {
            let raw = unsafe { SHADOW_BG[row_idx][col_idx] };
            if raw != 0 {
                raw
            } else {
                COLOR_BG
            }
        } else {
            COLOR_BG
        };
        for col in 0..cw {
            put_pixel(cx + col, y, bg);
        }
        for col in 0..cw {
            put_pixel(cx + col, y + 1, bg);
        }
        CURSOR_VISIBLE.store(false, Ordering::Relaxed);
    }

    // Flush just the cursor cell — IRQ-safe (non-blocking try_lock).
    let flush_w = cw.min(fb_w.saturating_sub(cx));
    let flush_h = ch.min(fb_h.saturating_sub(cy));
    if flush_w > 0 && flush_h > 0 {
        flush_rect_irq(cx, cy, flush_w, flush_h);
    }
}

/// Write a single character with a single GPU flush.
pub fn write_char(ch: u8) {
    if !is_ready() {
        crate::drivers::vga_text::put_char(ch);
        return;
    }
    // For a single char just reuse write_str logic via a 1-byte slice
    let buf = [ch];
    if let Ok(s) = core::str::from_utf8(&buf) {
        write_str(s);
    }
}

/// Read a line of text from the keyboard with echo, storing it in `buf`.
/// Blocks until Enter is pressed.  Returns the number of bytes written
/// (not including the trailing '\n').
/// Password mode: if `echo` is false, typed characters are not echoed.
pub fn read_line(buf: &mut [u8], echo: bool) -> usize {
    let mut len = 0usize;
    loop {
        // Busy-wait / yield until a key event is available
        while !crate::drivers::input::keyboard::has_event() {
            crate::arch::hal::halt();
        }
        let evt = match crate::drivers::input::keyboard::read_event() {
            Some(e) => e,
            None => continue,
        };
        // Only handle key-down events
        if !evt.pressed {
            continue;
        }

        use crate::drivers::input::keyboard::Key;
        match evt.key {
            Key::Enter => {
                write_char(b'\n');
                break;
            }
            Key::Backspace => {
                if len > 0 {
                    len -= 1;
                    write_char(b'\x08');
                }
            }
            Key::Char(c) if c as u32 >= 32 && (c as u32) < 127 => {
                if len < buf.len().saturating_sub(1) {
                    buf[len] = c as u8;
                    len += 1;
                    if echo {
                        write_char(c as u8);
                    } else {
                        write_char(b'*');
                    }
                }
            }
            Key::Space => {
                if len < buf.len().saturating_sub(1) {
                    buf[len] = b' ';
                    len += 1;
                    if echo {
                        write_char(b' ');
                    } else {
                        write_char(b'*');
                    }
                }
            }
            _ => {}
        }
    }
    buf[len] = 0;
    len
}
