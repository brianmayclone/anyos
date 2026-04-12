#![no_std]
#![no_main]

use alloc::string::String;
use anyos_std::format;
use anyos_std::println;
use libanyui_client as anyui;

anyos_std::entry!(main);

const DIALOG_W: u32 = 660;
const DIALOG_H: u32 = 540;

const CONTENT_W: u32 = 612;
const CONTENT_H: u32 = 860;
const VIEWPORT_H: u32 = 460;

const HEADER_H: u32 = 120;
const SUMMARY_H: u32 = 126;
const SECTION_HEADER_H: u32 = 32;
const SUMMARY_TEXT_H: u32 = 108;
const REG_TEXT_H: u32 = 190;
const TRACE_TEXT_H: u32 = 132;

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

fn trim_cstr(bytes: &[u8]) -> &str {
    let len = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    core::str::from_utf8(&bytes[..len]).unwrap_or("")
}

fn format_summary(report: Option<&CrashReport>, fallback_tid: u32, fallback_sig: u32, fallback_rip: u64) -> String {
    if let Some(r) = report {
        let fault_addr = if r.cr2 != 0 {
            format!("Fault address: {:#018x}\n", r.cr2)
        } else {
            String::new()
        };
        return format!(
            "The process crashed while running in user mode.\n\nThread: {} (TID {})\nSignal: {} ({})\nInstruction pointer: {:#018x}\nStack pointer: {:#018x}\nError code: {:#018x}\n{}CS={:#x}  SS={:#x}  RFLAGS={:#018x}\n",
            trim_cstr(&r.name),
            r.tid,
            signal_name(r.signal),
            signal_detail(r.signal),
            r.rip,
            r.rsp,
            r.err_code,
            fault_addr,
            r.cs,
            r.ss,
            r.rflags,
        );
    }

    format!(
        "The process crashed, but no extended crash report was available.\n\nTID: {}\nSignal: {} ({})\nInstruction pointer: {:#018x}\n",
        fallback_tid,
        signal_name(fallback_sig),
        signal_detail(fallback_sig),
        fallback_rip,
    )
}

fn format_registers(report: Option<&CrashReport>) -> String {
    if let Some(r) = report {
        return format!(
            "RAX {:#018x}\nRBX {:#018x}\nRCX {:#018x}\nRDX {:#018x}\nRSI {:#018x}\nRDI {:#018x}\nRBP {:#018x}\nRSP {:#018x}\n\nR8  {:#018x}\nR9  {:#018x}\nR10 {:#018x}\nR11 {:#018x}\nR12 {:#018x}\nR13 {:#018x}\nR14 {:#018x}\nR15 {:#018x}\n",
            r.rax, r.rbx, r.rcx, r.rdx, r.rsi, r.rdi, r.rbp, r.rsp,
            r.r8, r.r9, r.r10, r.r11, r.r12, r.r13, r.r14, r.r15,
        );
    }

    String::from("No register dump was available for this crash.")
}

fn format_backtrace(report: Option<&CrashReport>) -> String {
    if let Some(r) = report {
        if r.num_frames == 0 {
            return String::from("No stack frames were captured.");
        }
        let mut out = String::new();
        let frames = (r.num_frames as usize).min(r.stack_frames.len());
        for i in 0..frames {
            let line = format!("#{:<2}  {:#018x}\n", i, r.stack_frames[i]);
            out.push_str(&line);
        }
        return out;
    }

    String::from("No backtrace was available for this crash.")
}

fn section_height(text_h: u32, expanded: bool) -> u32 {
    if expanded {
        SECTION_HEADER_H + text_h + 10
    } else {
        SECTION_HEADER_H
    }
}

fn update_section_layout(
    win: &anyui::Window,
    content: &anyui::View,
    summary_exp: &anyui::Expander,
    summary_editor: &anyui::TextEditor,
    regs_exp: &anyui::Expander,
    regs_editor: &anyui::TextEditor,
    trace_exp: &anyui::Expander,
    trace_editor: &anyui::TextEditor,
) {
    let mut y = HEADER_H as i32 + SUMMARY_H as i32 + 12;

    let summary_open = summary_exp.is_expanded();
    let summary_h = section_height(SUMMARY_TEXT_H, summary_open);
    summary_exp.set_position(0, y);
    summary_exp.set_size(CONTENT_W, summary_h);
    summary_editor.set_visible(summary_open);
    y += summary_h as i32 + 8;

    let regs_open = regs_exp.is_expanded();
    let regs_h = section_height(REG_TEXT_H, regs_open);
    regs_exp.set_position(0, y);
    regs_exp.set_size(CONTENT_W, regs_h);
    regs_editor.set_visible(regs_open);
    y += regs_h as i32 + 8;

    let trace_open = trace_exp.is_expanded();
    let trace_h = section_height(TRACE_TEXT_H, trace_open);
    trace_exp.set_position(0, y);
    trace_exp.set_size(CONTENT_W, trace_h);
    trace_editor.set_visible(trace_open);
    y += trace_h as i32 + 14;

    let total_h = y.max(VIEWPORT_H as i32 + 4) as u32;
    content.set_size(CONTENT_W, total_h);
    let current_w = win.get_size().0;
    win.resize(current_w.max(DIALOG_W), DIALOG_H.max(340));
}

