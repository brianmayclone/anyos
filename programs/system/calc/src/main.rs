#![no_std]
#![no_main]

use anyos_std::ui::window;
use anyos_std::String;

anyos_std::entry!(main);

// ---- Layout ----
const WIN_W: u16 = 240;
const WIN_H: u16 = 340;
const PAD: i16 = 8;
const DISPLAY_H: i16 = 76;
const GRID_Y: i16 = PAD + DISPLAY_H + 4;
const BTN_W: i16 = 53;
const BTN_H: i16 = 44;
const GAP: i16 = 4;
const BTN_R: u16 = 10;

// ---- Colors (macOS dark calculator) ----
const BG: u32 = 0xFF1C1C1E;
const COL_NUM: u32 = 0xFF333333;
const COL_NUM_P: u32 = 0xFF505050;
const COL_FN: u32 = 0xFFA5A5A5;
const COL_FN_P: u32 = 0xFFC0C0C0;
const COL_OP: u32 = 0xFFFF9500;
const COL_OP_P: u32 = 0xFFFFB040;
const COL_OP_ACT: u32 = 0xFFFFFFFF;
const TXT_WHITE: u32 = 0xFFFFFFFF;
const TXT_BLACK: u32 = 0xFF000000;
const TXT_OP_ACT: u32 = 0xFFFF9500;

// ---- Font ----
const FONT: u16 = 0;
const DISP_SZ: u16 = 36;
const BTN_SZ: u16 = 18;

// ---- Button definitions ----
#[derive(Clone, Copy, PartialEq)]
enum BKind { Num, Fn, Op }

struct Btn {
    label: &'static str,
    col: i16,
    row: i16,
    span: i16,
    kind: BKind,
}

const BUTTONS: [Btn; 19] = [
    Btn { label: "AC",  col: 0, row: 0, span: 1, kind: BKind::Fn },
    Btn { label: "+/-", col: 1, row: 0, span: 1, kind: BKind::Fn },
    Btn { label: "%",   col: 2, row: 0, span: 1, kind: BKind::Fn },
    Btn { label: "/",   col: 3, row: 0, span: 1, kind: BKind::Op },
    Btn { label: "7",   col: 0, row: 1, span: 1, kind: BKind::Num },
    Btn { label: "8",   col: 1, row: 1, span: 1, kind: BKind::Num },
    Btn { label: "9",   col: 2, row: 1, span: 1, kind: BKind::Num },
    Btn { label: "x",   col: 3, row: 1, span: 1, kind: BKind::Op },
    Btn { label: "4",   col: 0, row: 2, span: 1, kind: BKind::Num },
    Btn { label: "5",   col: 1, row: 2, span: 1, kind: BKind::Num },
    Btn { label: "6",   col: 2, row: 2, span: 1, kind: BKind::Num },
    Btn { label: "-",   col: 3, row: 2, span: 1, kind: BKind::Op },
    Btn { label: "1",   col: 0, row: 3, span: 1, kind: BKind::Num },
    Btn { label: "2",   col: 1, row: 3, span: 1, kind: BKind::Num },
    Btn { label: "3",   col: 2, row: 3, span: 1, kind: BKind::Num },
    Btn { label: "+",   col: 3, row: 3, span: 1, kind: BKind::Op },
    Btn { label: "0",   col: 0, row: 4, span: 2, kind: BKind::Num },
    Btn { label: ".",   col: 2, row: 4, span: 1, kind: BKind::Num },
    Btn { label: "=",   col: 3, row: 4, span: 1, kind: BKind::Op },
];

// Operator enum
#[derive(Clone, Copy, PartialEq)]
enum Op { Add, Sub, Mul, Div }

struct Calc {
    display: String,
    acc: f64,
    pending: Option<Op>,
    new_input: bool,
    active_op: Option<Op>,
}

impl Calc {
    fn new() -> Self {
        Calc {
            display: String::from("0"),
            acc: 0.0,
            pending: None,
            new_input: true,
            active_op: None,
        }
    }

    fn display_val(&self) -> f64 {
        parse_f64(&self.display)
    }

