//! Minesweeper — Classic mine-sweeping puzzle game for anyOS.
//!
//! 9×9 grid with 10 mines. Left-click to reveal, right-click to flag.
//! First click is always safe. Flood-fill reveals empty regions.

#![no_std]
#![no_main]

use libanyui_client as ui;

anyos_std::entry!(main);

// ── Grid constants ──────────────────────────────────────────────────────────

const COLS: usize = 9;
const ROWS: usize = 9;
const TOTAL: usize = COLS * ROWS;
const MINE_COUNT: u8 = 10;
const CELL: i32 = 28;
const GRID_W: u32 = (COLS as u32) * (CELL as u32);
const GRID_H: u32 = (ROWS as u32) * (CELL as u32);

// ── Colors (dark theme) ─────────────────────────────────────────────────────

const COL_UNREVEALED: u32 = 0xFF3A3A3C;
const COL_UNREVEALED_BORDER: u32 = 0xFF505052;
const COL_REVEALED: u32 = 0xFF2C2C2E;
const COL_REVEALED_BORDER: u32 = 0xFF38383A;
const COL_MINE_BG: u32 = 0xFFFF3B30;
const COL_FLAG: u32 = 0xFFFF9500;
const COL_MINE: u32 = 0xFF1C1C1E;
const COL_WIN_BG: u32 = 0xFF30D158;

/// Number colors: index 0 unused, 1-8 correspond to adjacent mine counts.
const NUM_COLORS: [u32; 9] = [
    0x00000000, // 0 — not drawn
    0xFF5AC8FA, // 1 — blue
    0xFF30D158, // 2 — green
    0xFFFF453A, // 3 — red
    0xFF0A84FF, // 4 — dark blue
    0xFFBF5AF2, // 5 — purple
    0xFF64D2FF, // 6 — cyan
    0xFFFFFFFF, // 7 — white
    0xFFAEAEB2, // 8 — gray
];

// ── Bitmap font for digits 1-8 (5 wide × 7 tall) ───────────────────────────
//
// Each digit is 7 bytes. Each byte is a row; bits 4..0 = pixels left-to-right.
// Rendered at 3× scale → 15×21 px, centered in 28×28 cell.

const DIGIT_BITMAPS: [[u8; 7]; 9] = [
    [0; 7], // 0 — unused
    // 1
    [
        0b00100,
        0b01100,
        0b00100,
        0b00100,
        0b00100,
        0b00100,
        0b01110,
    ],
    // 2
    [
        0b01110,
        0b10001,
        0b00001,
        0b00110,
        0b01000,
        0b10000,
        0b11111,
    ],
    // 3
    [
        0b01110,
        0b10001,
        0b00001,
        0b00110,
        0b00001,
        0b10001,
        0b01110,
    ],
    // 4
    [
        0b00010,
        0b00110,
        0b01010,
        0b10010,
        0b11111,
        0b00010,
        0b00010,
    ],
    // 5
    [
        0b11111,
        0b10000,
        0b11110,
        0b00001,
        0b00001,
        0b10001,
        0b01110,
    ],
    // 6
    [
        0b00110,
        0b01000,
        0b10000,
        0b11110,
        0b10001,
        0b10001,
        0b01110,
    ],
    // 7
    [
        0b11111,
        0b00001,
        0b00010,
        0b00100,
        0b01000,
        0b01000,
        0b01000,
    ],
    // 8
    [
        0b01110,
        0b10001,
        0b10001,
        0b01110,
        0b10001,
        0b10001,
        0b01110,
    ],
];

// ── Game state ──────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum GameState {
    Ready,
    Playing,
    Won,
    Lost,
}

#[derive(Clone, Copy)]
struct Cell {
    is_mine: bool,
    is_revealed: bool,
    is_flagged: bool,
    adjacent: u8,
}

impl Cell {
    const fn new() -> Self {
        Self {
            is_mine: false,
            is_revealed: false,
            is_flagged: false,
            adjacent: 0,
        }
    }
}

struct Game {
    cells: [Cell; TOTAL],
    state: GameState,
    flags_placed: u8,
    start_time: u32,
    elapsed_secs: u32,
    // UI handles
    canvas: ui::Canvas,
    mine_label: ui::Label,
    timer_label: ui::Label,
}

static mut GAME: Option<Game> = None;

fn game() -> &'static mut Game {
    unsafe { GAME.as_mut().unwrap() }
}

// ── Simple LCG random ───────────────────────────────────────────────────────

static mut RNG_STATE: u32 = 0;

fn rng_seed(s: u32) {
    unsafe { RNG_STATE = s; }
}

fn rng_next() -> u32 {
    unsafe {
        RNG_STATE = RNG_STATE.wrapping_mul(1103515245).wrapping_add(12345);
        (RNG_STATE >> 16) & 0x7FFF
    }
}

