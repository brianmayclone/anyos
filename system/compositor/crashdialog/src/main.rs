#![no_std]
#![no_main]

use alloc::string::String;
use anyos_std::format;
use anyos_std::println;
use libanyui_client as anyui;

anyos_std::entry!(main);

const DIALOG_W: u32 = 720;
const DIALOG_H_EXPANDED: u32 = 600;
const DIALOG_H_COLLAPSED: u32 = 370;

const LEFT_X: i32 = 34;
const CONTENT_X: i32 = 150;
const CONTENT_W: u32 = 536;
const FOOTER_H: u32 = 64;
const FOOTER_Y_EXPANDED: i32 = 536;
const FOOTER_Y_COLLAPSED: i32 = 306;

const DARK_BG: u32 = 0xFF0D1117;
const DARK_PANEL: u32 = 0xFF161B22;
const DARK_TEXT: u32 = 0xFFE6EDF3;
const DARK_TEXT_SECONDARY: u32 = 0xFF8B949E;
const DARK_CONTROL_BG: u32 = 0xFF21262D;
const DARK_CONTROL_HOVER: u32 = 0xFF30363D;
const DARK_INPUT_BG: u32 = 0xFF0D1117;
const DARK_INPUT_BORDER: u32 = 0xFF30363D;
const DARK_SEPARATOR: u32 = 0xFF21262D;
const DARK_ACCENT: u32 = 0xFF2563EB;
const DARK_WARNING: u32 = 0xFFF59E0B;

#[repr(C)]
struct CrashReport {
    tid: u32,
    signal: u32,
    rip: u64,
    rsp: u64,
    rbp: u64,
    rax: u64,
    rbx: u64,
    rcx: u64,
    rdx: u64,
    rsi: u64,
    rdi: u64,
    r8: u64,
    r9: u64,
    r10: u64,
    r11: u64,
    r12: u64,
    r13: u64,
    r14: u64,
    r15: u64,
    cr2: u64,
    cs: u64,
    ss: u64,
    rflags: u64,
    err_code: u64,
    stack_frames: [u64; 16],
    num_frames: u32,
    name: [u8; 32],
    valid: bool,
}

impl CrashReport {
    const fn empty() -> Self {
        Self {
            tid: 0,
            signal: 0,
            rip: 0,
            rsp: 0,
            rbp: 0,
            rax: 0,
            rbx: 0,
            rcx: 0,
            rdx: 0,
            rsi: 0,
            rdi: 0,
            r8: 0,
            r9: 0,
            r10: 0,
            r11: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
            cr2: 0,
            cs: 0,
            ss: 0,
            rflags: 0,
            err_code: 0,
            stack_frames: [0; 16],
            num_frames: 0,
            name: [0; 32],
            valid: false,
        }
    }
}

#[derive(Clone, Copy)]
struct DialogPalette {
    bg: u32,
    panel: u32,
    text: u32,
    secondary: u32,
    control: u32,
    control_hover: u32,
    input: u32,
    input_border: u32,
    separator: u32,
    accent: u32,
    warning: u32,
}

impl DialogPalette {
    fn current_dark() -> Self {
        let tc = anyui::theme::colors();
        if anyui::theme::is_light() {
            Self {
                bg: DARK_BG,
                panel: DARK_PANEL,
                text: DARK_TEXT,
                secondary: DARK_TEXT_SECONDARY,
                control: DARK_CONTROL_BG,
                control_hover: DARK_CONTROL_HOVER,
                input: DARK_INPUT_BG,
                input_border: DARK_INPUT_BORDER,
                separator: DARK_SEPARATOR,
                accent: DARK_ACCENT,
                warning: DARK_WARNING,
            }
        } else {
            Self {
                bg: tc.window_bg,
                panel: tc.sidebar_bg,
                text: tc.text,
                secondary: tc.text_secondary,
                control: tc.control_bg,
                control_hover: tc.control_hover,
                input: tc.input_bg,
                input_border: tc.input_border,
                separator: tc.separator,
                accent: tc.accent,
                warning: tc.warning,
            }
        }
    }
}

