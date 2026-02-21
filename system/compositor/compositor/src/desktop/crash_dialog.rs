//! Crash dialog — displayed when a user-space thread terminates with a signal.
//!
//! Shows thread name, signal, and an expandable section with register dump and stack trace.

use alloc::string::String;
use alloc::format;

/// Mirrors `kernel/src/task/crash_info::CrashReport` layout.
/// Must be kept in sync with the kernel struct.
#[repr(C)]
pub struct CrashReport {
    pub tid: u32,
    pub signal: u32,
    pub rip: u64,
    pub rsp: u64,
    pub rbp: u64,
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub cr2: u64,
    pub cs: u64,
    pub ss: u64,
    pub rflags: u64,
    pub err_code: u64,
    pub stack_frames: [u64; 16],
    pub num_frames: u32,
    pub name: [u8; 32],
    pub valid: bool,
}

/// Dialog width and heights.
const DIALOG_W: u32 = 420;
const DIALOG_COLLAPSED_H: u32 = 160;
const DIALOG_EXPANDED_H: u32 = 440;

/// Button dimensions.
const OK_BTN_W: u32 = 80;
const OK_BTN_H: u32 = 28;
const OK_BTN_RADIUS: u32 = 6;

/// Colors.
const COLOR_DIALOG_BG: u32 = 0xFF2A2A2A;
const COLOR_TEXT: u32 = 0xFFE6E6E6;
const COLOR_TEXT_DIM: u32 = 0xFF969696;
const COLOR_DETAIL_BG: u32 = 0xFF1E1E1E;
const COLOR_RED: u32 = 0xFFFF3B30;
const COLOR_ACCENT: u32 = 0xFF007AFF;
const COLOR_BTN_BG: u32 = 0xFF3C3C3C;

/// Font constants (match compositor theme).
const FONT_ID: u16 = 0;
const FONT_ID_BOLD: u16 = 1;
const FONT_SIZE: u16 = 13;
const FONT_SIZE_SMALL: u16 = 11;
const FONT_SIZE_TITLE: u16 = 15;

/// Stored state for one crash dialog.
pub struct CrashDialog {
    /// Compositor window ID for this dialog.
    pub window_id: u32,
    /// TID of the crashed thread.
    pub tid: u32,
    /// Human-readable thread name.
    pub thread_name: String,
    /// Signal name (e.g. "SIGSEGV").
    pub signal_name: String,
    /// Signal number.
    pub signal: u32,
    /// Whether the details section is expanded.
    pub details_expanded: bool,
    /// Formatted crash details text (registers + stack trace).
    pub crash_text: String,
    /// Faulting address (RIP).
    pub rip: u64,
    /// CR2 for page faults.
    pub cr2: u64,
}

impl CrashDialog {
    /// Create a new crash dialog from a raw CrashReport buffer.
    pub fn from_report(window_id: u32, report: &CrashReport) -> Self {
        let thread_name = {
            let len = report.name.iter().position(|&b| b == 0).unwrap_or(32);
            let s = core::str::from_utf8(&report.name[..len]).unwrap_or("unknown");
            String::from(s)
        };

        let signal_name = String::from(match report.signal {
            132 => "SIGILL (Invalid opcode)",
            135 => "SIGBUS (Device not available)",
            136 => "SIGFPE (Floating-point exception)",
            139 => "SIGSEGV (Segmentation fault)",
            _ => "Unknown signal",
        });

        // Format register dump
        let mut text = String::new();

        text.push_str(&format!("RIP: {:#018x}\n", report.rip));
        text.push_str(&format!("RSP: {:#018x}  RBP: {:#018x}\n", report.rsp, report.rbp));
        text.push_str(&format!("RAX: {:#018x}  RBX: {:#018x}\n", report.rax, report.rbx));
        text.push_str(&format!("RCX: {:#018x}  RDX: {:#018x}\n", report.rcx, report.rdx));
        text.push_str(&format!("RSI: {:#018x}  RDI: {:#018x}\n", report.rsi, report.rdi));
        text.push_str(&format!("R8:  {:#018x}  R9:  {:#018x}\n", report.r8, report.r9));
        text.push_str(&format!("R10: {:#018x}  R11: {:#018x}\n", report.r10, report.r11));
        text.push_str(&format!("R12: {:#018x}  R13: {:#018x}\n", report.r12, report.r13));
        text.push_str(&format!("R14: {:#018x}  R15: {:#018x}\n", report.r14, report.r15));

        if report.signal == 139 && report.cr2 != 0 {
            text.push_str(&format!("CR2: {:#018x}\n", report.cr2));
        }

        if report.num_frames > 0 {
            text.push_str("\nStack trace:\n");
            for i in 0..report.num_frames.min(16) {
                text.push_str(&format!("  #{}: {:#018x}\n", i, report.stack_frames[i as usize]));
            }
        } else {
            text.push_str("\n(no stack trace available)\n");
        }

        CrashDialog {
            window_id,
            tid: report.tid,
            thread_name,
            signal_name,
            signal: report.signal,
            details_expanded: false,
            crash_text: text,
            rip: report.rip,
            cr2: report.cr2,
        }
    }

    /// Current content height for the dialog.
    pub fn content_height(&self) -> u32 {
        if self.details_expanded {
            DIALOG_EXPANDED_H
        } else {
            DIALOG_COLLAPSED_H
        }
    }

