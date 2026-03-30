#![no_std]
#![no_main]

use anyos_std::String;
use libanyui_client as ui;
use ui::Widget;

anyos_std::entry!(main);

// ---- Layout ----
const WIN_W: u32 = 240;
const WIN_H: u32 = 340;
const PAD: i32 = 8;
const DISPLAY_H: u32 = 76;
const GRID_Y: i32 = PAD + DISPLAY_H as i32 + 4;
const BTN_W: u32 = 53;
const BTN_H: u32 = 44;
const GAP: i32 = 4;

// ---- Colors (macOS dark calculator) ----
const BG: u32 = 0xFF1C1C1E;
const COL_NUM: u32 = 0xFF333333;
const COL_FN: u32 = 0xFFA5A5A5;
const COL_OP: u32 = 0xFFFF9500;
const COL_OP_ACT: u32 = 0xFFFFFFFF;
const TXT_WHITE: u32 = 0xFFFFFFFF;
const TXT_BLACK: u32 = 0xFF000000;
const TXT_OP_ACT: u32 = 0xFFFF9500;

// ---- Button definitions ----
#[derive(Clone, Copy, PartialEq)]
enum BKind { Num, Fn, Op }

struct BtnDef {
    label: &'static str,
    col: i32,
    row: i32,
    span: i32,
    kind: BKind,
}

const BUTTONS: [BtnDef; 19] = [
    BtnDef { label: "AC",  col: 0, row: 0, span: 1, kind: BKind::Fn },
    BtnDef { label: "+/-", col: 1, row: 0, span: 1, kind: BKind::Fn },
    BtnDef { label: "%",   col: 2, row: 0, span: 1, kind: BKind::Fn },
    BtnDef { label: "/",   col: 3, row: 0, span: 1, kind: BKind::Op },
    BtnDef { label: "7",   col: 0, row: 1, span: 1, kind: BKind::Num },
    BtnDef { label: "8",   col: 1, row: 1, span: 1, kind: BKind::Num },
    BtnDef { label: "9",   col: 2, row: 1, span: 1, kind: BKind::Num },
    BtnDef { label: "x",   col: 3, row: 1, span: 1, kind: BKind::Op },
    BtnDef { label: "4",   col: 0, row: 2, span: 1, kind: BKind::Num },
    BtnDef { label: "5",   col: 1, row: 2, span: 1, kind: BKind::Num },
    BtnDef { label: "6",   col: 2, row: 2, span: 1, kind: BKind::Num },
    BtnDef { label: "-",   col: 3, row: 2, span: 1, kind: BKind::Op },
    BtnDef { label: "1",   col: 0, row: 3, span: 1, kind: BKind::Num },
    BtnDef { label: "2",   col: 1, row: 3, span: 1, kind: BKind::Num },
    BtnDef { label: "3",   col: 2, row: 3, span: 1, kind: BKind::Num },
    BtnDef { label: "+",   col: 3, row: 3, span: 1, kind: BKind::Op },
    BtnDef { label: "0",   col: 0, row: 4, span: 2, kind: BKind::Num },
    BtnDef { label: ".",   col: 2, row: 4, span: 1, kind: BKind::Num },
    BtnDef { label: "=",   col: 3, row: 4, span: 1, kind: BKind::Op },
];

// ---- Operator enum ----
#[derive(Clone, Copy, PartialEq)]
enum Op { Add, Sub, Mul, Div }

// ---- Calculator state ----
struct AppState {
    display: String,
    acc: f64,
    pending: Option<Op>,
    new_input: bool,
    active_op: Option<Op>,
    display_label: ui::Label,
    btns: [ui::Button; 19],
}

anyos_std::global_app_state!(AppState);

