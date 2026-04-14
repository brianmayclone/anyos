//! demo_anyui — Showcase of all anyui components.
//!
//! Demonstrates ScrollView, Expander, StackPanel, ContextMenu, Tooltips,
//! and every control type in an organized, scrollable layout.

#![no_std]
#![no_main]

use anyos_std::i18n;
use libanyui_client as ui;

anyos_std::entry!(main);

fn main() {
    if !ui::init() {
        anyos_std::println!("Failed to load libanyui.so");
        return;
    }
    i18n::init();

    let win = ui::Window::new(i18n::t("anyui Component Showcase"), -1, -1, 520, 620);

    let nav = ui::NavigationBar::new(i18n::t("anyui Component Showcase"));
    nav.set_position(0, 0);
    nav.set_size(520, 36);
    win.add(&nav);

    let tabs = ui::TabBar::new("Basics|Desktop|Advanced");
    tabs.set_position(0, 36);
    tabs.set_size(520, 34);
    win.add(&tabs);

    let controls_panel = ui::View::new();
    controls_panel.set_position(0, 72);
    controls_panel.set_size(520, 548);
    win.add(&controls_panel);

    let controls_scroll = ui::ScrollView::new();
    controls_scroll.set_position(0, 0);
    controls_scroll.set_size(520, 548);
    controls_scroll.set_dock(ui::DOCK_FILL);
    controls_panel.add(&controls_scroll);

    let controls_content = ui::StackPanel::vertical();
    controls_content.set_position(0, 0);
    controls_content.set_size(500, 1220);
    controls_content.set_padding(20, 10, 20, 20);
    controls_scroll.add(&controls_content);

    let desktop_panel = ui::View::new();
    desktop_panel.set_position(0, 72);
    desktop_panel.set_size(520, 548);
    win.add(&desktop_panel);

    let desktop_scroll = ui::ScrollView::new();
    desktop_scroll.set_position(0, 0);
    desktop_scroll.set_size(520, 548);
    desktop_scroll.set_dock(ui::DOCK_FILL);
    desktop_panel.add(&desktop_scroll);

    let desktop_content = ui::StackPanel::vertical();
    desktop_content.set_position(0, 0);
    desktop_content.set_size(500, 1100);
    desktop_content.set_padding(20, 10, 20, 20);
    desktop_scroll.add(&desktop_content);

    let misc_panel = ui::View::new();
    misc_panel.set_position(0, 72);
    misc_panel.set_size(520, 548);
    win.add(&misc_panel);

    let misc_scroll = ui::ScrollView::new();
    misc_scroll.set_position(0, 0);
    misc_scroll.set_size(520, 548);
    misc_scroll.set_dock(ui::DOCK_FILL);
    misc_panel.add(&misc_scroll);

    let misc_content = ui::StackPanel::vertical();
    misc_content.set_position(0, 0);
    misc_content.set_size(500, 520);
    misc_content.set_padding(20, 10, 20, 20);
    misc_scroll.add(&misc_content);

    tabs.connect_panels(&[&controls_panel, &desktop_panel, &misc_panel]);

    // ════════════════════════════════════════════════════════════════
    //  Header
    // ════════════════════════════════════════════════════════════════

    let title = ui::Label::new(i18n::t("anyui Component Showcase"));
    title.set_color(0xFF167CFF);
    title.set_text_color(0xFFFFFFFF);
    title.set_text_align(ui::TEXT_ALIGN_CENTER);
    title.set_size(340, 28);
    title.set_margin(40, 0, 40, 6);
    controls_content.add(&title);

    let subtitle = ui::Label::new(i18n::t("Desktop controls, layouts and data views in one place"));
    subtitle.set_color(0xFF707782);
    subtitle.set_text_color(0xFFE8ECF2);
    subtitle.set_text_align(ui::TEXT_ALIGN_CENTER);
    subtitle.set_size(460, 24);
    subtitle.set_margin(0, 0, 0, 10);
    controls_content.add(&subtitle);

    let div = ui::Divider::new();
    div.set_size(460, 1);
    div.set_margin(0, 0, 0, 8);
    controls_content.add(&div);

    // ════════════════════════════════════════════════════════════════
    //  Section 1: Buttons & Actions
    // ════════════════════════════════════════════════════════════════

    let exp_buttons = ui::Expander::new(i18n::t("Buttons & Actions"));
    exp_buttons.set_size(460, 82); // 32 header + 50 content
    exp_buttons.set_margin(0, 0, 0, 8);
    controls_content.add(&exp_buttons);

    // Horizontal row of button-like controls
    let row_btns = ui::FlowPanel::new();
    row_btns.set_position(0, 0);
    row_btns.set_size(460, 40);
    row_btns.set_padding(4, 4, 4, 4);
    exp_buttons.add(&row_btns);

    let btn = ui::Button::new(i18n::t("Primary"));
    btn.set_size(100, 32);
    btn.set_margin(0, 0, 6, 0);
    btn.set_color(0xFF167CFF);
    btn.set_tooltip(i18n::t("Show MessageBox"));
    btn.on_click(|_e| {
        ui::MessageBox::show(ui::MessageBoxType::Info, i18n::t("Button clicked!"), None);
    });
    row_btns.add(&btn);

    let icon_btn = ui::IconButton::new("*");
    icon_btn.set_size(32, 32);
    icon_btn.set_margin(0, 0, 6, 0);
    icon_btn.on_click(|_e| {
        ui::MessageBox::show(ui::MessageBoxType::Warning, "Starred!", Some("Cool"));
    });
    row_btns.add(&icon_btn);

    let tag1 = ui::Tag::new("Rust");
    tag1.set_color(0xFF167CFF);
    tag1.set_margin(0, 4, 4, 0);
    row_btns.add(&tag1);

    let tag2 = ui::Tag::new("anyOS");
    tag2.set_color(0xFF167CFF);
    tag2.set_margin(0, 4, 4, 0);
    row_btns.add(&tag2);

    let badge = ui::Badge::new("3");
    badge.set_color(0xFFE53935);
    badge.set_margin(0, 4, 8, 0);
    row_btns.add(&badge);

    let status = ui::StatusIndicator::new(i18n::t("Online"));
    status.set_margin(0, 6, 0, 0);
    row_btns.add(&status);

    // ════════════════════════════════════════════════════════════════
    //  Section 2: Input Controls
    // ════════════════════════════════════════════════════════════════

    let exp_inputs = ui::Expander::new(i18n::t("Input Controls"));
    exp_inputs.set_size(460, 420);
    exp_inputs.set_margin(0, 0, 0, 8);
    controls_content.add(&exp_inputs);

    let inp_stack = ui::StackPanel::vertical();
    inp_stack.set_position(0, 0);
    inp_stack.set_size(460, 388);
    inp_stack.set_padding(4, 4, 4, 4);
    exp_inputs.add(&inp_stack);

    // Toggle row
    let toggle_row = ui::View::new();
    toggle_row.set_size(452, 28);
    toggle_row.set_margin(0, 0, 0, 6);
    inp_stack.add(&toggle_row);

    let toggle_lbl = ui::Label::new(i18n::t("Dark Mode"));
    toggle_lbl.set_position(0, 4);
    toggle_row.add(&toggle_lbl);

    let toggle = ui::Toggle::new(true);
    toggle.set_position(100, 0);
    toggle_row.add(&toggle);

    // Checkbox
    let cb = ui::Checkbox::new(i18n::t("Enable notifications"));
    cb.set_size(220, 20);
    cb.set_margin(0, 0, 0, 6);
    cb.set_state(0);
    inp_stack.add(&cb);

    // Radio buttons row
    let radio_row = ui::View::new();
    radio_row.set_size(452, 20);
    radio_row.set_margin(0, 0, 0, 6);
    inp_stack.add(&radio_row);

    let rb1 = ui::RadioButton::new("Option A");
    rb1.set_position(0, 0);
    rb1.set_state(1);
    radio_row.add(&rb1);

    let rb2 = ui::RadioButton::new("Option B");
    rb2.set_position(120, 0);
    radio_row.add(&rb2);

    // Text inputs row
    let text_row = ui::View::new();
    text_row.set_size(452, 28);
    text_row.set_margin(0, 0, 0, 6);
    inp_stack.add(&text_row);

    let tf = ui::TextField::new();
    tf.set_position(0, 0);
    tf.set_size(220, 28);
    tf.set_text("Hello World");
    tf.set_placeholder("Regular text field");
    tf.set_tooltip(i18n::t("Type your name here"));
    text_row.add(&tf);

    let search = ui::SearchField::new();
    search.set_position(228, 0);
    search.set_size(220, 28);
    search.set_placeholder("Search");
    text_row.add(&search);

    let field_row = ui::View::new();
    field_row.set_size(452, 28);
    field_row.set_margin(0, 0, 0, 6);
    inp_stack.add(&field_row);

    let pwd = ui::TextField::new();
    pwd.set_position(0, 0);
    pwd.set_size(220, 28);
    pwd.set_placeholder("Password field");
    pwd.set_password_mode(true);
    pwd.set_text("secret42");
    field_row.add(&pwd);

    let readonly = ui::TextField::new();
    readonly.set_position(228, 0);
    readonly.set_size(220, 28);
    readonly.set_text("/Users/demo/Documents");
    readonly.set_read_only(true);
    readonly.set_tooltip(i18n::t("Read-only text fields still support selection and copy"));
    field_row.add(&readonly);

    let select_row = ui::View::new();
    select_row.set_size(452, 28);
    select_row.set_margin(0, 0, 0, 6);
    inp_stack.add(&select_row);

    let dd = ui::DropDown::new("System|Dark|Light");
    dd.set_position(0, 0);
    dd.set_size(150, 28);
    dd.set_selected_index(0);
    select_row.add(&dd);

    let combo = ui::ComboBox::new();
    combo.set_position(160, 0);
    combo.set_size(140, 28);
    combo.set_items("Stable|Beta|Nightly");
    combo.set_selected_index(Some(1));
    select_row.add(&combo);

    let combo_edit = ui::ComboBox::new();
    combo_edit.set_position(310, 0);
    combo_edit.set_size(142, 28);
    combo_edit.set_editable(true);
    combo_edit.set_placeholder("Tag or branch");
    combo_edit.set_items("main|release|feature/ui-refresh|fix/dnd");
    select_row.add(&combo_edit);

    let auto = ui::AutoCompleteTextField::new();
    auto.set_size(452, 28);
    auto.set_margin(0, 0, 0, 6);
    auto.set_placeholder("Autocomplete: type 'mi', 'an' or 'de'");
    auto.set_suggestions("Mike Strathmann|Anna Becker|Dennis Schulz|Mila Winter|Anya Dev");
    inp_stack.add(&auto);

    let ta = ui::TextArea::new();
    ta.set_size(452, 72);
    ta.set_margin(0, 0, 0, 6);
    ta.set_text("Editable text area with selection, drag selection and keyboard navigation.\nTry Ctrl+A, Shift+Arrow or copy/paste here.");
    inp_stack.add(&ta);

    let ta_readonly = ui::TextArea::new();
    ta_readonly.set_size(452, 56);
    ta_readonly.set_read_only(true);
    ta_readonly.set_text("Read-only text area.\nUseful for logs, paths, generated commands and diagnostics.");
    inp_stack.add(&ta_readonly);

    // ════════════════════════════════════════════════════════════════
    //  Section 3: Sliders & Progress
    // ════════════════════════════════════════════════════════════════

    let exp_sliders = ui::Expander::new(i18n::t("Sliders & Progress"));
    exp_sliders.set_size(460, 152); // 32 + 120
    exp_sliders.set_margin(0, 0, 0, 8);
    controls_content.add(&exp_sliders);

    let sl_stack = ui::StackPanel::vertical();
    sl_stack.set_position(0, 0);
    sl_stack.set_size(460, 120);
    sl_stack.set_padding(4, 4, 4, 4);
    exp_sliders.add(&sl_stack);

    // Volume slider row
    let vol_row = ui::View::new();
    vol_row.set_size(452, 20);
    vol_row.set_margin(0, 0, 0, 8);
    sl_stack.add(&vol_row);

    let sl_label = ui::Label::new(i18n::t("Volume"));
    sl_label.set_position(0, 2);
    vol_row.add(&sl_label);

    let slider = ui::Slider::new(65);
    slider.set_position(70, 0);
    slider.set_size(380, 20);
    vol_row.add(&slider);

    // Progress bar row
    let prog_row = ui::View::new();
    prog_row.set_size(452, 12);
    prog_row.set_margin(0, 0, 0, 8);
    sl_stack.add(&prog_row);

    let pb_label = ui::Label::new(i18n::t("Progress"));
    pb_label.set_position(0, 0);
    prog_row.add(&pb_label);

    let progress = ui::ProgressBar::new(65);
    progress.set_position(70, 2);
    progress.set_size(380, 8);
    prog_row.add(&progress);

    slider.on_value_changed(move |e| {
        progress.set_state(e.value);
    });

    // Stepper row
    let step_row = ui::View::new();
    step_row.set_size(452, 28);
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

    let exp_tabs = ui::Expander::new(i18n::t("Segmented Control"));
    exp_tabs.set_size(460, 128); // 32 + 96
    exp_tabs.set_margin(0, 0, 0, 8);
    controls_content.add(&exp_tabs);

    let tab_stack = ui::StackPanel::vertical();
    tab_stack.set_position(0, 0);
    tab_stack.set_size(460, 96);
    tab_stack.set_padding(4, 4, 4, 4);
    exp_tabs.add(&tab_stack);

    let seg_str = alloc::format!("{}|{}|{}", i18n::t("General"), i18n::t("Appearance"), i18n::t("Privacy"));
    let seg = ui::SegmentedControl::new(&seg_str);
    seg.set_size(452, 28);
    seg.set_margin(0, 0, 0, 6);
    tab_stack.add(&seg);

    let panel_a = ui::View::new();
    panel_a.set_size(452, 40);
    tab_stack.add(&panel_a);
    let pa_lbl = ui::Label::new(i18n::t("General settings panel"));
    pa_lbl.set_position(10, 10);
    panel_a.add(&pa_lbl);

    let panel_b = ui::View::new();
    panel_b.set_size(452, 40);
    tab_stack.add(&panel_b);
    let pb_lbl = ui::Label::new(i18n::t("Appearance settings panel"));
    pb_lbl.set_position(10, 10);
    panel_b.add(&pb_lbl);

    let panel_c = ui::View::new();
    panel_c.set_size(452, 40);
    tab_stack.add(&panel_c);
    let pc_lbl = ui::Label::new(i18n::t("Privacy settings panel"));
    pc_lbl.set_position(10, 10);
    panel_c.add(&pc_lbl);

    seg.connect_panels(&[&panel_a, &panel_b, &panel_c]);

    // ════════════════════════════════════════════════════════════════
    //  Section 5: Cards & Containers
    // ════════════════════════════════════════════════════════════════

    let exp_cards = ui::Expander::new(i18n::t("Cards & Containers"));
    exp_cards.set_size(460, 156);
    exp_cards.set_margin(0, 0, 0, 8);
    desktop_content.add(&exp_cards);

    let cards_row = ui::View::new();
    cards_row.set_position(0, 0);
    cards_row.set_size(460, 124);
    cards_row.set_padding(4, 4, 4, 4);
    exp_cards.add(&cards_row);

    let card = ui::Card::new();
    card.set_position(0, 0);
    card.set_size(220, 110);
    cards_row.add(&card);

    let card_title = ui::Label::new(i18n::t("Glass Card"));
    card_title.set_position(12, 8);
    card_title.set_size(196, 16);
    card_title.set_text_color(0xFF7FB7FF);
    card.add(&card_title);

    let card_text = ui::Label::new(i18n::t("Updated rounded corners and softer gloss gradient."));
    card_text.set_position(12, 30);
    card_text.set_size(196, 28);
    card.add(&card_text);

    let card_tag = ui::Tag::new("New");
    card_tag.set_position(12, 72);
    card_tag.set_color(0xFF24B04A);
    card.add(&card_tag);

    let gb = ui::GroupBox::new(i18n::t("Settings Group"));
    gb.set_position(230, 0);
    gb.set_size(220, 110);
    cards_row.add(&gb);

    let gb_lbl = ui::Label::new(i18n::t("GroupBox still works for denser, classic desktop forms."));
    gb_lbl.set_position(10, 26);
    gb_lbl.set_size(196, 28);
    gb.add(&gb_lbl);

    let gb_status = ui::StatusIndicator::new(i18n::t("Ready"));
    gb_status.set_position(10, 68);
    gb_status.set_state(1);
    gb.add(&gb_status);

    // ════════════════════════════════════════════════════════════════
    //  Section 6: Color & Status
    // ════════════════════════════════════════════════════════════════

    let exp_misc = ui::Expander::new(i18n::t("Color & Status"));
    exp_misc.set_size(460, 110); // 32 + 78
    exp_misc.set_margin(0, 0, 0, 8);
    desktop_content.add(&exp_misc);

    let misc_stack = ui::StackPanel::vertical();
    misc_stack.set_position(0, 0);
    misc_stack.set_size(460, 78);
    misc_stack.set_padding(4, 4, 4, 4);
    exp_misc.add(&misc_stack);

    // Color picker row
    let color_row = ui::View::new();
    color_row.set_size(452, 28);
    color_row.set_margin(0, 0, 0, 8);
    misc_stack.add(&color_row);

    let cw_label = ui::Label::new(i18n::t("Pick a color:"));
    cw_label.set_position(0, 6);
    color_row.add(&cw_label);

    let swatches = [
        0xFFFF3B30u32,
        0xFF167CFFu32,
        0xFF59C135u32,
        0xFF8E44D9u32,
        0xFF4B5260u32,
    ];
    for (idx, color) in swatches.iter().enumerate() {
        let sw = ui::ColorWell::new();
        sw.set_position(100 + (idx as i32 * 30), 2);
        sw.set_size(20, 20);
        sw.set_color(*color);
        color_row.add(&sw);
    }

    // Status indicators row
    let status_row = ui::View::new();
    status_row.set_size(452, 20);
    misc_stack.add(&status_row);

    let si1 = ui::StatusIndicator::new(i18n::t("Connected"));
    si1.set_position(0, 0);
    si1.set_state(1); // green
    status_row.add(&si1);

    let si2 = ui::StatusIndicator::new(i18n::t("Idle"));
    si2.set_position(120, 0);
    si2.set_state(2); // yellow
    status_row.add(&si2);

    let si3 = ui::StatusIndicator::new(i18n::t("Offline"));
    si3.set_position(200, 0);
    si3.set_state(0); // red
    status_row.add(&si3);

    // ════════════════════════════════════════════════════════════════
    //  Section 7: Date/Time Pickers & ListBox
    // ════════════════════════════════════════════════════════════════

    let exp_datetime = ui::Expander::new("Date/Time, Lists & Menus");
    exp_datetime.set_size(460, 230);
    exp_datetime.set_margin(0, 0, 0, 8);
    desktop_content.add(&exp_datetime);

    let dt_stack = ui::StackPanel::vertical();
    dt_stack.set_size(460, 190);
    dt_stack.set_padding(4, 4, 4, 4);
    exp_datetime.add(&dt_stack);

    // DatePicker row
    let date_row = ui::View::new();
    date_row.set_size(452, 28);
    date_row.set_margin(0, 0, 0, 6);
    dt_stack.add(&date_row);

    let date_label = ui::Label::new("Date:");
    date_label.set_position(0, 6);
    date_label.set_size(80, 20);
    date_row.add(&date_label);

    let date_picker = ui::DatePicker::new();
    date_picker.set_position(80, 0);
    date_picker.set_size(180, 28);
    date_picker.set_date(9, 4, 2026);
    date_row.add(&date_picker);

    // TimePicker row
    let time_row = ui::View::new();
    time_row.set_size(452, 28);
    time_row.set_margin(0, 0, 0, 6);
    dt_stack.add(&time_row);

    let time_label = ui::Label::new("Time:");
    time_label.set_position(0, 6);
    time_label.set_size(80, 20);
    time_row.add(&time_label);

    let time_picker = ui::TimePicker::new();
    time_picker.set_position(80, 0);
    time_picker.set_size(120, 28);
    time_picker.set_time(14, 30);
    time_row.add(&time_picker);

    // DateTimePicker row
    let datetime_row = ui::View::new();
    datetime_row.set_size(452, 28);
    datetime_row.set_margin(0, 0, 0, 6);
    dt_stack.add(&datetime_row);

    let datetime_label = ui::Label::new("DateTime:");
    datetime_label.set_position(0, 6);
    datetime_label.set_size(80, 20);
    datetime_row.add(&datetime_label);

    let datetime_picker = ui::DateTimePicker::new();
    datetime_picker.set_position(80, 0);
    datetime_picker.set_size(220, 28);
    datetime_picker.set_datetime(9, 4, 2026, 14, 30);
    datetime_row.add(&datetime_picker);

    // ListBox row
    let list_row = ui::View::new();
    list_row.set_size(452, 90);
    dt_stack.add(&list_row);

    let list_label = ui::Label::new("ListBox:");
    list_label.set_position(0, 0);
    list_label.set_size(80, 20);
    list_row.add(&list_label);

    let listbox = ui::ListBox::new("Apple|Banana|Cherry|Date|Elderberry|Fig|Grape");
    listbox.set_position(80, 0);
    listbox.set_size(200, 90);
    list_row.add(&listbox);

    let picker_mode = ui::DropDown::new("Fast install|Balanced|Safe mode");
    picker_mode.set_position(290, 28);
    picker_mode.set_size(150, 28);
    picker_mode.set_selected_index(1);
    list_row.add(&picker_mode);

    // ════════════════════════════════════════════════════════════════
    //  Section 8: Data Views & Split Layout
    // ════════════════════════════════════════════════════════════════

    let exp_data = ui::Expander::new(i18n::t("Data Views & Split Layout"));
    exp_data.set_size(460, 286);
    exp_data.set_margin(0, 0, 0, 8);
    desktop_content.add(&exp_data);

    let split = ui::SplitView::new();
    split.set_position(4, 4);
    split.set_size(452, 220);
    split.set_orientation(ui::ORIENTATION_HORIZONTAL);
    split.set_split_ratio(36);
    split.set_min_split(24);
    split.set_max_split(72);
    exp_data.add(&split);

    let left_panel = ui::View::new();
    left_panel.set_size(162, 220);
    split.add(&left_panel);

    let project_tree = ui::TreeView::new(162, 220);
    project_tree.set_position(0, 0);
    project_tree.set_size(162, 220);
    left_panel.add(&project_tree);

    let root = project_tree.add_root("demo_anyui");
    let src = project_tree.add_child(root, "src");
    project_tree.add_child(src, "main.rs");
    project_tree.add_child(root, "Cargo.toml");
    let assets = project_tree.add_child(root, "assets");
    project_tree.add_child(assets, "icon.ico");
    project_tree.set_expanded(root, true);
    project_tree.set_expanded(src, true);
    project_tree.set_selected(src);

    let right_panel = ui::View::new();
    right_panel.set_size(286, 220);
    split.add(&right_panel);

    let grid = ui::DataGrid::new(286, 182);
    grid.set_position(0, 0);
    grid.set_size(286, 182);
    grid.set_columns(&[
        ui::ColumnDef::new("File").width(118),
        ui::ColumnDef::new("State").width(74),
        ui::ColumnDef::new("Lines").width(70).align(ui::ALIGN_RIGHT).numeric(),
    ]);
    grid.set_data(&[
        alloc::vec!["main.rs", "Modified", "612"],
        alloc::vec!["theme.rs", "Staged", "534"],
        alloc::vec!["combobox.rs", "New", "418"],
        alloc::vec!["installer.log", "Read-only", "248"],
    ]);
    grid.set_selected_row(0);
    right_panel.add(&grid);

    let data_status = ui::Label::new(i18n::t("TreeView, SplitView and DataGrid are interactive."));
    data_status.set_position(0, 192);
    data_status.set_size(286, 20);
    right_panel.add(&data_status);

    let data_status_tree = data_status;
    project_tree.on_selection_changed(move |e| {
        let msg = alloc::format!("Tree selection changed: node {}", e.index);
        data_status_tree.set_text(&msg);
    });

    let data_status_grid = data_status;
    grid.on_selection_changed(move |e| {
        let msg = alloc::format!("Grid selection changed: row {}", e.index);
        data_status_grid.set_text(&msg);
    });

    // ════════════════════════════════════════════════════════════════
    //  Section 9: Context Menu Demo
    // ════════════════════════════════════════════════════════════════

    let ctx_label = ui::Label::new(i18n::t("Right-click the button for a context menu:"));
    ctx_label.set_size(460, 16);
    ctx_label.set_margin(0, 4, 0, 4);
    misc_content.add(&ctx_label);

    let ctx_btn = ui::Button::new(i18n::t("Right-Click Me"));
    ctx_btn.set_size(160, 32);
    ctx_btn.set_margin(0, 0, 0, 8);
    misc_content.add(&ctx_btn);

    let menu_str = alloc::format!(
        "{}|\u{1d}{}|{}|-|{}",
        i18n::t("Open"),
        i18n::t("Rename (Coming Soon)"),
        i18n::t("Copy"),
        i18n::t("Select All")
    );
    let menu = ui::ContextMenu::new(&menu_str);
    menu.on_item_click(|e| {
        let item_name = match e.index {
            0 => "Open",
            2 => "Copy",
            4 => "Select All",
            _ => return,
        };
        ui::MessageBox::show(ui::MessageBoxType::Info, item_name, Some("OK"));
    });
    misc_content.add(&menu);
    ctx_btn.set_context_menu(&menu);

    // ════════════════════════════════════════════════════════════════
    //  Section 10: Canvas Drawing
    // ════════════════════════════════════════════════════════════════

    let exp_canvas = ui::Expander::new(i18n::t("Canvas Drawing"));
    exp_canvas.set_size(460, 142); // 32 + 110
    exp_canvas.set_margin(0, 0, 0, 8);
    misc_content.add(&exp_canvas);

    let canvas = ui::Canvas::new(452, 100);
    canvas.set_position(4, 4);
    canvas.clear(0xFF1F242D);
    canvas.fill_rect(10, 10, 80, 40, 0xFF167CFF);
    canvas.fill_rect(100, 10, 80, 40, 0xFF24B04A);
    canvas.fill_rect(190, 10, 80, 40, 0xFFD92C36);
    canvas.draw_line(10, 70, 300, 70, 0xFFD8DEE8);
    canvas.fill_circle(350, 50, 22, 0xFF7A35D8);
    exp_canvas.add(&canvas);

    // ════════════════════════════════════════════════════════════════
    //  Footer
    // ════════════════════════════════════════════════════════════════

    let div2 = ui::Divider::new();
    div2.set_size(460, 1);
    div2.set_margin(0, 0, 0, 8);
    misc_content.add(&div2);

    let footer = ui::Label::new(i18n::t("End of showcase - built with anyui"));
    footer.set_color(0xFF5A5A5A);
    footer.set_size(460, 16);
    misc_content.add(&footer);

    // ── Run event loop ──
    ui::run();
}