// ── Main ────────────────────────────────────────────────────────────────────

fn main() {
    if !ui::init() {
        anyos_std::println!("[Minesweeper] Failed to init libanyui");
        return;
    }

    // Seed RNG from uptime
    rng_seed(anyos_std::sys::uptime_ms());

    let win_w = GRID_W;
    let win_h = GRID_H + 44;
    let win = ui::Window::new_with_flags(
        "Minesweeper", -1, -1, win_w, win_h,
        ui::WIN_FLAG_NOT_RESIZABLE,
    );

    // ── Header bar (DOCK_TOP) ─────────────────────────────────────────
    let header = ui::View::new();
    header.set_dock(ui::DOCK_TOP);
    header.set_size(win_w, 44);
    header.set_color(0xFF1C1C1E);
    win.add(&header);

    let mine_label = ui::Label::new("010");
    mine_label.set_position(16, 10);
    mine_label.set_size(60, 24);
    mine_label.set_text_color(0xFFFF453A);
    mine_label.set_font_size(18);
    header.add(&mine_label);

    let new_btn = ui::Button::new("New");
    new_btn.set_position((win_w as i32 / 2) - 30, 8);
    new_btn.set_size(60, 28);
    header.add(&new_btn);

    let timer_label = ui::Label::new("000");
    timer_label.set_position(win_w as i32 - 76, 10);
    timer_label.set_size(60, 24);
    timer_label.set_text_color(0xFF30D158);
    timer_label.set_font_size(18);
    header.add(&timer_label);

    // ── Canvas (DOCK_FILL — fills space below header) ───────────────
    let canvas = ui::Canvas::new(GRID_W, GRID_H);
    canvas.set_dock(ui::DOCK_FILL);
    canvas.set_interactive(true);
    win.add(&canvas);

    // ── Init game ───────────────────────────────────────────────────────
    unsafe {
        GAME = Some(Game {
            cells: [Cell::new(); TOTAL],
            state: GameState::Ready,
            flags_placed: 0,
            start_time: 0,
            elapsed_secs: 0,
            canvas,
            mine_label,
            timer_label,
        });
    }

    game().render();

    // ── Callbacks ───────────────────────────────────────────────────────
    game().canvas.on_mouse_down(|x, y, button| {
        on_grid_click(x, y, button);
    });

    new_btn.on_click(|_| {
        new_game();
    });

    ui::run();
    ui::shutdown();
}

// ── Event handlers ──────────────────────────────────────────────────────────

fn on_grid_click(x: i32, y: i32, button: u32) {
    let g = game();

    // Update timer on every interaction
    if g.state == GameState::Playing {
        let now = anyos_std::sys::uptime_ms();
        g.elapsed_secs = now.wrapping_sub(g.start_time) / 1000;
        update_timer_label();
    }

    if g.state == GameState::Won || g.state == GameState::Lost {
        return;
    }

    let col = (x / CELL) as usize;
    let row = (y / CELL) as usize;
    if col >= COLS || row >= ROWS {
        return;
    }

    if button == 1 {
        // Left click — reveal
        if g.state == GameState::Ready {
            // First click: place mines avoiding this cell
            place_mines(row, col);
            g.state = GameState::Playing;
            g.start_time = anyos_std::sys::uptime_ms();
        }

        let idx = row * COLS + col;
        if !g.cells[idx].is_flagged && !g.cells[idx].is_revealed {
            if g.cells[idx].is_mine {
                // Game over
                g.cells[idx].is_revealed = true;
                g.state = GameState::Lost;
                reveal_all_mines();
            } else {
                flood_reveal(row, col);
                check_win();
            }
        }
    } else if button == 2 {
        // Right click — flag/unflag
        if g.state == GameState::Ready {
            return; // Can't flag before first reveal
        }
        let idx = row * COLS + col;
        if !g.cells[idx].is_revealed {
            g.cells[idx].is_flagged = !g.cells[idx].is_flagged;
            if g.cells[idx].is_flagged {
                g.flags_placed += 1;
            } else {
                g.flags_placed -= 1;
            }
            update_mine_label();
        }
    }

    game().render();
}

fn new_game() {
    let g = game();
    g.cells = [Cell::new(); TOTAL];
    g.state = GameState::Ready;
    g.flags_placed = 0;
    g.start_time = 0;
    g.elapsed_secs = 0;
    update_mine_label();
    update_timer_label();
    g.render();
}

// ── Mine placement ──────────────────────────────────────────────────────────