static mut DETAILS_VISIBLE: bool = true;

fn signal_name(sig: u32) -> &'static str {
    match sig {
        132 => "SIGILL",
        135 => "SIGBUS",
        136 => "SIGFPE",
        139 => "SIGSEGV",
        _ => "Signal",
    }
}

fn signal_detail(sig: u32) -> &'static str {
    match sig {
        132 => "Invalid opcode",
        135 => "Device not available",
        136 => "Floating-point exception",
        139 => "Segmentation fault",
        _ => "Unknown fault",
    }
}

fn trim_cstr(bytes: &[u8]) -> &str {
    let len = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    core::str::from_utf8(&bytes[..len]).unwrap_or("")
}

fn format_registers(report: Option<&CrashReport>) -> String {
    if let Some(r) = report {
        if r.valid {
            return format!(
                "    rax: {:#018x}    rbx: {:#018x}\n    rcx: {:#018x}    rdx: {:#018x}\n    rdi: {:#018x}    rsi: {:#018x}\n    rbp: {:#018x}    rsp: {:#018x}\n     r8: {:#018x}     r9: {:#018x}\n    r10: {:#018x}    r11: {:#018x}\n    r12: {:#018x}    r13: {:#018x}\n    r14: {:#018x}    r15: {:#018x}\n    rip: {:#018x} rflags: {:#018x}\n     cs: {:#018x}     ss: {:#018x}\n",
                r.rax,
                r.rbx,
                r.rcx,
                r.rdx,
                r.rdi,
                r.rsi,
                r.rbp,
                r.rsp,
                r.r8,
                r.r9,
                r.r10,
                r.r11,
                r.r12,
                r.r13,
                r.r14,
                r.r15,
                r.rip,
                r.rflags,
                r.cs,
                r.ss,
            );
        }
    }

    String::from("    No register dump was available for this crash.\n")
}

fn format_backtrace(report: Option<&CrashReport>) -> String {
    if let Some(r) = report {
        if !r.valid {
            return String::from("    No backtrace was available for this crash.\n");
        }
        if r.num_frames == 0 {
            return String::from("    No stack frames were captured.\n");
        }

        let mut out = String::new();
        let frames = (r.num_frames as usize).min(r.stack_frames.len());
        for i in 0..frames {
            out.push_str(&format!("    {:>2}  {:#018x}\n", i, r.stack_frames[i]));
        }
        return out;
    }

    String::from("    No backtrace was available for this crash.\n")
}

fn format_problem_details(
    report: Option<&CrashReport>,
    fallback_tid: u32,
    fallback_signal: u32,
    fallback_rip: u64,
    process_name: &str,
) -> String {
    let tid = report.map(|r| r.tid).unwrap_or(fallback_tid);
    let signal = report.map(|r| r.signal).unwrap_or(fallback_signal);
    let rip = report.map(|r| r.rip).unwrap_or(fallback_rip);
    let rsp = report.map(|r| r.rsp).unwrap_or(0);
    let fault_addr = report.map(|r| r.cr2).unwrap_or(0);
    let err_code = report.map(|r| r.err_code).unwrap_or(0);

    let mut out = String::new();
    out.push_str("----------------------------------------\n");
    out.push_str("Translated Report (Full Report Below)\n");
    out.push_str("----------------------------------------\n\n");
    out.push_str(&format!(
        "Incident Identifier: TID-{}-RIP-{:#x}\n",
        tid, rip
    ));
    out.push_str("CrashReporter Key:  anyOS CrashReporter\n");
    out.push_str("Hardware Model:     anyOS x86_64\n");
    out.push_str(&format!("Process:            {} [{}]\n", process_name, tid));
    out.push_str("Path:               not captured\n");
    out.push_str("Identifier:         anyOS.user-process\n\n");
    out.push_str(&format!(
        "Exception Type:     {} ({})\n",
        signal_name(signal),
        signal_detail(signal)
    ));
    out.push_str(&format!("Exception Codes:    error {:#018x}\n", err_code));
    if fault_addr != 0 {
        out.push_str(&format!("Fault Address:      {:#018x}\n", fault_addr));
    }
    out.push_str(&format!("Instruction:        {:#018x}\n", rip));
    if rsp != 0 {
        out.push_str(&format!("Stack Pointer:      {:#018x}\n", rsp));
    }
    out.push_str("\nThread State (x86_64):\n");
    out.push_str(&format_registers(report));
    out.push_str("\nBacktrace:\n");
    out.push_str(&format_backtrace(report));
    out
}