    /// Render the dialog content into the window's pixel buffer.
    pub fn render(&self, pixels: &mut [u32], stride: u32, full_h: u32) {
        use super::drawing::{fill_rect, fill_rounded_rect};

        // Clear content area (below title bar, which the window system draws)
        let content_y = super::window::TITLE_BAR_HEIGHT;
        for row in content_y..full_h {
            for col in 0..stride {
                let idx = (row * stride + col) as usize;
                if idx < pixels.len() {
                    pixels[idx] = COLOR_DIALOG_BG;
                }
            }
        }

        let mut y = content_y as i32 + 16;
        let x_pad = 20i32;

        // Red warning indicator circle
        fill_rounded_rect(pixels, stride, full_h, x_pad, y, 8, 8, 4, COLOR_RED);

        // Title
        anyos_std::ui::window::font_render_buf(
            FONT_ID_BOLD, FONT_SIZE_TITLE, pixels, stride, full_h,
            x_pad + 16, y - 2, COLOR_TEXT, "Application Crashed",
        );
        y += 28;

        // Thread info
        let info_line = format!("{} (TID {})", self.thread_name, self.tid);
        anyos_std::ui::window::font_render_buf(
            FONT_ID, FONT_SIZE, pixels, stride, full_h,
            x_pad, y, COLOR_TEXT, &info_line,
        );
        y += 20;

        // Signal
        anyos_std::ui::window::font_render_buf(
            FONT_ID, FONT_SIZE, pixels, stride, full_h,
            x_pad, y, COLOR_TEXT_DIM, &self.signal_name,
        );
        y += 20;

        // RIP
        let rip_text = format!("at {:#018x}", self.rip);
        anyos_std::ui::window::font_render_buf(
            FONT_ID, FONT_SIZE_SMALL, pixels, stride, full_h,
            x_pad, y, COLOR_TEXT_DIM, &rip_text,
        );
        y += 24;

        // Details toggle
        let arrow = if self.details_expanded { "\x19" } else { "\x1A" }; // down/right arrow
        let details_label = format!("{} Details", arrow);
        anyos_std::ui::window::font_render_buf(
            FONT_ID, FONT_SIZE, pixels, stride, full_h,
            x_pad, y, COLOR_ACCENT, &details_label,
        );
        y += 20;

        if self.details_expanded {
            // Details background
            let detail_x = x_pad;
            let detail_y = y;
            let detail_w = (stride as i32 - 2 * x_pad) as u32;
            let detail_h = (full_h as i32 - y - 50) as u32; // leave room for OK button
            fill_rounded_rect(pixels, stride, full_h, detail_x, detail_y, detail_w, detail_h, 4, COLOR_DETAIL_BG);

            // Render crash text lines
            let mut ty = detail_y + 8;
            for line in self.crash_text.lines() {
                if ty + 14 > detail_y + detail_h as i32 {
                    break;
                }
                anyos_std::ui::window::font_render_buf(
                    FONT_ID, FONT_SIZE_SMALL, pixels, stride, full_h,
                    detail_x + 8, ty, COLOR_TEXT_DIM, line,
                );
                ty += 14;
            }
        }

        // OK button (bottom-right)
        let btn_x = (stride as i32 - x_pad - OK_BTN_W as i32);
        let btn_y = (full_h as i32 - 20 - OK_BTN_H as i32);
        fill_rounded_rect(pixels, stride, full_h, btn_x, btn_y, OK_BTN_W, OK_BTN_H, OK_BTN_RADIUS, COLOR_ACCENT);

        let (tw, th) = anyos_std::ui::window::font_measure(FONT_ID_BOLD, FONT_SIZE, "OK");
        let tx = btn_x + (OK_BTN_W as i32 - tw as i32) / 2;
        let ty = btn_y + (OK_BTN_H as i32 - th as i32) / 2;
        anyos_std::ui::window::font_render_buf(
            FONT_ID_BOLD, FONT_SIZE, pixels, stride, full_h,
            tx, ty, 0xFFFFFFFF, "OK",
        );
    }

    /// Handle a click within the dialog's content area.
    /// Returns `CrashDialogAction` indicating what to do.
    pub fn handle_click(&mut self, wx: i32, wy: i32) -> CrashDialogAction {
        let content_y = super::window::TITLE_BAR_HEIGHT as i32;
        let x_pad = 20i32;

        // Check OK button
        let btn_x = (DIALOG_W as i32 - x_pad - OK_BTN_W as i32);
        let content_h = self.content_height();
        let full_h = content_h + super::window::TITLE_BAR_HEIGHT;
        let btn_y = (full_h as i32 - 20 - OK_BTN_H as i32);
        if wx >= btn_x && wx < btn_x + OK_BTN_W as i32
            && wy >= btn_y && wy < btn_y + OK_BTN_H as i32
        {
            return CrashDialogAction::Dismiss;
        }

        // Check Details toggle area (roughly y=content_y+108..+128)
        let details_y = content_y + 108;
        if wy >= details_y && wy < details_y + 20 && wx >= x_pad && wx < x_pad + 100 {
            self.details_expanded = !self.details_expanded;
            return CrashDialogAction::ToggleDetails;
        }

        CrashDialogAction::None
    }
}

/// Action returned by click handling.
pub enum CrashDialogAction {
    /// No action.
    None,
    /// Toggle details expansion — caller should resize and re-render.
    ToggleDetails,
    /// Dismiss the dialog — caller should destroy the window.
    Dismiss,
}