    fn evaluate(&mut self) {
        if let Some(op) = self.pending.take() {
            let rhs = self.display_val();
            let result = match op {
                Op::Add => self.acc + rhs,
                Op::Sub => self.acc - rhs,
                Op::Mul => self.acc * rhs,
                Op::Div => {
                    if rhs == 0.0 { f64::INFINITY } else { self.acc / rhs }
                }
            };
            self.acc = result;
            self.display = fmt_f64(result);
            self.new_input = true;
        }
    }

    fn press_digit(&mut self, d: u8) {
        self.active_op_clear_highlight();
        if self.new_input {
            self.display.clear();
            if d == 0 {
                self.display.push('0');
            }
            self.new_input = false;
        }
        // Max 12 digits
        let digits: usize = self.display.bytes().filter(|b| b.is_ascii_digit()).count();
        if digits >= 12 { return; }
        if self.display == "0" && d != 0 {
            self.display.clear();
        }
        self.display.push((b'0' + d) as char);
    }

    fn press_decimal(&mut self) {
        self.active_op_clear_highlight();
        if self.new_input {
            self.display = String::from("0.");
            self.new_input = false;
            return;
        }
        if !self.display.contains('.') {
            self.display.push('.');
        }
    }

    fn press_op(&mut self, op: Op) {
        if !self.new_input && self.pending.is_some() {
            self.evaluate();
        } else if self.new_input && self.pending.is_some() {
            // Just change the operator
        } else {
            self.acc = self.display_val();
        }
        self.pending = Some(op);
        self.active_op = Some(op);
        self.new_input = true;
    }

    fn press_equals(&mut self) {
        self.active_op = None;
        self.evaluate();
    }

    fn press_clear(&mut self) {
        if self.display == "0" || self.new_input {
            // AC: clear everything
            self.acc = 0.0;
            self.pending = None;
            self.active_op = None;
        }
        self.display = String::from("0");
        self.new_input = true;
    }

    fn press_negate(&mut self) {
        if self.display == "0" { return; }
        if self.display.starts_with('-') {
            self.display = String::from(&self.display[1..]);
        } else {
            let mut s = String::from("-");
            s.push_str(&self.display);
            self.display = s;
        }
    }

    fn press_percent(&mut self) {
        let val = self.display_val() / 100.0;
        self.display = fmt_f64(val);
        self.new_input = true;
    }

    fn active_op_clear_highlight(&mut self) {
        self.active_op = None;
    }

    fn clear_label(&self) -> &str {
        if self.display == "0" || self.new_input { "AC" } else { "C" }
    }
}

// ---- Number formatting ----
fn fmt_f64(val: f64) -> String {
    if val != val { return String::from("Error"); }
    if val == f64::INFINITY || val == f64::NEG_INFINITY {
        return String::from("Error");
    }
    if val == 0.0 { return String::from("0"); }

    let neg = val < 0.0;
    let v = if neg { -val } else { val };

    // Integer check
    let int_part = v as u64;
    let frac = v - int_part as f64;
    if frac.abs() < 1e-9 && int_part < 1_000_000_000_000 {
        let mut s = fmt_u64(int_part);
        if neg { s.insert(0, '-'); }
        return s;
    }

    // Decimal formatting
    let mut s = String::new();
    if neg { s.push('-'); }
    s.push_str(&fmt_u64(int_part));
    s.push('.');

    let mut f = frac;
    let mut dec = [0u8; 10];
    for d in dec.iter_mut() {
        f *= 10.0;
        let digit = f as u8;
        *d = digit;
        f -= digit as f64;
    }
    // Strip trailing zeros
    let mut last = 0;
    for i in 0..10 {
        if dec[i] != 0 { last = i; }
    }
    for i in 0..=last {
        s.push((b'0' + dec[i]) as char);
    }
    s
}

fn fmt_u64(mut val: u64) -> String {
    if val == 0 { return String::from("0"); }
    let mut buf = [0u8; 20];
    let mut i = 20;
    while val > 0 {
        i -= 1;
        buf[i] = b'0' + (val % 10) as u8;
        val /= 10;
    }
    let mut s = String::new();
    for &b in &buf[i..] {
        s.push(b as char);
    }
    s
}