fn draw_warning_icon(canvas: &anyui::Canvas, p: DialogPalette) {
    canvas.clear(p.bg);

    let top_x = 40i32;
    let top_y = 8i32;
    let left_x = 8i32;
    let right_x = 72i32;
    let bottom_y = 70i32;
    let height = bottom_y - top_y;

    let mut y = top_y;
    while y <= bottom_y {
        let dy = y - top_y;
        let lx = top_x - ((top_x - left_x) * dy / height);
        let rx = top_x + ((right_x - top_x) * dy / height);
        canvas.fill_rect(lx, y, (rx - lx + 1) as u32, 1, p.warning);
        y += 1;
    }

    canvas.draw_thick_line(top_x, top_y, left_x, bottom_y, p.input_border, 3);
    canvas.draw_thick_line(top_x, top_y, right_x, bottom_y, p.input_border, 3);
    canvas.draw_thick_line(left_x, bottom_y, right_x, bottom_y, p.input_border, 3);
    canvas.fill_rect(37, 25, 6, 26, 0xFFFFFFFF);
    canvas.fill_circle(40, 59, 4, 0xFFFFFFFF);
}

fn load_crash_report(path: &str) -> Option<CrashReport> {
    let data = anyos_std::fs::read_to_vec(path).ok()?;
    if data.len() < core::mem::size_of::<CrashReport>() {
        return None;
    }

    let mut report = CrashReport::empty();
    unsafe {
        core::ptr::copy_nonoverlapping(
            data.as_ptr(),
            (&mut report as *mut CrashReport).cast::<u8>(),
            core::mem::size_of::<CrashReport>(),
        );
    }
    Some(report)
}

fn format_missing_report(path: &str) -> String {
    format!(
        "----------------------------------------\nCrash Report File Missing\n----------------------------------------\n\nCrashDialog could not read the report file:\n{}\n\nThe process crash was detected, but the Sessionhost report handoff did not provide a readable CrashReport payload.\n",
        path,
    )
}

fn first_token(s: &str) -> Option<&str> {
    let s = s.trim();
    if s.is_empty() {
        None
    } else {
        s.split_whitespace().next()
    }
}

fn crash_log_arg<'a>(raw_args: &'a str, arg_tail: &'a str) -> Option<&'a str> {
    if let Some(path) = first_token(arg_tail) {
        return Some(path);
    }

    // Compatibility for the previous broken handoff where Sessionhost passed
    // only the report path. process::args() treats that single token as argv[0]
    // and strips it, so recover it from the raw argument string.
    let raw = raw_args.trim();
    if raw.starts_with("/tmp/crashlog_") {
        first_token(raw)
    } else {
        None
    }
}