// ---- Number formatting ----
fn fmt_f64(val: f64) -> String {
    if val != val || val == f64::INFINITY || val == f64::NEG_INFINITY {
        return String::from("Error");
    }
    anyos_std::fmt::fmt_f64(val)
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
        if b == b'.' { in_frac = true; continue; }
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

// ---- Calculator logic ----
fn display_val() -> f64 {
    parse_f64(&app().display)
}

fn update_display() {
    let a = app();
    let sz = if a.display.len() > 10 { 24 } else { 36 };
    a.display_label.set_text(&a.display);
    a.display_label.set_font_size(sz);
    // Update AC/C label
    let clear_label = if a.display == "0" || a.new_input { "AC" } else { "C" };
    a.btns[0].set_text(clear_label);
}

fn update_op_buttons() {
    let a = app();
    // Op button indices: / = 3, x = 7, - = 11, + = 15, = = 18
    let op_indices: [(usize, Op); 4] = [
        (3, Op::Div), (7, Op::Mul), (11, Op::Sub), (15, Op::Add),
    ];
    for &(idx, op) in &op_indices {
        let is_active = a.active_op == Some(op);
        let bg = if is_active { COL_OP_ACT } else { COL_OP };
        let fg = if is_active { TXT_OP_ACT } else { TXT_WHITE };
        a.btns[idx].set_color(bg);
        a.btns[idx].set_text_color(fg);
    }
}

fn evaluate() {
    let a = app();
    if let Some(op) = a.pending.take() {
        let rhs = parse_f64(&a.display);
        let result = match op {
            Op::Add => a.acc + rhs,
            Op::Sub => a.acc - rhs,
            Op::Mul => a.acc * rhs,
            Op::Div => if rhs == 0.0 { f64::INFINITY } else { a.acc / rhs },
        };
        a.acc = result;
        a.display = fmt_f64(result);
        a.new_input = true;
    }
}

fn press_digit(d: u8) {
    let a = app();
    a.active_op = None;
    update_op_buttons();
    if a.new_input {
        a.display.clear();
        if d == 0 { a.display.push('0'); }
        a.new_input = false;
    }
    let digits: usize = a.display.bytes().filter(|b| b.is_ascii_digit()).count();
    if digits >= 12 { return; }
    if a.display == "0" && d != 0 {
        a.display.clear();
    }
    a.display.push((b'0' + d) as char);
    update_display();
}

fn press_decimal() {
    let a = app();
    a.active_op = None;
    update_op_buttons();
    if a.new_input {
        a.display = String::from("0.");
        a.new_input = false;
        update_display();
        return;
    }
    if !a.display.contains('.') {
        a.display.push('.');
    }
    update_display();
}

fn press_op(op: Op) {
    let a = app();
    if !a.new_input && a.pending.is_some() {
        evaluate();
    } else if a.new_input && a.pending.is_some() {
        // Just change the operator
    } else {
        a.acc = display_val();
    }
    let a = app();
    a.pending = Some(op);
    a.active_op = Some(op);
    a.new_input = true;
    update_op_buttons();
    update_display();
}

fn press_equals() {
    app().active_op = None;
    evaluate();
    update_op_buttons();
    update_display();
}

fn press_clear() {
    let a = app();
    if a.display == "0" || a.new_input {
        a.acc = 0.0;
        a.pending = None;
        a.active_op = None;
    }
    a.display = String::from("0");
    a.new_input = true;
    update_op_buttons();
    update_display();
}

fn press_negate() {
    let a = app();
    if a.display == "0" { return; }
    if a.display.starts_with('-') {
        a.display = String::from(&a.display[1..]);
    } else {
        let mut s = String::from("-");
        s.push_str(&a.display);
        a.display = s;
    }
    update_display();
}

fn press_percent() {
    let val = display_val() / 100.0;
    app().display = fmt_f64(val);
    app().new_input = true;
    update_display();
}

fn handle_button(idx: usize) {
    match BUTTONS[idx].label {
        "0" => press_digit(0),
        "1" => press_digit(1),
        "2" => press_digit(2),
        "3" => press_digit(3),
        "4" => press_digit(4),
        "5" => press_digit(5),
        "6" => press_digit(6),
        "7" => press_digit(7),
        "8" => press_digit(8),
        "9" => press_digit(9),
        "." => press_decimal(),
        "+" => press_op(Op::Add),
        "-" => press_op(Op::Sub),
        "x" => press_op(Op::Mul),
        "/" => press_op(Op::Div),
        "=" => press_equals(),
        "AC" => press_clear(),
        "+/-" => press_negate(),
        "%" => press_percent(),
        _ => {}
    }
}

fn handle_key(ke: &ui::KeyEvent) {
    match ke.char_code {
        c if c == b'0' as u32 => press_digit(0),
        c if c == b'1' as u32 => press_digit(1),
        c if c == b'2' as u32 => press_digit(2),
        c if c == b'3' as u32 => press_digit(3),
        c if c == b'4' as u32 => press_digit(4),
        c if c == b'5' as u32 => press_digit(5),
        c if c == b'6' as u32 => press_digit(6),
        c if c == b'7' as u32 => press_digit(7),
        c if c == b'8' as u32 => press_digit(8),
        c if c == b'9' as u32 => press_digit(9),
        c if c == b'.' as u32 => press_decimal(),
        c if c == b'+' as u32 => press_op(Op::Add),
        c if c == b'-' as u32 => press_op(Op::Sub),
        c if c == b'*' as u32 => press_op(Op::Mul),
        c if c == b'/' as u32 => press_op(Op::Div),
        c if c == b'=' as u32 => press_equals(),
        _ => {
            match ke.keycode {
                ui::KEY_ENTER => press_equals(),
                ui::KEY_ESCAPE => press_clear(),
                ui::KEY_BACKSPACE => {
                    let a = app();
                    if !a.new_input && a.display.len() > 1 {
                        a.display.pop();
                    } else {
                        a.display = String::from("0");
                        a.new_input = true;
                    }
                    update_display();
                }
                _ => {}
            }
        }
    }
}

// ---- Main ----
fn main() {
    if !ui::init() { return; }

    let win = ui::Window::new_with_flags("Calculator", -1, -1, WIN_W, WIN_H,
        ui::WIN_FLAG_NOT_RESIZABLE);
    win.set_color(BG);

    // Menu
    let mut mb = ui::MenuBarBuilder::new()
        .menu("Calculator")
            .item(100, "About Calculator", 0)
            .separator()
            .item(101, "Quit Calculator", 0)
        .end_menu();
    let menu_data = mb.build();
    let menu = ui::MenuBar::set(win.id(), menu_data);
    menu.on_item(|e| {
        if e.item_id == 101 {
            ui::quit();
        }
    });

    // Display label (right-aligned)
    let display_label = ui::Label::new("0");
    display_label.set_position(PAD, PAD);
    display_label.set_size(WIN_W - PAD as u32 * 2, DISPLAY_H);
    display_label.set_text_color(TXT_WHITE);
    display_label.set_font_size(36);
    display_label.set_text_align(ui::TEXT_ALIGN_RIGHT);
    win.add(&display_label);

    // Create buttons
    let dummy = ui::Button::new("");
    let mut btns = [dummy; 19];
    for i in 0..19 {
        let b = &BUTTONS[i];
        let x = PAD + b.col * (BTN_W as i32 + GAP);
        let y = GRID_Y + b.row * (BTN_H as i32 + GAP);
        let w = b.span as u32 * BTN_W + (b.span - 1) as u32 * GAP as u32;

        let btn = ui::Button::new(b.label);
        btn.set_position(x, y);
        btn.set_size(w, BTN_H);
        btn.set_font_size(18);

        match b.kind {
            BKind::Num => {
                btn.set_color(COL_NUM);
                btn.set_text_color(TXT_WHITE);
            }
            BKind::Fn => {
                btn.set_color(COL_FN);
                btn.set_text_color(TXT_BLACK);
            }
            BKind::Op => {
                btn.set_color(COL_OP);
                btn.set_text_color(TXT_WHITE);
            }
        }

        btns[i] = btn;
        win.add(&btn);

        let idx = i;
        btn.on_click(move |_| {
            handle_button(idx);
        });
    }

    // Initialize global state
    unsafe {
        APP = Some(AppState {
            display: String::from("0"),
            acc: 0.0,
            pending: None,
            new_input: true,
            active_op: None,
            display_label,
            btns,
        });
    }

    // Keyboard input
    win.on_key_down(|ke| {
        handle_key(ke);
    });

    win.on_close(|_| {
        ui::quit();
    });

    ui::run();
}