fn parse_f64(s: &str) -> f64 {
    let s = s.trim();
    if s.is_empty() || s == "Error" { return 0.0; }
    let neg = s.starts_with('-');
    let s = if neg { &s[1..] } else { s };

    let mut int_part: u64 = 0;
    let mut frac_part: f64 = 0.0;
    let mut in_frac = false;
    let mut frac_div: f64 = 1.0;

    for b in s.bytes() {
        if b == b'.' {
            in_frac = true;
            continue;
        }
        if b < b'0' || b > b'9' { continue; }
        let d = (b - b'0') as u64;
        if in_frac {
            frac_div *= 10.0;
            frac_part += d as f64 / frac_div;
        } else {
            int_part = int_part * 10 + d;
        }
    }
    let val = int_part as f64 + frac_part;
    if neg { -val } else { val }
}

// ---- Button geometry ----
fn btn_rect(idx: usize) -> (i16, i16, i16, i16) {
    let b = &BUTTONS[idx];
    let x = PAD + b.col * (BTN_W + GAP);
    let y = GRID_Y + b.row * (BTN_H + GAP);
    let w = b.span * BTN_W + (b.span - 1) * GAP;
    (x, y, w, BTN_H)
}

fn hit_test(mx: i16, my: i16) -> Option<usize> {
    for i in 0..BUTTONS.len() {
        let (bx, by, bw, bh) = btn_rect(i);
        if mx >= bx && mx < bx + bw && my >= by && my < by + bh {
            return Some(i);
        }
    }
    None
}

// ---- Rendering ----
fn render(win: u32, calc: &Calc, pressed: Option<usize>) {
    let w = WIN_W as i16;

    // Background
    window::fill_rect(win, 0, 0, WIN_W, WIN_H, BG);

    // Display text (right-aligned)
    let disp = &calc.display;
    // Determine font size based on text length
    let sz = if disp.len() > 10 { 24 } else { DISP_SZ };
    let (tw, _th) = window::font_measure(FONT, sz, disp);
    let tx = w - PAD - tw as i16 - 4;
    let ty = PAD + DISPLAY_H - sz as i16 - 8;
    window::draw_text_ex(win, tx, ty, TXT_WHITE, FONT, sz, disp);

    // Buttons
    for i in 0..BUTTONS.len() {
        let b = &BUTTONS[i];
        let (bx, by, bw, bh) = btn_rect(i);
        let is_pressed = pressed == Some(i);

        // Check if this operator is the active one
        let is_active_op = match (b.kind, calc.active_op) {
            (BKind::Op, Some(Op::Div)) if b.label == "/" => true,
            (BKind::Op, Some(Op::Mul)) if b.label == "x" => true,
            (BKind::Op, Some(Op::Sub)) if b.label == "-" => true,
            (BKind::Op, Some(Op::Add)) if b.label == "+" => true,
            _ => false,
        };

        let bg_color = if is_active_op {
            COL_OP_ACT
        } else {
            match b.kind {
                BKind::Num => if is_pressed { COL_NUM_P } else { COL_NUM },
                BKind::Fn  => if is_pressed { COL_FN_P } else { COL_FN },
                BKind::Op  => if is_pressed { COL_OP_P } else { COL_OP },
            }
        };

        let text_color = if is_active_op {
            TXT_OP_ACT
        } else {
            match b.kind {
                BKind::Fn => TXT_BLACK,
                _ => TXT_WHITE,
            }
        };

        window::fill_rounded_rect(win, bx, by, bw as u16, bh as u16, BTN_R, bg_color);

        // Button label
        let label = if i == 0 { calc.clear_label() } else { b.label };
        let (tw, th) = window::font_measure(FONT, BTN_SZ, label);
        let tx = bx + (bw - tw as i16) / 2;
        let ty = by + (bh - th as i16) / 2;
        window::draw_text_ex(win, tx, ty, text_color, FONT, BTN_SZ, label);
    }
}

