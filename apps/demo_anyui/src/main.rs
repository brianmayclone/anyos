//! demo_anyui — Showcase of all anyui components.
//!
//! Demonstrates ScrollView, Expander, StackPanel, ContextMenu, Tooltips,
//! and every control type in an organized, scrollable layout.

#![no_std]
#![no_main]

use libanyui_client as ui;
use ui::Widget;

anyos_std::entry!(main);

fn main() {
    if !ui::init() {
        anyos_std::println!("Failed to load libanyui.so");
        return;
    }

    let win = ui::Window::new("anyui Component Showcase", -1, -1, 460, 520);

    // ── ScrollView wraps all content ──
    let scroll = ui::ScrollView::new();
    scroll.set_position(0, 0);
    scroll.set_size(460, 520);
    scroll.set_dock(ui::DOCK_FILL);
    win.add(&scroll);

    // Content container — tall enough for all sections
    let content = ui::StackPanel::vertical();
    content.set_position(0, 0);
    content.set_size(440, 1400);
    content.set_padding(20, 10, 20, 20);
    scroll.add(&content);

    // ════════════════════════════════════════════════════════════════
    //  Header
    // ════════════════════════════════════════════════════════════════

    let title = ui::Label::new("anyui Component Showcase");
    title.set_color(0xFF007AFF);
    title.set_size(420, 20);
    title.set_margin(0, 0, 0, 4);
    content.add(&title);

    let subtitle = ui::Label::new("Scroll down to see all components");
    subtitle.set_color(0xFF969696);
    subtitle.set_size(420, 16);
    subtitle.set_margin(0, 0, 0, 8);
    content.add(&subtitle);

    let div = ui::Divider::new();
    div.set_size(420, 1);
    div.set_margin(0, 0, 0, 8);
    content.add(&div);

    // ════════════════════════════════════════════════════════════════
    //  Section 1: Buttons & Actions
    // ════════════════════════════════════════════════════════════════

    let exp_buttons = ui::Expander::new("Buttons & Actions");
    exp_buttons.set_size(420, 82); // 32 header + 50 content
    exp_buttons.set_margin(0, 0, 0, 8);
    content.add(&exp_buttons);

    // Horizontal row of button-like controls
    let row_btns = ui::FlowPanel::new();
    row_btns.set_position(0, 0);
    row_btns.set_size(420, 40);
    row_btns.set_padding(4, 4, 4, 4);
    exp_buttons.add(&row_btns);

    let tooltip_btn = ui::Tooltip::new("Click to show a MessageBox");
    tooltip_btn.set_size(100, 32);
    tooltip_btn.set_margin(0, 0, 6, 0);
    row_btns.add(&tooltip_btn);

    let btn = ui::Button::new("Primary");
    btn.set_size(100, 32);
    btn.on_click(|_e| {
        ui::MessageBox::show(ui::MessageBoxType::Info, "Button clicked!", None);
    });
    tooltip_btn.add(&btn);

    let icon_btn = ui::IconButton::new("*");
    icon_btn.set_size(32, 32);
    icon_btn.set_margin(0, 0, 6, 0);
    icon_btn.on_click(|_e| {
        ui::MessageBox::show(ui::MessageBoxType::Warning, "Starred!", Some("Cool"));
    });
    row_btns.add(&icon_btn);

    let tag1 = ui::Tag::new("Rust");
    tag1.set_margin(0, 4, 4, 0);
    row_btns.add(&tag1);

    let tag2 = ui::Tag::new("anyOS");
    tag2.set_margin(0, 4, 4, 0);
    row_btns.add(&tag2);

    let badge = ui::Badge::new("3");
    badge.set_margin(0, 4, 8, 0);
    row_btns.add(&badge);

    let status = ui::StatusIndicator::new("Online");
    status.set_margin(0, 6, 0, 0);
    row_btns.add(&status);

    // ════════════════════════════════════════════════════════════════
    //  Section 2: Input Controls
    // ════════════════════════════════════════════════════════════════

    let exp_inputs = ui::Expander::new("Input Controls");
    exp_inputs.set_size(420, 280); // 32 + 248
    exp_inputs.set_margin(0, 0, 0, 8);
    content.add(&exp_inputs);

    let inp_stack = ui::StackPanel::vertical();
    inp_stack.set_position(0, 0);
    inp_stack.set_size(420, 248);
    inp_stack.set_padding(4, 4, 4, 4);
    exp_inputs.add(&inp_stack);

    // Toggle row
    let toggle_row = ui::View::new();
    toggle_row.set_size(412, 28);
    toggle_row.set_margin(0, 0, 0, 6);
    inp_stack.add(&toggle_row);

    let toggle_lbl = ui::Label::new("Dark Mode");
    toggle_lbl.set_position(0, 4);
    toggle_row.add(&toggle_lbl);

    let toggle = ui::Toggle::new(true);
    toggle.set_position(100, 0);
    toggle_row.add(&toggle);

    // Checkbox
    let cb = ui::Checkbox::new("Enable notifications");
    cb.set_size(200, 20);
    cb.set_margin(0, 0, 0, 6);
    inp_stack.add(&cb);

    // Radio buttons row
    let radio_row = ui::View::new();
    radio_row.set_size(412, 20);
    radio_row.set_margin(0, 0, 0, 6);
    inp_stack.add(&radio_row);

    let rb1 = ui::RadioButton::new("Option A");
    rb1.set_position(0, 0);
    radio_row.add(&rb1);

    let rb2 = ui::RadioButton::new("Option B");
    rb2.set_position(120, 0);
    radio_row.add(&rb2);

    // Text inputs row
    let text_row = ui::View::new();
    text_row.set_size(412, 28);
    text_row.set_margin(0, 0, 0, 6);
    inp_stack.add(&text_row);

    let tooltip_tf = ui::Tooltip::new("Type your name here");
    tooltip_tf.set_position(0, 0);
    tooltip_tf.set_size(200, 28);
    text_row.add(&tooltip_tf);

    let tf = ui::TextField::new();
    tf.set_size(200, 28);
    tf.set_text("Hello World");
    tooltip_tf.add(&tf);

    let search = ui::SearchField::new();
    search.set_position(208, 0);
    search.set_size(200, 28);
    search.set_placeholder("Search...");
    text_row.add(&search);

    // TextArea
    let ta = ui::TextArea::new();
    ta.set_size(412, 60);
    ta.set_text("Multi-line text area.\nType here...");
    inp_stack.add(&ta);

    // ════════════════════════════════════════════════════════════════
    //  Section 3: Sliders & Progress
    // ════════════════════════════════════════════════════════════════

    let exp_sliders = ui::Expander::new("Sliders & Progress");
    exp_sliders.set_size(420, 152); // 32 + 120
    exp_sliders.set_margin(0, 0, 0, 8);
    content.add(&exp_sliders);

    let sl_stack = ui::StackPanel::vertical();
    sl_stack.set_position(0, 0);
    sl_stack.set_size(420, 120);
    sl_stack.set_padding(4, 4, 4, 4);
    exp_sliders.add(&sl_stack);

    // Volume slider row
    let vol_row = ui::View::new();
    vol_row.set_size(412, 20);
    vol_row.set_margin(0, 0, 0, 8);
    sl_stack.add(&vol_row);

    let sl_label = ui::Label::new("Volume");
    sl_label.set_position(0, 2);
    vol_row.add(&sl_label);

    let slider = ui::Slider::new(65);
    slider.set_position(70, 0);
    slider.set_size(340, 20);
    vol_row.add(&slider);

    // Progress bar row
    let prog_row = ui::View::new();
    prog_row.set_size(412, 12);
    prog_row.set_margin(0, 0, 0, 8);
    sl_stack.add(&prog_row);

    let pb_label = ui::Label::new("Progress");
    pb_label.set_position(0, 0);
    prog_row.add(&pb_label);

    let progress = ui::ProgressBar::new(65);
    progress.set_position(70, 2);
    progress.set_size(340, 8);
    prog_row.add(&progress);

    slider.on_value_changed(move |e| {
        progress.set_state(e.value);
    });

    // Stepper row
    let step_row = ui::View::new();
    step_row.set_size(412, 28);
    sl_stack.add(&step_row);

    let st_label = ui::Label::new("Qty");
    st_label.set_position(0, 6);
    step_row.add(&st_label);

    let stepper = ui::Stepper::new();
    stepper.set_position(70, 0);
    stepper.set_state(5);
    step_row.add(&stepper);

    // ════════════════════════════════════════════════════════════════
    //  Section 4: Segmented Control
    // ════════════════════════════════════════════════════════════════

    let exp_tabs = ui::Expander::new("Segmented Control");
    exp_tabs.set_size(420, 128); // 32 + 96
    exp_tabs.set_margin(0, 0, 0, 8);
    content.add(&exp_tabs);

    let tab_stack = ui::StackPanel::vertical();
    tab_stack.set_position(0, 0);
    tab_stack.set_size(420, 96);
    tab_stack.set_padding(4, 4, 4, 4);
    exp_tabs.add(&tab_stack);

    let seg = ui::SegmentedControl::new("General|Appearance|Privacy");
    seg.set_size(412, 28);
    seg.set_margin(0, 0, 0, 6);
    tab_stack.add(&seg);

    let panel_a = ui::View::new();
    panel_a.set_size(412, 40);
    tab_stack.add(&panel_a);
    let pa_lbl = ui::Label::new("General settings panel");
    pa_lbl.set_position(10, 10);
    panel_a.add(&pa_lbl);

    let panel_b = ui::View::new();
    panel_b.set_size(412, 40);
    tab_stack.add(&panel_b);
    let pb_lbl = ui::Label::new("Appearance settings panel");
    pb_lbl.set_position(10, 10);
    panel_b.add(&pb_lbl);

    let panel_c = ui::View::new();
    panel_c.set_size(412, 40);
    tab_stack.add(&panel_c);
    let pc_lbl = ui::Label::new("Privacy settings panel");
    pc_lbl.set_position(10, 10);
    panel_c.add(&pc_lbl);

    seg.connect_panels(&[&panel_a, &panel_b, &panel_c]);

    // ════════════════════════════════════════════════════════════════
    //  Section 5: Cards & Containers
    // ════════════════════════════════════════════════════════════════

    let exp_cards = ui::Expander::new("Cards & Containers");
    exp_cards.set_size(420, 110); // 32 + 78
    exp_cards.set_margin(0, 0, 0, 8);
    content.add(&exp_cards);

    let cards_row = ui::View::new();
    cards_row.set_position(0, 0);
    cards_row.set_size(420, 70);
    cards_row.set_padding(4, 4, 4, 4);
    exp_cards.add(&cards_row);

    // Card
    let card = ui::Card::new();
    card.set_position(0, 0);
    card.set_size(200, 60);
    cards_row.add(&card);

    let card_title = ui::Label::new("Card Widget");
    card_title.set_position(12, 8);
    card_title.set_color(0xFF007AFF);
    card.add(&card_title);

    let card_text = ui::Label::new("With nested content.");
    card_text.set_position(12, 30);
    card.add(&card_text);

    // GroupBox
    let gb = ui::GroupBox::new("Settings Group");
    gb.set_position(210, 0);
    gb.set_size(200, 60);
    cards_row.add(&gb);

    let gb_lbl = ui::Label::new("Grouped content");
    gb_lbl.set_position(10, 24);
    gb.add(&gb_lbl);

    // ════════════════════════════════════════════════════════════════
    //  Section 6: Color & Status
    // ════════════════════════════════════════════════════════════════

    let exp_misc = ui::Expander::new("Color & Status");
    exp_misc.set_size(420, 110); // 32 + 78
    exp_misc.set_margin(0, 0, 0, 8);
    content.add(&exp_misc);

    let misc_stack = ui::StackPanel::vertical();
    misc_stack.set_position(0, 0);
    misc_stack.set_size(420, 78);
    misc_stack.set_padding(4, 4, 4, 4);
    exp_misc.add(&misc_stack);

    // Color picker row
    let color_row = ui::View::new();
    color_row.set_size(412, 28);
    color_row.set_margin(0, 0, 0, 8);
    misc_stack.add(&color_row);

    let cw_label = ui::Label::new("Pick a color:");
    cw_label.set_position(0, 6);
    color_row.add(&cw_label);

    let cw = ui::ColorWell::new();
    cw.set_position(100, 0);
    cw.set_state(0xFF007AFF);
    color_row.add(&cw);

    let cw_swatch = ui::View::new();
    cw_swatch.set_position(160, 0);
    cw_swatch.set_size(28, 28);
    cw_swatch.set_color(0xFF007AFF);
    color_row.add(&cw_swatch);

    cw.on_color_selected(move |e| {
        cw_swatch.set_color(e.color);
    });

    // Status indicators row
    let status_row = ui::View::new();
    status_row.set_size(412, 20);
    misc_stack.add(&status_row);

    let si1 = ui::StatusIndicator::new("Connected");
    si1.set_position(0, 0);
    si1.set_state(1); // green
    status_row.add(&si1);

    let si2 = ui::StatusIndicator::new("Idle");
    si2.set_position(120, 0);
    si2.set_state(2); // yellow
    status_row.add(&si2);

    let si3 = ui::StatusIndicator::new("Offline");
    si3.set_position(200, 0);
    si3.set_state(0); // red
    status_row.add(&si3);

    // ════════════════════════════════════════════════════════════════
    //  Section 7: Canvas Drawing
    // ════════════════════════════════════════════════════════════════

    let exp_canvas = ui::Expander::new("Canvas Drawing");
    exp_canvas.set_size(420, 142); // 32 + 110
    exp_canvas.set_margin(0, 0, 0, 8);
    content.add(&exp_canvas);

    let canvas = ui::Canvas::new(412, 100);
    canvas.set_position(4, 4);
    canvas.clear(0xFF1C1C1E);
    // Draw some shapes
    canvas.fill_rect(10, 10, 80, 40, 0xFF007AFF);   // blue rect
    canvas.fill_rect(100, 10, 80, 40, 0xFF30D158);   // green rect
    canvas.fill_rect(190, 10, 80, 40, 0xFFFF453A);   // red rect
    canvas.draw_line(10, 70, 270, 70, 0xFFFFFFFF);    // white line
    canvas.draw_circle(320, 50, 30, 0xFFFF9F0A);      // orange circle
    canvas.fill_circle(380, 50, 20, 0xFFBF5AF2);      // purple filled circle
    exp_canvas.add(&canvas);

    // ════════════════════════════════════════════════════════════════
    //  Section 8: Context Menu Demo
    // ════════════════════════════════════════════════════════════════

    let ctx_label = ui::Label::new("Right-click the button for a context menu:");
    ctx_label.set_size(420, 16);
    ctx_label.set_margin(0, 4, 0, 4);
    content.add(&ctx_label);

    let ctx_btn = ui::Button::new("Right-Click Me");
    ctx_btn.set_size(160, 32);
    ctx_btn.set_margin(0, 0, 0, 8);
    content.add(&ctx_btn);

    let menu = ui::ContextMenu::new("Cut|Copy|Paste|Select All");
    menu.on_item_click(|e| {
        let item_name = match e.index {
            0 => "Cut",
            1 => "Copy",
            2 => "Paste",
            _ => "Select All",
        };
        ui::MessageBox::show(ui::MessageBoxType::Info, item_name, Some("OK"));
    });
    content.add(&menu);
    ctx_btn.set_context_menu(&menu);

    // ════════════════════════════════════════════════════════════════
    //  Footer
    // ════════════════════════════════════════════════════════════════

    let div2 = ui::Divider::new();
    div2.set_size(420, 1);
    div2.set_margin(0, 0, 0, 8);
    content.add(&div2);

    let footer = ui::Label::new("End of showcase - built with anyui");
    footer.set_color(0xFF5A5A5A);
    footer.set_size(420, 16);
    content.add(&footer);

    // ── Run event loop ──
    ui::run();
}
