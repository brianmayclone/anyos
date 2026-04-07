use alloc::format;
use alloc::string::String;
use libanyui_client as anyui;
use anyui::Widget;

use crate::logic::build::ToolStatus;

// ════════════════════════════════════════════════════════════════
//  Splash screen — Visual Studio-style startup screen
//  Shows for 3 seconds, then auto-closes. No buttons needed.
// ════════════════════════════════════════════════════════════════

const ESSENTIAL_TOOLS: &[&str] = &["anyrc", "acargo", "cc", "make"];
const SPLASH_DURATION_MS: u32 = 3000;

/// Show the splash screen for 3 seconds.
/// Returns `false` if essential tools are missing and user should be warned.
pub fn show(tool_status: &[ToolStatus]) -> bool {
    let tc = anyui::theme::colors();

    // ── Splash window (no title bar, centered, dark) ──
    let win = anyui::Window::new("", -1, -1, 520, 320);

    // ── Full dark background ──
    let bg = anyui::View::new();
    bg.set_dock(anyui::DOCK_FILL);
    bg.set_color(0xFF1E1E1E);
    win.add(&bg);

    // ── Accent stripe at top (4px) ──
    let stripe = anyui::View::new();
    stripe.set_dock(anyui::DOCK_TOP);
    stripe.set_size(520, 4);
    stripe.set_color(tc.accent);
    bg.add(&stripe);

    // ── Product title ──
    let title = anyui::Label::new("anyOS Code");
    title.set_position(40, 40);
    title.set_font_size(36);
    title.set_text_color(0xFFFFFFFF);
    bg.add(&title);

    // ── Subtitle ──
    let subtitle = anyui::Label::new("Professional IDE");
    subtitle.set_position(42, 86);
    subtitle.set_font_size(14);
    subtitle.set_text_color(0xAACCCCCC);
    bg.add(&subtitle);

    // ── Version ──
    let version = anyui::Label::new("Version 2.0");
    version.set_position(42, 108);
    version.set_font_size(12);
    version.set_text_color(0x88AAAAAA);
    bg.add(&version);

    // ── Thin separator line ──
    let sep = anyui::View::new();
    sep.set_position(40, 138);
    sep.set_size(440, 1);
    sep.set_color(0xFF333333);
    bg.add(&sep);

    // ── Tool status (compact, right-aligned list) ──
    let mut y = 152;
    let mut all_essential_ok = true;
    let mut missing_tools = String::new();

    for tool in tool_status {
        let is_essential = ESSENTIAL_TOOLS.contains(&tool.name);

        let icon = if tool.available { "\u{2713}" } else { "\u{2717}" };
        let label_text = format!("{} {}", icon, tool.description);

        let lbl = anyui::Label::new(&label_text);
        lbl.set_position(50, y);
        lbl.set_font_size(11);
        lbl.set_text_color(if tool.available {
            0xFF6A9955  // green
        } else if is_essential {
            0xFFF44747  // red
        } else {
            0xFF808080  // grey
        });
        bg.add(&lbl);

        // Path or "not found" on the right
        let status = if tool.available {
            format!("{}", tool.name)
        } else {
            if is_essential {
                all_essential_ok = false;
                if !missing_tools.is_empty() {
                    missing_tools.push_str(", ");
                }
                missing_tools.push_str(tool.name);
            }
            String::from("not found")
        };

        let status_lbl = anyui::Label::new(&status);
        status_lbl.set_position(300, y);
        status_lbl.set_font_size(11);
        status_lbl.set_text_color(if tool.available { 0xFF808080 } else { 0xFFF44747 });
        bg.add(&status_lbl);

        y += 18;
    }

    // ── Loading indicator ──
    let loading_text = if all_essential_ok {
        "Loading workspace..."
    } else {
        "Warning: essential tools missing"
    };
    let loading = anyui::Label::new(loading_text);
    loading.set_position(42, 286);
    loading.set_font_size(11);
    loading.set_text_color(if all_essential_ok { 0xFF569CD6 } else { 0xFFF44747 });
    bg.add(&loading);

    // ── Copyright ──
    let copy = anyui::Label::new("anyOS Project");
    copy.set_position(380, 286);
    copy.set_font_size(11);
    copy.set_text_color(0xFF555555);
    bg.add(&copy);

    // ── Show for 3 seconds using run_once() loop ──
    // This avoids calling quit() which would poison the global quit state.
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

    // Close the splash window
    win.destroy();

    // If essential tools are missing, show a warning dialog
    if !all_essential_ok {
        let msg = format!(
            "The following essential tools are not installed:\n{}\n\nInstall them to /System/bin/ for full IDE functionality.",
            missing_tools,
        );
        anyui::MessageBox::show(anyui::MessageBoxType::Warning, &msg, Some("OK"));
    }

    true
}
