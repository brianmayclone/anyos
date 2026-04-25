use alloc::format;
use alloc::string::String;
use libanyui_client as ui;
use ui::Widget;

use crate::logic::ai::{AiConfig, AiProvider};

// ════════════════════════════════════════════════════════════════
//  AI Settings Dialog — proper configuration UI
//
//  ┌─── AI Assistant Settings ──────────────────────────┐
//  │                                                     │
//  │  Provider       [OpenAI         ▼]                  │
//  │                                                     │
//  │  API Key        [sk-••••••••••••••••••]              │
//  │                 Your key is stored locally only.     │
//  │                                                     │
//  │  Model          [gpt-4o         ▼]                  │
//  │                                                     │
//  │  ─────────── Advanced ───────────                   │
//  │                                                     │
//  │  Max Tokens     [4096        ]                      │
//  │  Temperature    [====●=======] 0.7                  │
//  │  Custom Endpoint [                    ]             │
//  │                                                     │
//  │  ┌──────────────────────────────────────────┐       │
//  │  │ Status: ✓ Connected / ✗ Not configured   │       │
//  │  └──────────────────────────────────────────┘       │
//  │                                                     │
//  │               [Test Connection]  [Save]  [Cancel]   │
//  └─────────────────────────────────────────────────────┘
// ════════════════════════════════════════════════════════════════

const DLG_W: u32 = 520;
const DLG_H: u32 = 480;
const LABEL_X: i32 = 24;
const FIELD_X: i32 = 150;
const FIELD_W: u32 = 330;

pub struct AiSettingsDialog {
    win: ui::Window,
    provider_dropdown: ui::DropDown,
    api_key_field: ui::TextField,
    model_dropdown: ui::DropDown,
    max_tokens_field: ui::TextField,
    temperature_slider: ui::Slider,
    temperature_label: ui::Label,
    endpoint_field: ui::TextField,
    status_label: ui::Label,
    btn_test: ui::Button,
    btn_save: ui::Button,
    btn_cancel: ui::Button,
}

