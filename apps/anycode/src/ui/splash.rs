use alloc::format;
use alloc::string::String;
use libanyui_client as anyui;

use crate::logic::build::ToolStatus;

// ════════════════════════════════════════════════════════════════
//  Splash screen — borderless, centered, auto-closes after 3s
// ════════════════════════════════════════════════════════════════

const ESSENTIAL_TOOLS: &[&str] = &["crust", "ccargo", "cc", "make"];
const SPLASH_W: u32 = 520;
const SPLASH_H: u32 = 300;
const SPLASH_DURATION_MS: u32 = 3000;

pub fn show(tool_status: &[ToolStatus]) -> bool {
    let tc = anyui::theme::colors();

    // Center on screen
    let (sw, sh) = anyui::screen_size();
    let x = if sw > SPLASH_W {
        (sw - SPLASH_W) / 2
    } else {
        0
    } as i32;
    let y = if sh > SPLASH_H {
        (sh - SPLASH_H) / 2
    } else {
        0
    } as i32;

    // Borderless window with shadow
    let flags = anyui::WIN_FLAG_BORDERLESS | anyui::WIN_FLAG_SHADOW | anyui::WIN_FLAG_NOT_RESIZABLE;
    let win = anyui::Window::new_with_flags("", x, y, SPLASH_W, SPLASH_H, flags);

    // ── Dark background ──
    let bg = anyui::View::new();
    bg.set_dock(anyui::DOCK_FILL);
    bg.set_color(0xFF1A1A2E);
    win.add(&bg);

    // ── Accent stripe (top, 3px) ──
    let stripe = anyui::View::new();
    stripe.set_dock(anyui::DOCK_TOP);
    stripe.set_size(SPLASH_W, 3);
    stripe.set_color(tc.accent);
    bg.add(&stripe);

    // ── Product name ──
    let title = anyui::Label::new("anyOS Code");
    title.set_position(36, 36);
    title.set_font_size(32);
    title.set_text_color(0xFFFFFFFF);
    bg.add(&title);

    let subtitle = anyui::Label::new("Professional IDE");
    subtitle.set_position(38, 76);
    subtitle.set_font_size(13);
    subtitle.set_text_color(0xAACCCCCC);
    bg.add(&subtitle);

    let version = anyui::Label::new("Version 2.0");
    version.set_position(38, 96);
    version.set_font_size(11);
    version.set_text_color(0x88888888);
    bg.add(&version);

    // ── Tool status (compact list) ──
    let mut y_pos: i32 = 130;
    let mut all_essential_ok = true;
    let mut missing = String::new();

    for tool in tool_status {
        let is_essential = ESSENTIAL_TOOLS.contains(&tool.name);
        let icon = if tool.available {
            "\u{2713}"
        } else {
            "\u{2717}"
        };
        let text = format!("{} {}", icon, tool.description);

        let lbl = anyui::Label::new(&text);
        lbl.set_position(44, y_pos);
        lbl.set_font_size(11);
        lbl.set_text_color(if tool.available {
            0xFF6A9955
        } else if is_essential {
            all_essential_ok = false;
            if !missing.is_empty() {
                missing.push_str(", ");
            }
            missing.push_str(tool.name);
            0xFFF44747
        } else {
            0xFF666666
        });
        bg.add(&lbl);

        if tool.available {
            let path_lbl = anyui::Label::new(tool.name);
            path_lbl.set_position(280, y_pos);
            path_lbl.set_font_size(11);
            path_lbl.set_text_color(0xFF555555);
            bg.add(&path_lbl);
        }

        y_pos += 17;
    }

    // ── Bottom bar ──
    let loading = anyui::Label::new(if all_essential_ok {
        "Loading workspace..."
    } else {
        "Some tools are missing"
    });
    loading.set_position(38, (SPLASH_H as i32) - 26);
    loading.set_font_size(11);
    loading.set_text_color(if all_essential_ok {
        0xFF569CD6
    } else {
        0xFFF44747
    });
    bg.add(&loading);

    let copy = anyui::Label::new("anyOS Project");
    copy.set_position((SPLASH_W as i32) - 110, (SPLASH_H as i32) - 26);
    copy.set_font_size(11);
    copy.set_text_color(0xFF444444);
    bg.add(&copy);

    // ── Run for 3 seconds then close ──
    let start = anyos_std::sys::uptime_ms();
    loop {
        if !anyui::run_once() {
            break;
        }
        let elapsed = anyos_std::sys::uptime_ms().wrapping_sub(start);
        if elapsed >= SPLASH_DURATION_MS {
            break;
        }
    }

    win.destroy();

    // Warn about missing tools
    if !all_essential_ok {
        let msg = format!(
            "Missing: {}\n\nInstall to /System/bin/ for full functionality.",
            missing,
        );
        anyui::MessageBox::show(anyui::MessageBoxType::Warning, &msg, Some("OK"));
    }

    true
}
