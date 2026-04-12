use super::*;

impl Renderer {
    fn emit_checkbox_control(
        &mut self,
        kind: FormFieldKind,
        bx: &LayoutBox,
        x: i32,
        y: i32,
        parent: &ui::View,
    ) {
        let node_id = bx.node_id.unwrap_or(0);
        let w = bx.width;
        let h = bx.height;
        if bx.appearance_none {
            self.track_canvas_only_control(kind, node_id, x, y, w, h, HitKind::Checkbox(node_id));
            return;
        }

        let accent = self.effective_accent_color(bx);
        if let Some(fc) = self.find_control_mut(node_id, kind) {
            let ctrl = ui::Control::from_id(fc.control_id);
            ctrl.set_position(x, y);
            ctrl.set_size(w as u32, h as u32);
            ctrl.set_color(accent);
            ctrl.set_state(if bx.form_checked { 1 } else { 0 });
            ctrl.set_enabled(!bx.form_disabled);
            Self::update_control_bounds(fc, x, y, w, h);
            return;
        }

        let cb = ui::Checkbox::new("");
        cb.set_position(x, y);
        cb.set_size(w as u32, h as u32);
        cb.set_color(accent);
        cb.set_state(if bx.form_checked { 1 } else { 0 });
        cb.set_enabled(!bx.form_disabled);
        parent.add(&cb);
        self.push_form_control(cb.id(), node_id, kind, x, y, w, h);
    }

    fn emit_radio_control(
        &mut self,
        kind: FormFieldKind,
        bx: &LayoutBox,
        x: i32,
        y: i32,
        parent: &ui::View,
    ) {
        let node_id = bx.node_id.unwrap_or(0);
        let w = bx.width;
        let h = bx.height;
        if bx.appearance_none {
            self.track_canvas_only_control(kind, node_id, x, y, w, h, HitKind::Radio(node_id));
            return;
        }

        let accent = self.effective_accent_color(bx);
        if let Some(fc) = self.find_control_mut(node_id, kind) {
            let ctrl = ui::Control::from_id(fc.control_id);
            ctrl.set_position(x, y);
            ctrl.set_size(w as u32, h as u32);
            ctrl.set_color(accent);
            ctrl.set_state(if bx.form_checked { 1 } else { 0 });
            ctrl.set_enabled(!bx.form_disabled);
            Self::update_control_bounds(fc, x, y, w, h);
            return;
        }

        let rb = ui::RadioButton::new("");
        rb.set_position(x, y);
        rb.set_size(w as u32, h as u32);
        rb.set_color(accent);
        rb.set_state(if bx.form_checked { 1 } else { 0 });
        rb.set_enabled(!bx.form_disabled);
        parent.add(&rb);
        self.push_form_control(rb.id(), node_id, kind, x, y, w, h);
    }