impl AiSettingsDialog {
    /// Show the AI settings dialog. Loads current config and populates fields.
    pub fn show() {
        let config = AiConfig::load();
        let tc = ui::theme::colors();
        let t = anyos_std::i18n::t;

        let win = ui::Window::new(t("AI Assistant Settings"), -1, -1, DLG_W, DLG_H);

        // ── Header with accent stripe ──
        let header = ui::View::new();
        header.set_dock(ui::DOCK_TOP);
        header.set_size(DLG_W, 56);
        header.set_color(tc.sidebar_bg);
        win.add(&header);

        let stripe = ui::View::new();
        stripe.set_dock(ui::DOCK_TOP);
        stripe.set_size(DLG_W, 3);
        stripe.set_color(tc.accent);
        header.add(&stripe);

        let title = ui::Label::new(t("AI Assistant Settings"));
        title.set_position(24, 16);
        title.set_font_size(18);
        title.set_text_color(tc.text);
        header.add(&title);

        let desc = ui::Label::new(t("Configure your AI coding assistant"));
        desc.set_position(24, 38);
        desc.set_font_size(11);
        desc.set_text_color(tc.text_secondary);
        header.add(&desc);

        // ── Main content area ──
        let content = ui::View::new();
        content.set_dock(ui::DOCK_FILL);
        content.set_color(tc.editor_bg);
        win.add(&content);

        let mut y: i32 = 16;

        // ── Provider ──
        let prov_label = ui::Label::new(t("Provider"));
        prov_label.set_position(LABEL_X, y + 4);
        prov_label.set_font_size(13);
        prov_label.set_text_color(tc.text);
        content.add(&prov_label);

        let provider_dropdown = ui::DropDown::new("OpenAI|Anthropic (Claude)");
        provider_dropdown.set_position(FIELD_X, y);
        provider_dropdown.set_size(FIELD_W, 28);
        let prov_idx = match config.provider {
            AiProvider::OpenAI => 0,
            AiProvider::Anthropic => 1,
        };
        provider_dropdown.set_state(prov_idx);
        content.add(&provider_dropdown);

        y += 42;

        // ── API Key ──
        let key_label = ui::Label::new(t("API Key"));
        key_label.set_position(LABEL_X, y + 4);
        key_label.set_font_size(13);
        key_label.set_text_color(tc.text);
        content.add(&key_label);

        let api_key_field = ui::TextField::new();
        api_key_field.set_position(FIELD_X, y);
        api_key_field.set_size(FIELD_W, 28);
        api_key_field.set_font(4);
        api_key_field.set_font_size(12);
        api_key_field.set_color(tc.control_bg);
        api_key_field.set_text_color(tc.text);
        api_key_field.set_placeholder("sk-... or anthropic key");
        if !config.api_key.is_empty() {
            api_key_field.set_text(&config.api_key);
        }
        content.add(&api_key_field);

        y += 24;

        let key_hint = ui::Label::new(t("Your key is stored locally only."));
        key_hint.set_position(FIELD_X, y + 4);
        key_hint.set_font_size(10);
        key_hint.set_text_color(tc.text_secondary);
        content.add(&key_hint);

        y += 30;

        // ── Model ──
        let model_label = ui::Label::new(t("Model"));
        model_label.set_position(LABEL_X, y + 4);
        model_label.set_font_size(13);
        model_label.set_text_color(tc.text);
        content.add(&model_label);

        let models_openai = "gpt-5.3-codex|gpt-5.4|gpt-5.4-mini|gpt-4o";
        let models_anthropic =
            "claude-sonnet-4-20250514|claude-haiku-4-5-20251001|claude-opus-4-20250514";
        let model_items = match config.provider {
            AiProvider::OpenAI => models_openai,
            AiProvider::Anthropic => models_anthropic,
        };

        let model_dropdown = ui::DropDown::new(model_items);
        model_dropdown.set_position(FIELD_X, y);
        model_dropdown.set_size(FIELD_W, 28);
        // Select current model
        let model_idx = find_model_index(model_items, &config.model);
        model_dropdown.set_state(model_idx);
        content.add(&model_dropdown);

        y += 48;

        // ── Advanced separator ──
        let adv_sep = ui::View::new();
        adv_sep.set_position(LABEL_X, y);
        adv_sep.set_size((DLG_W - 48) as u32, 1);
        adv_sep.set_color(0xFF333333);
        content.add(&adv_sep);

        y += 4;
        let adv_label = ui::Label::new(t("Advanced"));
        adv_label.set_position(LABEL_X, y);
        adv_label.set_font_size(11);
        adv_label.set_text_color(tc.text_secondary);
        content.add(&adv_label);

        y += 24;

        // ── Max Tokens ──
        let tok_label = ui::Label::new(t("Max Tokens"));
        tok_label.set_position(LABEL_X, y + 4);
        tok_label.set_font_size(13);
        tok_label.set_text_color(tc.text);
        content.add(&tok_label);

        let max_tokens_field = ui::TextField::new();
        max_tokens_field.set_position(FIELD_X, y);
        max_tokens_field.set_size(120, 28);
        max_tokens_field.set_font(4);
        max_tokens_field.set_font_size(12);
        max_tokens_field.set_color(tc.control_bg);
        max_tokens_field.set_text_color(tc.text);
        max_tokens_field.set_text(&format!("{}", config.max_tokens));
        content.add(&max_tokens_field);

        y += 42;

        // ── Temperature ──
        let temp_label = ui::Label::new(t("Temperature"));
        temp_label.set_position(LABEL_X, y + 4);
        temp_label.set_font_size(13);
        temp_label.set_text_color(tc.text);
        content.add(&temp_label);

        let temp_pct = ((config.temperature / 2.0) * 100.0) as u32;
        let temperature_slider = ui::Slider::new(temp_pct.min(100));
        temperature_slider.set_position(FIELD_X, y + 2);
        temperature_slider.set_size(220, 24);
        content.add(&temperature_slider);

        let temp_val = format!("{:.1}", config.temperature);
        let temperature_label = ui::Label::new(&temp_val);
        temperature_label.set_position(FIELD_X + 230, y + 4);
        temperature_label.set_font_size(12);
        temperature_label.set_text_color(tc.text);
        content.add(&temperature_label);

        y += 42;

        // ── Custom Endpoint ──
        let ep_label = ui::Label::new(t("Endpoint"));
        ep_label.set_position(LABEL_X, y + 4);
        ep_label.set_font_size(13);
        ep_label.set_text_color(tc.text);
        content.add(&ep_label);

        let endpoint_field = ui::TextField::new();
        endpoint_field.set_position(FIELD_X, y);
        endpoint_field.set_size(FIELD_W, 28);
        endpoint_field.set_font(4);
        endpoint_field.set_font_size(12);
        endpoint_field.set_color(tc.control_bg);
        endpoint_field.set_text_color(tc.text);
        endpoint_field.set_placeholder(t("Leave empty for default"));
        if !config.custom_endpoint.is_empty() {
            endpoint_field.set_text(&config.custom_endpoint);
        }
        content.add(&endpoint_field);

        y += 20;

        let ep_hint = ui::Label::new(t("Optional: self-hosted or proxy endpoint URL"));
        ep_hint.set_position(FIELD_X, y + 4);
        ep_hint.set_font_size(10);
        ep_hint.set_text_color(tc.text_secondary);
        content.add(&ep_hint);

        // ── Status indicator ──
        let status_text = if config.is_configured() {
            format!(
                "\u{2713} {} — {}",
                t("Configured"),
                config.provider.display_name()
            )
        } else {
            format!(
                "\u{2717} {}",
                t("Not configured — enter your API key above")
            )
        };
        let status_label = ui::Label::new(&status_text);
        status_label.set_position(LABEL_X, DLG_H as i32 - 110);
        status_label.set_font_size(12);
        status_label.set_text_color(if config.is_configured() {
            tc.success
        } else {
            tc.warning
        });
        content.add(&status_label);

        // ── Button bar ──
        let btn_bar = ui::View::new();
        btn_bar.set_dock(ui::DOCK_BOTTOM);
        btn_bar.set_size(DLG_W, 50);
        btn_bar.set_color(tc.sidebar_bg);
        win.add(&btn_bar);

        let btn_test = ui::Button::new(t("Test Connection"));
        btn_test.set_size(130, 32);
        btn_test.set_position(24, 9);
        btn_test.set_color(tc.control_bg);
        btn_bar.add(&btn_test);

        let btn_save = ui::Button::new(t("Save"));
        btn_save.set_size(80, 32);
        btn_save.set_position((DLG_W as i32) - 190, 9);
        btn_save.set_color(tc.accent);
        btn_bar.add(&btn_save);

        let btn_cancel = ui::Button::new(t("Cancel"));
        btn_cancel.set_size(80, 32);
        btn_cancel.set_position((DLG_W as i32) - 100, 9);
        btn_cancel.set_color(tc.control_bg);
        btn_bar.add(&btn_cancel);

        // ── Wire events ──
        let win_id = win.id();

        // Provider change tracked (model list includes both providers' models)

        // Temperature slider → update label
        let temp_lbl_id = temperature_label.id();
        temperature_slider.on_value_changed(move |e| {
            let val = (e.value as f64) * 2.0 / 100.0;
            let text = format!("{:.1}", val);
            ui::Control::from_id(temp_lbl_id).set_text(&text);
        });

        // Save button
        let key_id = api_key_field.id();
        let prov_id = provider_dropdown.id();
        let model_id = model_dropdown.id();
        let tokens_id = max_tokens_field.id();
        let temp_id = temperature_slider.id();
        let ep_id = endpoint_field.id();
        let status_id = status_label.id();

        btn_save.on_click(move |_| {
            save_config(
                prov_id, key_id, model_id, tokens_id, temp_id, ep_id, status_id,
            );
            ui::Control::from_id(win_id).set_visible(false);
        });

        // Cancel button
        btn_cancel.on_click(move |_| {
            ui::Control::from_id(win_id).set_visible(false);
        });

        // Test button
        let key_id2 = api_key_field.id();
        let prov_id2 = provider_dropdown.id();
        let status_id2 = status_label.id();
        btn_test.on_click(move |_| {
            test_connection(prov_id2, key_id2, status_id2);
        });

        win.on_close(move |_| {
            // Window closed via X button — just destroy
        });
    }
}