fn main() {
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
    let thread_name_arg = parts.next().unwrap_or("unknown");

    let tid = parse_u32(tid_str);
    let signal = parse_u32(sig_str);
    let rip = parse_hex(rip_str);

    let mut crash_buf = [0u8; core::mem::size_of::<CrashReport>()];
    let crash_len = anyos_std::sys::get_crash_info(tid, &mut crash_buf) as usize;
    let report = if crash_len >= core::mem::size_of::<CrashReport>() {
        Some(unsafe { &*(crash_buf.as_ptr() as *const CrashReport) })
    } else {
        None
    };

    let thread_name = if let Some(r) = report {
        let from_report = trim_cstr(&r.name);
        if from_report.is_empty() { thread_name_arg } else { from_report }
    } else {
        thread_name_arg
    };

    let signal_line = format!("{} ({})", signal_name(signal), signal_detail(signal));
    let summary_text = format_summary(report, tid, signal, rip);
    let register_text = format_registers(report);
    let trace_text = format_backtrace(report);

    if !anyui::init() {
        println!("crashdialog: failed to init anyui");
        return;
    }

    let (screen_w, screen_h) = anyui::screen_size();
    let wx = (screen_w.saturating_sub(DIALOG_W) / 2) as i32;
    let wy = (screen_h.saturating_sub(DIALOG_H) / 2) as i32;

    let win = anyui::Window::new_with_flags(
        "Application Crash",
        wx,
        wy,
        DIALOG_W,
        DIALOG_H,
        anyui::WIN_FLAG_ALWAYS_ON_TOP | anyui::WIN_FLAG_SHADOW,
    );

    let root = anyui::View::new();
    root.set_dock(anyui::DOCK_FILL);
    root.set_color(0xFFF4F0EA);
    win.add(&root);

    let header = anyui::Card::new();
    header.set_position(16, 16);
    header.set_size(CONTENT_W, HEADER_H);
    header.set_color(0xFF221A18);
    root.add(&header);

    let badge = anyui::Label::new("Crash Report");
    badge.set_position(18, 16);
    badge.set_size(116, 24);
    badge.set_color(0xFFE45D48);
    badge.set_text_color(0xFFFFFFFF);
    badge.set_text_align(anyui::TEXT_ALIGN_CENTER);
    header.add(&badge);

    let title = anyui::Label::new("This application stopped unexpectedly");
    title.set_position(18, 48);
    title.set_size(530, 30);
    title.set_font(1);
    title.set_font_size(18);
    title.set_text_color(0xFFF7F2EC);
    header.add(&title);

    let subtitle = anyui::Label::new("You can close this dialog or inspect the captured crash details below.");
    subtitle.set_position(18, 80);
    subtitle.set_size(560, 22);
    subtitle.set_text_color(0xFFD9CBC2);
    header.add(&subtitle);

    let close_hint = anyui::PlainButton::new("Dismiss");
    close_hint.set_position(506, 18);
    close_hint.set_size(86, 26);
    close_hint.on_click(|_| anyui::quit());
    header.add(&close_hint);

    let summary = anyui::Card::new();
    summary.set_position(16, 148);
    summary.set_size(CONTENT_W, SUMMARY_H);
    summary.set_color(0xFFFFFBF7);
    root.add(&summary);

    let name_lbl = anyui::Label::new(thread_name);
    name_lbl.set_position(18, 14);
    name_lbl.set_size(360, 24);
    name_lbl.set_font(1);
    name_lbl.set_font_size(16);
    name_lbl.set_text_color(0xFF2A211F);
    summary.add(&name_lbl);

    let signal_lbl = anyui::Label::new(&signal_line);
    signal_lbl.set_position(18, 42);
    signal_lbl.set_size(260, 20);
    signal_lbl.set_color(0xFFFCE4DE);
    signal_lbl.set_text_color(0xFFC44536);
    summary.add(&signal_lbl);

    let tid_lbl = anyui::Label::new(&format!("Thread ID: {}", tid));
    tid_lbl.set_position(18, 72);
    tid_lbl.set_size(180, 20);
    tid_lbl.set_text_color(0xFF655854);
    summary.add(&tid_lbl);

    let rip_lbl = anyui::Label::new(&format!("Instruction: {:#018x}", report.map(|r| r.rip).unwrap_or(rip)));
    rip_lbl.set_position(220, 72);
    rip_lbl.set_size(340, 20);
    rip_lbl.set_text_color(0xFF655854);
    summary.add(&rip_lbl);

    let note_lbl = anyui::Label::new("If the app keeps crashing, inspect registers and stack frames before retrying.");
    note_lbl.set_position(18, 96);
    note_lbl.set_size(560, 18);
    note_lbl.set_text_color(0xFF8A7A74);
    summary.add(&note_lbl);

    let scroll = anyui::ScrollView::new();
    scroll.set_position(16, 286);
    scroll.set_size(CONTENT_W, VIEWPORT_H);
    scroll.set_color(0xFFF4F0EA);
    root.add(&scroll);

    let content = anyui::View::new();
    content.set_position(0, 0);
    content.set_size(CONTENT_W, CONTENT_H);
    content.set_color(0xFFF4F0EA);
    scroll.add(&content);

    let summary_exp = anyui::Expander::new("Summary");
    summary_exp.set_expanded(true);
    content.add(&summary_exp);

    let summary_editor = anyui::TextEditor::new(CONTENT_W - 20, SUMMARY_TEXT_H);
    summary_editor.set_position(10, 32);
    summary_editor.set_read_only(true);
    summary_editor.set_show_line_numbers(false);
    summary_editor.set_editor_font(2, 12);
    summary_editor.set_text(&summary_text);
    summary_exp.add(&summary_editor);

    let regs_exp = anyui::Expander::new("Registers");
    regs_exp.set_expanded(false);
    content.add(&regs_exp);

    let regs_editor = anyui::TextEditor::new(CONTENT_W - 20, REG_TEXT_H);
    regs_editor.set_position(10, 32);
    regs_editor.set_read_only(true);
    regs_editor.set_show_line_numbers(false);
    regs_editor.set_editor_font(2, 12);
    regs_editor.set_text(&register_text);
    regs_exp.add(&regs_editor);

    let trace_exp = anyui::Expander::new("Backtrace");
    trace_exp.set_expanded(false);
    content.add(&trace_exp);

    let trace_editor = anyui::TextEditor::new(CONTENT_W - 20, TRACE_TEXT_H);
    trace_editor.set_position(10, 32);
    trace_editor.set_read_only(true);
    trace_editor.set_show_line_numbers(false);
    trace_editor.set_editor_font(2, 12);
    trace_editor.set_text(&trace_text);
    trace_exp.add(&trace_editor);

    update_section_layout(
        &win,
        &content,
        &summary_exp,
        &summary_editor,
        &regs_exp,
        &regs_editor,
        &trace_exp,
        &trace_editor,
    );

    {
        let win = win;
        let content = content;
        let summary_exp = summary_exp;
        let summary_editor = summary_editor;
        let regs_exp = regs_exp;
        let regs_editor = regs_editor;
        let trace_exp = trace_exp;
        let trace_editor = trace_editor;
        summary_exp.on_toggled(move |_| {
            update_section_layout(
                &win,
                &content,
                &summary_exp,
                &summary_editor,
                &regs_exp,
                &regs_editor,
                &trace_exp,
                &trace_editor,
            );
        });
    }

    {
        let win = win;
        let content = content;
        let summary_exp = summary_exp;
        let summary_editor = summary_editor;
        let regs_exp = regs_exp;
        let regs_editor = regs_editor;
        let trace_exp = trace_exp;
        let trace_editor = trace_editor;
        regs_exp.on_toggled(move |_| {
            update_section_layout(
                &win,
                &content,
                &summary_exp,
                &summary_editor,
                &regs_exp,
                &regs_editor,
                &trace_exp,
                &trace_editor,
            );
        });
    }

    {
        let win = win;
        let content = content;
        let summary_exp = summary_exp;
        let summary_editor = summary_editor;
        let regs_exp = regs_exp;
        let regs_editor = regs_editor;
        let trace_exp = trace_exp;
        let trace_editor = trace_editor;
        trace_exp.on_toggled(move |_| {
            update_section_layout(
                &win,
                &content,
                &summary_exp,
                &summary_editor,
                &regs_exp,
                &regs_editor,
                &trace_exp,
                &trace_editor,
            );
        });
    }

    let dismiss_btn = anyui::Button::new("Close");
    dismiss_btn.set_position(504, 494);
    dismiss_btn.set_size(124, 30);
    dismiss_btn.set_color(0xFF201A18);
    dismiss_btn.on_click(|_| anyui::quit());
    root.add(&dismiss_btn);

    let secondary = anyui::Label::new("Esc, Enter or window close also dismiss this dialog.");
    secondary.set_position(20, 500);
    secondary.set_size(360, 18);
    secondary.set_text_color(0xFF7D6E68);
    secondary.on_click(|_| anyui::quit());
    root.add(&secondary);

    win.on_close(|_| anyui::quit());
    win.on_key_down(|e| {
        if e.keycode == 27 || e.keycode == 13 {
            anyui::quit();
        }
    });

    anyui::run();
}