    fn emit_select_control(
        &mut self,
        kind: FormFieldKind,
        bx: &LayoutBox,
        x: i32,
        y: i32,
        parent: &ui::View,
    ) {
        let node_id = bx.node_id.unwrap_or(0);
        let w = bx.width;
        let h = bx.height;
        if bx.appearance_none && !bx.form_multiple && bx.form_size <= 1 {
            self.track_canvas_only_control(kind, node_id, x, y, w, h, HitKind::Select(node_id));
            return;
        }

        let bg = self.default_control_bg(bx);
        let fg = self.default_control_fg(bx);
        let use_listbox = bx.form_multiple || bx.form_size > 1;
        if let Some(fc) = self.find_control_mut(node_id, kind) {
            let ctrl = ui::Control::from_id(fc.control_id);
            ctrl.set_position(x, y);
            ctrl.set_size(w as u32, h as u32);
            ctrl.set_color(bg);
            ctrl.set_text_color(fg);
            ctrl.set_enabled(!bx.form_disabled);
            Self::update_control_bounds(fc, x, y, w, h);
            return;
        }

        let items = bx.form_options.as_deref().unwrap_or("");
        let id = if use_listbox {
            let prefix = if bx.form_multiple { "multi:" } else { "" };
            let mut lb_items = String::from(prefix);
            lb_items.push_str(items);
            let lb = ui::ListBox::new(&lb_items);
            lb.set_position(x, y);
            lb.set_size(w as u32, h as u32);
            lb.set_color(bg);
            lb.set_text_color(fg);
            if bx.form_selected_index >= 0 {
                lb.set_selected_index(bx.form_selected_index as u32);
            }
            lb.set_enabled(!bx.form_disabled);
            parent.add(&lb);
            lb.id()
        } else {
            let dd = ui::DropDown::new(items);
            dd.set_position(x, y);
            dd.set_size(w as u32, h as u32);
            dd.set_color(bg);
            dd.set_text_color(fg);
            if bx.form_selected_index >= 0 {
                dd.set_selected_index(bx.form_selected_index as u32);
            }
            dd.set_enabled(!bx.form_disabled);
            parent.add(&dd);
            dd.id()
        };

        self.push_form_control(id, node_id, kind, x, y, w, h);
    }

    fn emit_range_control(
        &mut self,
        kind: FormFieldKind,
        bx: &LayoutBox,
        x: i32,
        y: i32,
        parent: &ui::View,
    ) {
        let node_id = bx.node_id.unwrap_or(0);
        let w = bx.width;
        let h = bx.height;
        if bx.appearance_none {
            self.track_canvas_only_control(kind, node_id, x, y, w, h, HitKind::Range(node_id));
            return;
        }

        let accent = self.effective_accent_color(bx);
        if let Some(fc) = self.find_control_mut(node_id, kind) {
            let ctrl = ui::Control::from_id(fc.control_id);
            ctrl.set_position(x, y);
            ctrl.set_size(w as u32, h as u32);
            ctrl.set_color(accent);
            ctrl.set_enabled(!bx.form_disabled);
            Self::update_control_bounds(fc, x, y, w, h);
            return;
        }

        let pct_i: u32 = bx
            .form_value
            .as_deref()
            .and_then(|s| {
                if s == "X" {
                    Some(100u32)
                } else {
                    s.parse::<u32>().ok().map(|v| v / 10)
                }
            })
            .unwrap_or(50);
        let slider = ui::Slider::new(pct_i);
        slider.set_position(x, y);
        slider.set_size(w as u32, h as u32);
        slider.set_color(accent);
        slider.set_enabled(!bx.form_disabled);
        parent.add(&slider);
        self.push_form_control(slider.id(), node_id, kind, x, y, w, h);
    }

    fn emit_color_control(
        &mut self,
        kind: FormFieldKind,
        bx: &LayoutBox,
        x: i32,
        y: i32,
        parent: &ui::View,
    ) {
        let node_id = bx.node_id.unwrap_or(0);
        let w = bx.width;
        let h = bx.height;
        if bx.appearance_none {
            self.track_canvas_only_control(kind, node_id, x, y, w, h, HitKind::ColorInput(node_id));
            return;
        }

        if let Some(fc) = self.find_control_mut(node_id, kind) {
            let ctrl = ui::Control::from_id(fc.control_id);
            ctrl.set_position(x, y);
            ctrl.set_size(w as u32, h as u32);
            Self::update_control_bounds(fc, x, y, w, h);
            return;
        }

        let cw = ui::ColorWell::new();
        cw.set_position(x, y);
        cw.set_size(w as u32, h as u32);
        let val = bx.form_value.as_deref().unwrap_or("#000000");
        let color = parse_color_value(val);
        cw.set_selected_color(color);
        if bx.form_disabled {
            cw.set_enabled(false);
        }
        parent.add(&cw);
        self.push_form_control(cw.id(), node_id, kind, x, y, w, h);
    }
}