fn save_config(
    prov_id: u32,
    key_id: u32,
    model_id: u32,
    tokens_id: u32,
    temp_id: u32,
    ep_id: u32,
    status_id: u32,
) {
    let tc = ui::theme::colors();

    // Read provider
    let prov_state = ui::Control::from_id(prov_id).get_state();
    let provider = if prov_state == 1 {
        AiProvider::Anthropic
    } else {
        AiProvider::OpenAI
    };

    // Read API key
    let mut key_buf = [0u8; 256];
    let key_len = ui::Control::from_id(key_id).get_text(&mut key_buf);
    let api_key = core::str::from_utf8(&key_buf[..key_len as usize])
        .unwrap_or("")
        .trim();

    // Read model
    let model_state = ui::Control::from_id(model_id).get_state();
    let model = match provider {
        AiProvider::OpenAI => match model_state {
            0 => "gpt-5.3-codex",
            1 => "gpt-5.4",
            2 => "gpt-5.4-mini",
            3 => "gpt-4o",
            _ => "gpt-5.3-codex",
        },
        AiProvider::Anthropic => match model_state {
            0 => "claude-sonnet-4-20250514",
            1 => "claude-haiku-4-5-20251001",
            2 => "claude-opus-4-20250514",
            _ => "claude-sonnet-4-20250514",
        },
    };

    // Read max tokens
    let mut tok_buf = [0u8; 32];
    let tok_len = ui::Control::from_id(tokens_id).get_text(&mut tok_buf);
    let tok_str = core::str::from_utf8(&tok_buf[..tok_len as usize]).unwrap_or("4096");
    let max_tokens = parse_u32(tok_str).unwrap_or(4096);

    // Read temperature from slider (0-100 → 0.0-2.0)
    let temp_state = ui::Control::from_id(temp_id).get_state();
    let temperature = (temp_state as f64) * 2.0 / 100.0;

    // Read endpoint
    let mut ep_buf = [0u8; 256];
    let ep_len = ui::Control::from_id(ep_id).get_text(&mut ep_buf);
    let endpoint = core::str::from_utf8(&ep_buf[..ep_len as usize])
        .unwrap_or("")
        .trim();

    // Build and save config
    let config = AiConfig {
        provider,
        api_key: String::from(api_key),
        model: String::from(model),
        max_tokens,
        temperature,
        reasoning_effort: String::from("medium"),
        privacy_mode: true,
        require_patch_review: true,
        custom_endpoint: String::from(endpoint),
    };
    config.save();

    // Update main app AI client
    let s = crate::app();
    s.ai_client.config = AiConfig::load();
    s.ai_panel.set_provider(s.ai_client.config.provider);

    // Update status
    if config.is_configured() {
        let t = anyos_std::i18n::t;
        let status = ui::Control::from_id(status_id);
        status.set_text(&format!(
            "\u{2713} {} — {}",
            t("Saved"),
            provider.display_name()
        ));
        status.set_text_color(tc.success);
    }
}

