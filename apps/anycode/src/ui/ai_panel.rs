use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use libanyui_client as ui;

use crate::logic::ai::{AiProvider, ChatMessage, MessageRole};

// ════════════════════════════════════════════════════════════════
//  AI Chat Panel — Vibe Coding Interface
//
//  Layout:
//    ┌─────────────────────────────────────┐
//    │  AI ASSISTANT              [Claude ▼]│
//    │  ┌─────────────────────────────────┐│
//    │  │ Welcome! I'm your AI coding     ││
//    │  │ assistant. Ask me anything...    ││
//    │  │                                 ││
//    │  │ 🤖 How can I help?              ││
//    │  │                                 ││
//    │  │ You: explain this function      ││
//    │  │                                 ││
//    │  │ AI: This function does...       ││
//    │  └─────────────────────────────────┘│
//    │  ┌─────────────────────────────────┐│
//    │  │ Ask AI...                  [Send]││
//    │  └─────────────────────────────────┘│
//    │  [Explain] [Refactor] [Fix] [Tests] │
//    └─────────────────────────────────────┘
// ════════════════════════════════════════════════════════════════

pub struct AiPanel {
    pub panel: ui::View,
    pub chat_area: ui::TextArea,
    pub input_field: ui::TextField,
    pub btn_send: ui::Button,
    // Quick action buttons
    pub btn_explain: ui::Button,
    pub btn_refactor: ui::Button,
    pub btn_fix: ui::Button,
    pub btn_generate: ui::Button,
    pub btn_test: ui::Button,
    pub btn_review: ui::Button,
    // Settings
    pub btn_settings: ui::LinkLabel,
    pub btn_clear: ui::Button,
    // Provider label
    pub provider_label: ui::Label,
    pub status_label: ui::Label,
}

impl AiPanel {
    pub fn new() -> Self {
        let tc = ui::theme::colors();
        let panel = ui::View::new();
        panel.set_dock(ui::DOCK_FILL);
        panel.set_color(tc.sidebar_bg);

        let t = anyos_std::i18n::t;

        // ── Header ──
        let header = ui::View::new();
        header.set_dock(ui::DOCK_TOP);
        header.set_size(200, 22);
        header.set_color(tc.sidebar_bg);
        panel.add(&header);

        let header_label = ui::Label::new(t("AI ASSISTANT"));
        header_label.set_position(8, 4);
        header_label.set_font_size(11);
        header_label.set_text_color(tc.text_secondary);
        header.add(&header_label);

        let provider_label = ui::Label::new("OpenAI");
        provider_label.set_position(140, 4);
        provider_label.set_font_size(11);
        provider_label.set_text_color(tc.accent);
        header.add(&provider_label);

        // ── Quick action bar (top) ──
        let action_bar = ui::FlowPanel::new();
        action_bar.set_dock(ui::DOCK_TOP);
        action_bar.set_size(200, 56);
        action_bar.set_color(tc.sidebar_bg);
        panel.add(&action_bar);

        let btn_explain = make_action_btn(t("Explain"), tc.accent);
        action_bar.add(&btn_explain);

        let btn_refactor = make_action_btn(t("Refactor"), tc.accent);
        action_bar.add(&btn_refactor);

        let btn_fix = make_action_btn(t("Fix"), tc.warning);
        action_bar.add(&btn_fix);

        let btn_generate = make_action_btn(t("Generate"), tc.success);
        action_bar.add(&btn_generate);

        let btn_test = make_action_btn(t("Tests"), tc.accent);
        action_bar.add(&btn_test);

        let btn_review = make_action_btn(t("Review"), tc.accent);
        action_bar.add(&btn_review);

        // ── Separator ──
        let sep = ui::View::new();
        sep.set_dock(ui::DOCK_TOP);
        sep.set_size(200, 1);
        sep.set_color(0xFF333333);
        panel.add(&sep);

        // ── Input area (bottom) ──
        let input_bar = ui::View::new();
        input_bar.set_dock(ui::DOCK_BOTTOM);
        input_bar.set_size(200, 34);
        input_bar.set_color(tc.sidebar_bg);
        panel.add(&input_bar);

        let input_field = ui::TextField::new();
        input_field.set_dock(ui::DOCK_FILL);
        input_field.set_size(160, 28);
        input_field.set_margin(4, 3, 50, 3);
        input_field.set_font(4);
        input_field.set_font_size(12);
        input_field.set_color(tc.control_bg);
        input_field.set_text_color(tc.text);
        input_field.set_placeholder(t("Ask AI... (Enter to send)"));
        input_bar.add(&input_field);

        let btn_send = ui::Button::new(t("Send"));
        btn_send.set_dock(ui::DOCK_RIGHT);
        btn_send.set_size(46, 28);
        btn_send.set_margin(0, 3, 4, 3);
        btn_send.set_color(tc.accent);
        input_bar.add(&btn_send);

        // ── Bottom button bar ──
        let bottom_bar = ui::View::new();
        bottom_bar.set_dock(ui::DOCK_BOTTOM);
        bottom_bar.set_size(200, 28);
        bottom_bar.set_color(tc.sidebar_bg);
        panel.add(&bottom_bar);

        let btn_clear = ui::Button::new(t("Clear"));
        btn_clear.set_size(50, 22);
        btn_clear.set_position(4, 2);
        bottom_bar.add(&btn_clear);

        let btn_settings = ui::LinkLabel::new(t("Settings..."));
        btn_settings.set_size(76, 22);
        btn_settings.set_position(60, 2);
        btn_settings.set_font_size(11);
        bottom_bar.add(&btn_settings);

        let status_label = ui::Label::new("");
        status_label.set_position(130, 5);
        status_label.set_font_size(11);
        status_label.set_text_color(tc.text_secondary);
        bottom_bar.add(&status_label);

        // ── Chat area (center, fills remaining space) ──
        let chat_area = ui::TextArea::new();
        chat_area.set_dock(ui::DOCK_FILL);
        chat_area.set_font(4);
        chat_area.set_font_size(12);
        chat_area.set_color(tc.editor_bg);
        chat_area.set_text_color(tc.text);
        panel.add(&chat_area);

        // Welcome message
        let welcome = format!(
            "{}\n{}\n\n{}\n\n{}\n  {} - {}\n  {} - {}\n  {} - {}\n  {} - {}\n  {} - {}\n",
            "Welcome to anyOS Code AI Assistant!",
            "────────────────────────────────────",
            "I can help you write, understand, and improve code.",
            "Quick actions (select code first):",
            "Explain",
            "Explain what selected code does",
            "Refactor",
            "Improve code structure",
            "Fix",
            "Find and fix bugs",
            "Generate",
            "Generate new code from description",
            "Tests",
            "Generate unit tests",
        );
        chat_area.set_text(&welcome);

        Self {
            panel,
            chat_area,
            input_field,
            btn_send,
            btn_explain,
            btn_refactor,
            btn_fix,
            btn_generate,
            btn_test,
            btn_review,
            btn_settings,
            btn_clear,
            provider_label,
            status_label,
        }
    }

