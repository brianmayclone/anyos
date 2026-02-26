//! Built-in: Display settings (GPU, Resolution, Brightness).

use alloc::format;
use alloc::vec::Vec;
use anyos_std::ui::window;
use libanyui_client as ui;
use ui::Widget;

/// Build the Display settings panel. Returns the panel View ID.
pub fn build(parent: &ui::ScrollView) -> u32 {
    let panel = ui::View::new();
    panel.set_dock(ui::DOCK_FILL);
    panel.set_color(0xFF1E1E1E);

    // Title
    let title = ui::Label::new("Display");
    title.set_dock(ui::DOCK_TOP);
    title.set_size(560, 36);
    title.set_font_size(18);
    title.set_text_color(0xFFFFFFFF);
    title.set_margin(16, 12, 16, 4);
    panel.add(&title);

    // Info card
    let card = ui::Card::new();
    card.set_dock(ui::DOCK_TOP);
    card.set_size(560, 120);
    card.set_margin(16, 8, 16, 8);
    card.set_color(0xFF2D2D30);

    // GPU Driver
    let row1 = ui::View::new();
    row1.set_dock(ui::DOCK_TOP);
    row1.set_size(528, 36);
    row1.set_margin(16, 8, 16, 0);
    let lbl = ui::Label::new("GPU Driver");
    lbl.set_position(0, 8);
    lbl.set_size(120, 20);
    lbl.set_text_color(0xFFCCCCCC);
    lbl.set_font_size(13);
    row1.add(&lbl);
    let gpu = window::gpu_name();
    let val = ui::Label::new(&gpu);
    val.set_position(130, 8);
    val.set_size(380, 20);
    val.set_text_color(0xFF969696);
    val.set_font_size(13);
    row1.add(&val);
    card.add(&row1);

    // Current resolution
    let row2 = ui::View::new();
    row2.set_dock(ui::DOCK_TOP);
    row2.set_size(528, 36);
    row2.set_margin(16, 0, 16, 0);
    let lbl2 = ui::Label::new("Resolution");
    lbl2.set_position(0, 8);
    lbl2.set_size(120, 20);
    lbl2.set_text_color(0xFFCCCCCC);
    lbl2.set_font_size(13);
    row2.add(&lbl2);
    let (sw, sh) = window::screen_size();
    let res_str = format!("{} x {}", sw, sh);
    let val2 = ui::Label::new(&res_str);
    val2.set_position(130, 8);
    val2.set_size(200, 20);
    val2.set_text_color(0xFF969696);
    val2.set_font_size(13);
    row2.add(&val2);
    card.add(&row2);

    panel.add(&card);

    // Resolution picker card
    let resolutions = window::list_resolutions();
    if !resolutions.is_empty() {
        let res_card = ui::Card::new();
        res_card.set_dock(ui::DOCK_TOP);
        let card_h = 44 + resolutions.len() as u32 * 30;
        res_card.set_size(560, card_h);
        res_card.set_margin(16, 8, 16, 8);
        res_card.set_color(0xFF2D2D30);

        let hdr = ui::Label::new("Change Resolution");
        hdr.set_dock(ui::DOCK_TOP);
        hdr.set_size(528, 32);
        hdr.set_font_size(13);
        hdr.set_text_color(0xFFCCCCCC);
        hdr.set_margin(16, 8, 16, 0);
        res_card.add(&hdr);

        let (cur_w, cur_h) = window::screen_size();
        let rg = ui::RadioGroup::new();
        rg.set_dock(ui::DOCK_TOP);
        let rg_h = resolutions.len() as u32 * 28;
        rg.set_size(528, rg_h);
        rg.set_margin(16, 4, 16, 8);

        // Create individual RadioButton controls for each resolution
        for &(rw, rh) in resolutions.iter() {
            let label = format!("{} x {}", rw, rh);
            let rb = ui::RadioButton::new(&label);
            rb.set_size(500, 24);
            rb.set_font_size(13);
            if rw == cur_w && rh == cur_h {
                rb.set_state(1);
            }
            rg.add(&rb);
        }

        // Clone resolutions for the closure
        let res_copy: Vec<(u32, u32)> = resolutions.clone();
        rg.on_selection_changed(move |e| {
            let idx = e.index as usize;
            if idx < res_copy.len() {
                let (rw, rh) = res_copy[idx];
                window::set_resolution(rw, rh);
            }
        });

        res_card.add(&rg);
        panel.add(&res_card);
    }

    parent.add(&panel);
    panel.id()
}