fn place_mines(safe_row: usize, safe_col: usize) {
    let g = game();
    let safe_idx = safe_row * COLS + safe_col;
    let mut placed = 0u8;

    while placed < MINE_COUNT {
        let idx = (rng_next() as usize) % TOTAL;
        if idx == safe_idx || g.cells[idx].is_mine {
            continue;
        }
        // Also avoid cells immediately adjacent to safe cell
        let r = idx / COLS;
        let c = idx % COLS;
        let dr = if r > safe_row { r - safe_row } else { safe_row - r };
        let dc = if c > safe_col { c - safe_col } else { safe_col - c };
        if dr <= 1 && dc <= 1 {
            continue;
        }
        g.cells[idx].is_mine = true;
        placed += 1;
    }

    // Calculate adjacent counts
    for r in 0..ROWS {
        for c in 0..COLS {
            if g.cells[r * COLS + c].is_mine {
                continue;
            }
            let mut count = 0u8;
            for_neighbors(r, c, |nr, nc| {
                if g.cells[nr * COLS + nc].is_mine {
                    count += 1;
                }
            });
            g.cells[r * COLS + c].adjacent = count;
        }
    }
}

// ── Flood-fill reveal ───────────────────────────────────────────────────────

fn flood_reveal(row: usize, col: usize) {
    let g = game();
    let idx = row * COLS + col;
    if g.cells[idx].is_revealed || g.cells[idx].is_flagged || g.cells[idx].is_mine {
        return;
    }

    g.cells[idx].is_revealed = true;

    // If zero adjacent mines, reveal neighbors recursively
    if g.cells[idx].adjacent == 0 {
        // Use an explicit stack to avoid deep recursion
        let mut stack = anyos_std::Vec::new();
        stack.push((row, col));

        while let Some((r, c)) = stack.pop() {
            for_neighbors(r, c, |nr, nc| {
                let ni = nr * COLS + nc;
                let cell = &mut g.cells[ni];
                if !cell.is_revealed && !cell.is_flagged && !cell.is_mine {
                    cell.is_revealed = true;
                    if cell.adjacent == 0 {
                        stack.push((nr, nc));
                    }
                }
            });
        }
    }
}

// ── Win check ───────────────────────────────────────────────────────────────

fn check_win() {
    let g = game();
    let unrevealed = g.cells.iter().filter(|c| !c.is_revealed).count();
    if unrevealed == MINE_COUNT as usize {
        g.state = GameState::Won;
        // Auto-flag remaining mines
        for i in 0..TOTAL {
            if g.cells[i].is_mine && !g.cells[i].is_flagged {
                g.cells[i].is_flagged = true;
            }
        }
        g.flags_placed = MINE_COUNT;
        update_mine_label();
    }
}

fn reveal_all_mines() {
    let g = game();
    for i in 0..TOTAL {
        if g.cells[i].is_mine {
            g.cells[i].is_revealed = true;
        }
    }
}

// ── Label updates ───────────────────────────────────────────────────────────

fn update_mine_label() {
    let g = game();
    let remaining = (MINE_COUNT as i16) - (g.flags_placed as i16);
    let mut buf = [0u8; 8];
    let s = format_i16(&mut buf, remaining);
    g.mine_label.set_text(s);
}

fn update_timer_label() {
    let g = game();
    let secs = g.elapsed_secs.min(999);
    let mut buf = [0u8; 4];
    buf[0] = b'0' + ((secs / 100) % 10) as u8;
    buf[1] = b'0' + ((secs / 10) % 10) as u8;
    buf[2] = b'0' + (secs % 10) as u8;
    let s = core::str::from_utf8(&buf[..3]).unwrap_or("000");
    g.timer_label.set_text(s);
}

fn format_i16(buf: &mut [u8; 8], val: i16) -> &str {
    let mut pos = 0;
    let abs = if val < 0 {
        buf[0] = b'-';
        pos = 1;
        (-val) as u16
    } else {
        val as u16
    };

    if abs >= 100 {
        buf[pos] = b'0' + ((abs / 100) % 10) as u8;
        pos += 1;
    }
    if abs >= 10 {
        buf[pos] = b'0' + ((abs / 10) % 10) as u8;
        pos += 1;
    }
    buf[pos] = b'0' + (abs % 10) as u8;
    pos += 1;

    core::str::from_utf8(&buf[..pos]).unwrap_or("0")
}

// ── Neighbor iteration ──────────────────────────────────────────────────────

fn for_neighbors(row: usize, col: usize, mut f: impl FnMut(usize, usize)) {
    let r = row as i32;
    let c = col as i32;
    for dr in -1..=1i32 {
        for dc in -1..=1i32 {
            if dr == 0 && dc == 0 {
                continue;
            }
            let nr = r + dr;
            let nc = c + dc;
            if nr >= 0 && nr < ROWS as i32 && nc >= 0 && nc < COLS as i32 {
                f(nr as usize, nc as usize);
            }
        }
    }
}