fn test_connection(prov_id: u32, key_id: u32, status_id: u32) {
    let tc = ui::theme::colors();
    let t = anyos_std::i18n::t;
    let status = ui::Control::from_id(status_id);

    // Read current field values
    let prov_state = ui::Control::from_id(prov_id).get_state();
    let provider = if prov_state == 1 {
        AiProvider::Anthropic
    } else {
        AiProvider::OpenAI
    };

    let mut key_buf = [0u8; 256];
    let key_len = ui::Control::from_id(key_id).get_text(&mut key_buf);
    let api_key = core::str::from_utf8(&key_buf[..key_len as usize]).unwrap_or("");

    if api_key.trim().is_empty() {
        status.set_text(&format!("\u{2717} {}", t("Enter an API key first")));
        status.set_text_color(tc.destructive);
        return;
    }

    status.set_text(t("Testing connection..."));
    status.set_text_color(tc.text_secondary);

    // Quick test: send a tiny request
    let test_config = AiConfig {
        provider,
        api_key: String::from(api_key.trim()),
        model: String::from(provider.default_model()),
        max_tokens: 10,
        temperature: 0.0,
        reasoning_effort: String::from("medium"),
        privacy_mode: true,
        require_patch_review: true,
        custom_endpoint: String::new(),
    };

    let mut client = crate::logic::ai::AiClient {
        config: test_config,
        history: alloc::vec::Vec::new(),
        is_requesting: false,
    };

    match client.chat("Say 'ok'") {
        Ok(_) => {
            status.set_text(&format!(
                "\u{2713} {} — {}",
                t("Connection successful"),
                provider.display_name()
            ));
            status.set_text_color(tc.success);
        }
        Err(e) => {
            let msg = if e.len() > 60 { &e[..60] } else { &e };
            status.set_text(&format!("\u{2717} {}", msg));
            status.set_text_color(tc.destructive);
        }
    }
}

fn find_model_index(items: &str, model: &str) -> u32 {
    for (i, item) in items.split('|').enumerate() {
        if item == model {
            return i as u32;
        }
    }
    0
}

fn parse_u32(s: &str) -> Option<u32> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut result: u32 = 0;
    for b in s.bytes() {
        if b < b'0' || b > b'9' {
            return None;
        }
        result = result.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(result)
}