// ---- Input handling ----
fn handle_button(calc: &mut Calc, idx: usize) {
    let label = BUTTONS[idx].label;
    match label {
        "0" => calc.press_digit(0),
        "1" => calc.press_digit(1),
        "2" => calc.press_digit(2),
        "3" => calc.press_digit(3),
        "4" => calc.press_digit(4),
        "5" => calc.press_digit(5),
        "6" => calc.press_digit(6),
        "7" => calc.press_digit(7),
        "8" => calc.press_digit(8),
        "9" => calc.press_digit(9),
        "." => calc.press_decimal(),
        "+" => calc.press_op(Op::Add),
        "-" => calc.press_op(Op::Sub),
        "x" => calc.press_op(Op::Mul),
        "/" => calc.press_op(Op::Div),
        "=" => calc.press_equals(),
        "AC" => calc.press_clear(),
        "+/-" => calc.press_negate(),
        "%" => calc.press_percent(),
        _ => {}
    }
}

fn handle_key(calc: &mut Calc, key: u32) {
    match key {
        k if k == b'0' as u32 => calc.press_digit(0),
        k if k == b'1' as u32 => calc.press_digit(1),
        k if k == b'2' as u32 => calc.press_digit(2),
        k if k == b'3' as u32 => calc.press_digit(3),
        k if k == b'4' as u32 => calc.press_digit(4),
        k if k == b'5' as u32 => calc.press_digit(5),
        k if k == b'6' as u32 => calc.press_digit(6),
        k if k == b'7' as u32 => calc.press_digit(7),
        k if k == b'8' as u32 => calc.press_digit(8),
        k if k == b'9' as u32 => calc.press_digit(9),
        k if k == b'.' as u32 => calc.press_decimal(),
        k if k == b'+' as u32 => calc.press_op(Op::Add),
        k if k == b'-' as u32 => calc.press_op(Op::Sub),
        k if k == b'*' as u32 => calc.press_op(Op::Mul),
        k if k == b'/' as u32 => calc.press_op(Op::Div),
        k if k == b'=' as u32 || k == 0x0A => calc.press_equals(), // Enter = 0x0A
        27 => calc.press_clear(), // Escape
        8 => { // Backspace
            if !calc.new_input && calc.display.len() > 1 {
                calc.display.pop();
            } else {
                calc.display = String::from("0");
                calc.new_input = true;
            }
        }
        _ => {}
    }
}

// ---- Main ----
fn main() {
    let win = window::create_ex(
        "Calculator", 200, 100, WIN_W, WIN_H,
        window::WIN_FLAG_NOT_RESIZABLE,
    );
    if win == u32::MAX { return; }

    // Menu
    let mut mb = window::MenuBarBuilder::new()
        .menu("Calculator")
            .item(100, "About Calculator", 0)
            .separator()
            .item(101, "Quit Calculator", 0)
        .end_menu();
    window::set_menu(win, mb.build());

    let mut calc = Calc::new();
    let mut pressed: Option<usize> = None;
    let mut event = [0u32; 5];

    render(win, &calc, None);
    window::present(win);

    loop {
        let mut dirty = false;
        while window::get_event(win, &mut event) == 1 {
            match event[0] {
                window::EVENT_MOUSE_DOWN => {
                    let mx = event[1] as i16;
                    let my = event[2] as i16;
                    pressed = hit_test(mx, my);
                    if let Some(idx) = pressed {
                        handle_button(&mut calc, idx);
                    }
                    dirty = true;
                }
                window::EVENT_MOUSE_UP => {
                    pressed = None;
                    dirty = true;
                }
                window::EVENT_KEY_DOWN => {
                    handle_key(&mut calc, event[1]);
                    dirty = true;
                }
                window::EVENT_WINDOW_CLOSE | window::EVENT_MENU_ITEM
                    if event[0] == window::EVENT_WINDOW_CLOSE
                        || event[1] == 101
                        || event[1] == window::APP_MENU_QUIT =>
                {
                    window::destroy(win);
                    return;
                }
                _ => {}
            }
        }

        if dirty {
            render(win, &calc, pressed);
            window::present(win);
        }
        anyos_std::process::yield_cpu();
    }
}