// ── Rendering ───────────────────────────────────────────────────────────────

impl Game {
    fn render(&self) {
        let canvas = &self.canvas;

        for row in 0..ROWS {
            for col in 0..COLS {
                let cell = &self.cells[row * COLS + col];
                let x = (col as i32) * CELL;
                let y = (row as i32) * CELL;

                if cell.is_revealed {
                    if cell.is_mine {
                        // Mine cell
                        let bg = if self.state == GameState::Lost {
                            COL_MINE_BG
                        } else {
                            COL_WIN_BG
                        };
                        canvas.fill_rect(x, y, CELL as u32, CELL as u32, bg);
                        canvas.draw_rect(x, y, CELL as u32, CELL as u32, COL_REVEALED_BORDER, 1);
                        // Draw mine as filled circle
                        let cx = x + CELL / 2;
                        let cy = y + CELL / 2;
                        canvas.fill_circle(cx, cy, 7, COL_MINE);
                        // Small cross on the mine
                        canvas.draw_line(cx - 4, cy, cx + 4, cy, COL_MINE);
                        canvas.draw_line(cx, cy - 4, cx, cy + 4, COL_MINE);
                    } else {
                        // Revealed empty or numbered cell
                        canvas.fill_rect(x, y, CELL as u32, CELL as u32, COL_REVEALED);
                        canvas.draw_rect(x, y, CELL as u32, CELL as u32, COL_REVEALED_BORDER, 1);

                        if cell.adjacent > 0 && cell.adjacent <= 8 {
                            draw_digit(canvas, x, y, cell.adjacent, NUM_COLORS[cell.adjacent as usize]);
                        }
                    }
                } else if cell.is_flagged {
                    // Flagged cell
                    canvas.fill_rect(x, y, CELL as u32, CELL as u32, COL_UNREVEALED);
                    canvas.draw_rect(x, y, CELL as u32, CELL as u32, COL_UNREVEALED_BORDER, 1);
                    draw_flag(canvas, x, y);
                } else {
                    // Unrevealed cell — raised look
                    canvas.fill_rect(x, y, CELL as u32, CELL as u32, COL_UNREVEALED);
                    canvas.draw_rect(x, y, CELL as u32, CELL as u32, COL_UNREVEALED_BORDER, 1);
                    // Highlight edges for 3D effect
                    canvas.draw_line(x + 1, y + 1, x + CELL - 2, y + 1, 0xFF505052);
                    canvas.draw_line(x + 1, y + 1, x + 1, y + CELL - 2, 0xFF505052);
                }
            }
        }

        // Status overlay for won/lost
        if self.state == GameState::Won {
            // Green tint on header label
            self.mine_label.set_text_color(0xFF30D158);
            self.mine_label.set_text("WIN!");
        } else if self.state == GameState::Lost {
            self.mine_label.set_text_color(0xFFFF453A);
            self.mine_label.set_text("BOOM");
        }
    }
}

/// Draw a bitmap digit (1-8) centered in a cell.
/// Scale = 3× → each bitmap pixel is 3×3 screen pixels.
/// Digit bitmap: 5 wide × 7 tall → 15×21 px on screen.
/// Cell is 28×28: offset = ((28-15)/2, (28-21)/2) = (6, 3).
fn draw_digit(canvas: &ui::Canvas, cell_x: i32, cell_y: i32, digit: u8, color: u32) {
    let bmp = &DIGIT_BITMAPS[digit as usize];
    let scale = 3;
    let ox = cell_x + (CELL - 5 * scale) / 2;
    let oy = cell_y + (CELL - 7 * scale) / 2;

    for (row, &bits) in bmp.iter().enumerate() {
        for bit in 0..5 {
            if bits & (1 << (4 - bit)) != 0 {
                let px = ox + (bit as i32) * scale;
                let py = oy + (row as i32) * scale;
                canvas.fill_rect(px, py, scale as u32, scale as u32, color);
            }
        }
    }
}

/// Draw a flag marker (orange triangle + pole) centered in a cell.
fn draw_flag(canvas: &ui::Canvas, cell_x: i32, cell_y: i32) {
    let cx = cell_x + CELL / 2;
    let cy = cell_y + CELL / 2;

    // Pole (vertical line)
    canvas.draw_line(cx, cy - 8, cx, cy + 6, 0xFFAEAEB2);

    // Flag triangle (orange, pointing right)
    for dy in 0..7 {
        let half = (7 - dy) / 2;
        let y = cy - 7 + dy;
        canvas.draw_line(cx + 1, y, cx + 1 + half, y, COL_FLAG);
    }

    // Base
    canvas.draw_line(cx - 3, cy + 6, cx + 3, cy + 6, 0xFFAEAEB2);
}