    /// Get the user's input text.
    pub fn get_input(&self) -> String {
        let mut buf = [0u8; 4096];
        let len = self.input_field.get_text(&mut buf);
        if len == 0 {
            return String::new();
        }
        match core::str::from_utf8(&buf[..len as usize]) {
            Ok(s) => String::from(s),
            Err(_) => String::new(),
        }
    }

    /// Clear the input field.
    pub fn clear_input(&self) {
        self.input_field.set_text("");
    }

    /// Append a user message to the chat.
    pub fn append_user_message(&self, msg: &str) {
        let formatted = format!("\nYou: {}\n", msg);
        self.append_to_chat(&formatted);
    }

    /// Append an AI response to the chat.
    pub fn append_ai_response(&self, msg: &str) {
        let formatted = format!("\nAI: {}\n", msg);
        self.append_to_chat(&formatted);
    }

    /// Append an error message.
    pub fn append_error(&self, msg: &str) {
        let formatted = format!("\n[Error] {}\n", msg);
        self.append_to_chat(&formatted);
    }

    /// Show a status message.
    pub fn set_status(&self, msg: &str) {
        self.status_label.set_text(msg);
    }

    /// Clear the chat.
    pub fn clear_chat(&self) {
        self.chat_area.set_text("Chat cleared.\n\n");
    }

    /// Update the provider label.
    pub fn set_provider(&self, provider: AiProvider) {
        self.provider_label.set_text(provider.display_name());
    }

    /// Show the login/setup view.
    pub fn show_setup_needed(&self) {
        let text = "\
AI Assistant Setup Required\n\
────────────────────────────\n\
\n\
To use the AI assistant, you need an API key.\n\
\n\
1. Click 'Settings' below\n\
2. Set your provider (OpenAI or Claude)\n\
3. Enter your API key\n\
\n\
OpenAI:    Get a key at platform.openai.com\n\
Anthropic: Get a key at console.anthropic.com\n\
\n\
The key is stored in your user profile via confd.\n\
";
        self.chat_area.set_text(text);
    }

    fn append_to_chat(&self, text: &str) {
        let mut buf = [0u8; 32768];
        let existing = self.chat_area.get_text(&mut buf) as usize;
        let add = text.len().min(buf.len() - existing);
        buf[existing..existing + add].copy_from_slice(&text.as_bytes()[..add]);
        let total = existing + add;
        if let Ok(full) = core::str::from_utf8(&buf[..total]) {
            self.chat_area.set_text(full);
        }
    }
}

fn make_action_btn(label: &str, color: u32) -> ui::Button {
    let btn = ui::Button::new(label);
    btn.set_size(58, 22);
    btn.set_color(color);
    btn
}