fn main() {
    let mut raw_arg_buf = [0u8; 256];
    let raw_len = anyos_std::process::getargs(&mut raw_arg_buf);
    let raw_args = core::str::from_utf8(&raw_arg_buf[..raw_len]).unwrap_or("");

    let mut arg_buf = [0u8; 256];
    let args = anyos_std::process::args(&mut arg_buf);

    let crash_log_path = match crash_log_arg(raw_args, args) {
        Some(path) => path,
        _ => {
            println!(
                "crashdialog: usage: CrashDialog <crash_report_path> (raw_args='{}')",
                raw_args
            );
            return;
        }
    };
    println!(
        "crashdialog: raw_args='{}' args='{}' report='{}'",
        raw_args, args, crash_log_path
    );

    let crash_report = load_crash_report(crash_log_path);
    let report = crash_report.as_ref();
    let tid = report.map(|r| r.tid).unwrap_or(0);
    let signal = report.map(|r| r.signal).unwrap_or(0);
    let rip = report.map(|r| r.rip).unwrap_or(0);

    let thread_name = if let Some(r) = report {
        let from_report = trim_cstr(&r.name);
        if from_report.is_empty() {
            "unknown"
        } else {
            from_report
        }
    } else {
        "unknown"
    };

    let crash_signal = report.map(|r| r.signal).unwrap_or(signal);
    let problem_title = format!("Problem Report for {}", thread_name);
    let headline_text = format!("{} quit unexpectedly.", thread_name);
    let detail_text = if report.is_some() {
        format_problem_details(report, tid, signal, rip, thread_name)
    } else {
        format_missing_report(crash_log_path)
    };

    if !anyui::init() {
        println!("crashdialog: failed to init anyui");
        return;
    }

    let palette = DialogPalette::current_dark();
    unsafe {
        DETAILS_VISIBLE = true;
    }

    let (screen_w, screen_h) = anyui::screen_size();
    let wx = (screen_w.saturating_sub(DIALOG_W) / 2) as i32;
    let wy = (screen_h.saturating_sub(DIALOG_H_EXPANDED) / 2) as i32;

    let win = anyui::Window::new_with_flags(
        &problem_title,
        wx,
        wy,
        DIALOG_W,
        DIALOG_H_EXPANDED,
        anyui::WIN_FLAG_ALWAYS_ON_TOP | anyui::WIN_FLAG_SHADOW | anyui::WIN_FLAG_NOT_RESIZABLE,
    );

    let root = anyui::View::new();
    root.set_dock(anyui::DOCK_FILL);
    root.set_color(palette.bg);
    win.add(&root);

    let top_rule = anyui::View::new();
    top_rule.set_position(0, 0);
    top_rule.set_size(DIALOG_W, 1);
    top_rule.set_color(palette.separator);
    root.add(&top_rule);

    let icon = anyui::Canvas::new(80, 80);
    icon.set_position(LEFT_X, 42);
    draw_warning_icon(&icon, palette);
    root.add(&icon);

    let headline = anyui::Label::new(&headline_text);
    headline.set_position(CONTENT_X, 34);
    headline.set_size(CONTENT_W, 30);
    headline.set_font(1);
    headline.set_font_size(18);
    headline.set_text_color(palette.text);
    root.add(&headline);

    let message = anyui::Label::new("anyOS captured a diagnostic report for this process.");
    message.set_position(CONTENT_X, 70);
    message.set_size(CONTENT_W, 22);
    message.set_font_size(14);
    message.set_text_color(palette.text);
    root.add(&message);

    let signal_line = format!(
        "{} ({})",
        signal_name(crash_signal),
        signal_detail(crash_signal)
    );
    let signal_label = anyui::Label::new(&signal_line);
    signal_label.set_position(CONTENT_X, 94);
    signal_label.set_size(CONTENT_W, 20);
    signal_label.set_font_size(12);
    signal_label.set_text_color(palette.secondary);
    root.add(&signal_label);

    let comments_header = anyui::Label::new("Comments");
    comments_header.set_position(CONTENT_X, 132);
    comments_header.set_size(CONTENT_W, 24);
    comments_header.set_font_size(15);
    comments_header.set_text_color(palette.text);
    root.add(&comments_header);

    let comments = anyui::TextArea::new();
    comments.set_position(CONTENT_X, 160);
    comments.set_size(CONTENT_W, 92);
    comments.set_color(palette.input);
    comments.set_text_color(palette.secondary);
    comments.set_font_size(14);
    comments.set_max_length(2048);
    root.add(&comments);

    let comment_placeholder =
        anyui::Label::new("Provide any steps necessary to reproduce the problem.");
    comment_placeholder.set_position(CONTENT_X + 10, 168);
    comment_placeholder.set_size(CONTENT_W - 20, 20);
    comment_placeholder.set_font_size(14);
    comment_placeholder.set_text_color(palette.secondary);
    root.add(&comment_placeholder);

    {
        let comments = comments;
        let comment_placeholder = comment_placeholder;
        comments.on_text_changed(move |_| {
            let mut first = [0u8; 1];
            comment_placeholder.set_visible(comments.get_text(&mut first) == 0);
        });
    }

    let details_label = anyui::Label::new("Problem Details and System Configuration");
    details_label.set_position(CONTENT_X, 274);
    details_label.set_size(CONTENT_W, 24);
    details_label.set_font_size(15);
    details_label.set_text_color(palette.text);
    root.add(&details_label);

    let details = anyui::TextArea::new();
    details.set_position(CONTENT_X, 302);
    details.set_size(CONTENT_W, 206);
    details.set_color(palette.input);
    details.set_text_color(palette.text);
    details.set_font(4);
    details.set_font_size(12);
    details.set_read_only(true);
    details.set_text(&detail_text);
    details.set_cursor(0);
    root.add(&details);

    let footer_rule = anyui::View::new();
    footer_rule.set_position(0, FOOTER_Y_EXPANDED - 12);
    footer_rule.set_size(DIALOG_W, 1);
    footer_rule.set_color(palette.separator);
    root.add(&footer_rule);

    let footer = anyui::View::new();
    footer.set_position(0, FOOTER_Y_EXPANDED);
    footer.set_size(DIALOG_W, FOOTER_H);
    footer.set_color(palette.panel);
    root.add(&footer);

    let help = anyui::Label::new("?");
    help.set_position(28, 16);
    help.set_size(32, 32);
    help.set_color(palette.control_hover);
    help.set_text_align(anyui::TEXT_ALIGN_CENTER);
    help.set_font(1);
    help.set_font_size(18);
    help.set_text_color(palette.accent);
    footer.add(&help);

    let details_toggle = anyui::Button::new("Hide Details");
    details_toggle.set_position(CONTENT_X, 17);
    details_toggle.set_size(124, 30);
    details_toggle.set_color(palette.control);
    footer.add(&details_toggle);

    let ok = anyui::Button::new("OK");
    ok.set_position(582, 17);
    ok.set_size(104, 30);
    ok.set_color(palette.accent);
    ok.on_click(|_| anyui::quit());
    footer.add(&ok);

    {
        let win = win;
        let details = details;
        let details_label = details_label;
        let footer = footer;
        let footer_rule = footer_rule;
        let details_toggle = details_toggle;
        details_toggle.on_click(move |_| {
            let visible = unsafe {
                DETAILS_VISIBLE = !DETAILS_VISIBLE;
                DETAILS_VISIBLE
            };
            details.set_visible(visible);
            details_label.set_visible(visible);

            if visible {
                footer_rule.set_position(0, FOOTER_Y_EXPANDED - 12);
                footer.set_position(0, FOOTER_Y_EXPANDED);
                details_toggle.set_text("Hide Details");
                win.resize(DIALOG_W, DIALOG_H_EXPANDED);
            } else {
                footer_rule.set_position(0, FOOTER_Y_COLLAPSED - 12);
                footer.set_position(0, FOOTER_Y_COLLAPSED);
                details_toggle.set_text("Show Details");
                win.resize(DIALOG_W, DIALOG_H_COLLAPSED);
            }
        });
    }

    win.on_close(|_| anyui::quit());
    win.on_key_down(|e| {
        if e.keycode == 27 || e.keycode == 13 {
            anyui::quit();
        }
    });

    anyui::run();
}
