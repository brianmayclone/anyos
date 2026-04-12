use super::*;

impl Renderer {
    fn register_control_selector_events(&self, control_id: u32) {
        let Some(cb) = self.link_cb else {
            return;
        };
        let ctrl = ui::Control::from_id(control_id);
        #[cfg(not(feature = "host"))]
        {
            ctrl.on_focus_raw(cb, self.link_cb_ud);
            ctrl.on_blur_raw(cb, self.link_cb_ud);
            ctrl.on_event_raw(ui::EVENT_MOUSE_ENTER, cb, self.link_cb_ud);
            ctrl.on_event_raw(ui::EVENT_MOUSE_LEAVE, cb, self.link_cb_ud);
        }
        ctrl.on_event_raw(ui::EVENT_MOUSE_DOWN, cb, self.link_cb_ud);
        ctrl.on_event_raw(ui::EVENT_MOUSE_UP, cb, self.link_cb_ud);
    }

    // ─────────────────────────────────────────────────────────────────────
    // Walk: form controls + hit regions (unchanged)
    // ─────────────────────────────────────────────────────────────────────

    pub(super) fn walk_controls(
        &mut self,
        bx: &LayoutBox,
        offset_x: i32,
        offset_y: i32,
        parent: &ui::View,
        submit_cb: Option<ui::Callback>,
        submit_cb_ud: u64,
    ) {
        if bx.visibility_hidden {
            return;
        }

        let (abs_x, abs_y) = if bx.is_fixed {
            (bx.x, bx.y)
        } else {
            (offset_x + bx.x, offset_y + bx.y)
        };

        if let Some(ref text) = bx.text {
            if !text.is_empty() && bx.form_field.is_none() {
                if let Some(ref url) = bx.link_url {
                    self.hit_regions.push(HitRegion {
                        x: abs_x,
                        y: abs_y,
                        w: bx.width,
                        h: bx.height,
                        kind: HitKind::Link(url.clone()),
                    });
                }
            }
        }

        if let Some(kind) = bx.form_field {
            self.emit_form_control(kind, bx, abs_x, abs_y, parent, submit_cb, submit_cb_ud);
        }

        for child in &bx.children {
            self.walk_controls(child, abs_x, abs_y, parent, submit_cb, submit_cb_ud);
        }
    }

    pub(super) fn walk_controls_visible(
        &mut self,
        bx: &LayoutBox,
        offset_x: i32,
        offset_y: i32,
        parent: &ui::View,
        submit_cb: Option<ui::Callback>,
        submit_cb_ud: u64,
        visible_y_start: i32,
        visible_y_end: i32,
    ) {
        if bx.visibility_hidden {
            return;
        }

        let (abs_x, abs_y) = if bx.is_fixed {
            (bx.x, bx.y)
        } else {
            (offset_x + bx.x, offset_y + bx.y)
        };

        if !bx.subtree_has_viewport_positioned {
            let subtree_abs_top = abs_y + bx.subtree_top;
            let subtree_abs_bottom = abs_y + bx.subtree_bottom;
            if subtree_abs_bottom <= visible_y_start || subtree_abs_top >= visible_y_end {
                return;
            }
        }

        let overlaps_visible_band =
            abs_y < visible_y_end && abs_y + bx.height.max(1) > visible_y_start;

        if overlaps_visible_band {
            if let Some(ref text) = bx.text {
                if !text.is_empty() && bx.form_field.is_none() {
                    if let Some(ref url) = bx.link_url {
                        self.hit_regions.push(HitRegion {
                            x: abs_x,
                            y: abs_y,
                            w: bx.width,
                            h: bx.height,
                            kind: HitKind::Link(url.clone()),
                        });
                    }
                }
            }

            if let Some(kind) = bx.form_field {
                self.emit_form_control(kind, bx, abs_x, abs_y, parent, submit_cb, submit_cb_ud);
            }
        }

        for child in &bx.children {
            self.walk_controls_visible(
                child,
                abs_x,
                abs_y,
                parent,
                submit_cb,
                submit_cb_ud,
                visible_y_start,
                visible_y_end,
            );
        }
    }

    fn emit_form_control(
        &mut self,
        kind: FormFieldKind,
        bx: &LayoutBox,
        x: i32,
        y: i32,
        parent: &ui::View,
        submit_cb: Option<ui::Callback>,
        submit_cb_ud: u64,
    ) {
        let node_id = bx.node_id.unwrap_or(0);

        let w = bx.width;
        let h = bx.height;

        match kind {
            FormFieldKind::TextInput | FormFieldKind::Password => {
                let bg = self.default_control_bg(bx);
                let fg = self.default_control_fg(bx);
                if let Some(fc) = self
                    .form_controls
                    .iter_mut()
                    .find(|fc| fc.node_id == node_id && fc.kind == kind)
                {
                    let ctrl = ui::Control::from_id(fc.control_id);
                    ctrl.set_position(x, y);
                    ctrl.set_size(w as u32, h as u32);
                    ctrl.set_color(bg);
                    ctrl.set_text_color(fg);
                    ctrl.set_enabled(!bx.form_disabled);
                    fc.seen = true;
                    fc.doc_x = x;
                    fc.doc_y = y;
                    fc.doc_w = w;
                    fc.doc_h = h;
                } else {
                    // If this input has a datalist, use AutoCompleteTextField.
                    let id = if let Some(ref suggestions) = bx.form_datalist {
                        let atf = ui::AutoCompleteTextField::new();
                        atf.set_position(x, y);
                        atf.set_size(w as u32, h as u32);
                        atf.set_color(bg);
                        atf.set_text_color(fg);
                        atf.set_enabled(!bx.form_disabled);
                        atf.set_suggestions(suggestions);
                        if let Some(ref ph) = bx.form_placeholder {
                            atf.set_placeholder(ph);
                        }
                        if let Some(ref val) = bx.form_value {
                            atf.set_text(val);
                        }
                        if let Some(cb) = submit_cb {
                            atf.on_submit_raw(cb, submit_cb_ud);
                        }
                        parent.add(&atf);
                        atf.id()
                    } else {
                        let tf = ui::TextField::new();
                        if kind == FormFieldKind::Password {
                            tf.set_password_mode(true);
                        }
                        tf.set_position(x, y);
                        tf.set_size(w as u32, h as u32);
                        tf.set_color(bg);
                        tf.set_text_color(fg);
                        tf.set_enabled(!bx.form_disabled);
                        if let Some(ref ph) = bx.form_placeholder {
                            tf.set_placeholder(ph);
                        }
                        if let Some(ref val) = bx.form_value {
                            tf.set_text(val);
                        }
                        if let Some(cb) = submit_cb {
                            tf.on_submit_raw(cb, submit_cb_ud);
                        }
                        parent.add(&tf);
                        tf.id()
                    };
                    let id = id;

                    self.form_controls.push(FormControl {
                        control_id: id,
                        node_id,
                        kind,
                        name: String::new(),
                        seen: true,
                        doc_x: x,
                        doc_y: y,
                        doc_w: w,
                        doc_h: h,
                    });
                }
            }

            FormFieldKind::Submit | FormFieldKind::ButtonEl => {
                self.hit_regions.push(HitRegion {
                    x,
                    y,
                    w,
                    h,
                    kind: HitKind::Submit(node_id),
                });
            }

            FormFieldKind::Reset => {
                self.hit_regions.push(HitRegion {
                    x,
                    y,
                    w,
                    h,
                    kind: HitKind::Reset(node_id),
                });
            }

            FormFieldKind::Checkbox => {
                if bx.appearance_none {
                    self.hit_regions.push(HitRegion {
                        x,
                        y,
                        w,
                        h,
                        kind: HitKind::Checkbox(node_id),
                    });
                    if let Some(fc) = self
                        .form_controls
                        .iter_mut()
                        .find(|fc| fc.node_id == node_id && fc.kind == kind)
                    {
                        if fc.control_id != 0 {
                            ui::Control::from_id(fc.control_id).remove();
                            fc.control_id = 0;
                        }
                        fc.seen = true;
                        fc.doc_x = x;
                        fc.doc_y = y;
                        fc.doc_w = w;
                        fc.doc_h = h;
                    } else {
                        self.form_controls.push(FormControl {
                            control_id: 0,
                            node_id,
                            kind,
                            name: String::new(),
                            seen: true,
                            doc_x: x,
                            doc_y: y,
                            doc_w: w,
                            doc_h: h,
                        });
                    }
                    return;
                }
                let accent = self.effective_accent_color(bx);
                if let Some(fc) = self
                    .form_controls
                    .iter_mut()
                    .find(|fc| fc.node_id == node_id && fc.kind == kind)
                {
                    let ctrl = ui::Control::from_id(fc.control_id);
                    ctrl.set_position(x, y);
                    ctrl.set_size(w as u32, h as u32);
                    ctrl.set_color(accent);
                    ctrl.set_state(if bx.form_checked { 1 } else { 0 });
                    ctrl.set_enabled(!bx.form_disabled);
                    fc.seen = true;
                    fc.doc_x = x;
                    fc.doc_y = y;
                    fc.doc_w = w;
                    fc.doc_h = h;
                } else {
                    let cb = ui::Checkbox::new("");
                    cb.set_position(x, y);
                    cb.set_size(w as u32, h as u32);
                    cb.set_color(accent);
                    cb.set_state(if bx.form_checked { 1 } else { 0 });
                    cb.set_enabled(!bx.form_disabled);
                    parent.add(&cb);
                    let id = cb.id();

                    self.form_controls.push(FormControl {
                        control_id: id,
                        node_id,
                        kind,
                        name: String::new(),
                        seen: true,
                        doc_x: x,
                        doc_y: y,
                        doc_w: w,
                        doc_h: h,
                    });
                }
            }

            FormFieldKind::Radio => {
                if bx.appearance_none {
                    self.hit_regions.push(HitRegion {
                        x,
                        y,
                        w,
                        h,
                        kind: HitKind::Radio(node_id),
                    });
                    if let Some(fc) = self
                        .form_controls
                        .iter_mut()
                        .find(|fc| fc.node_id == node_id && fc.kind == kind)
                    {
                        if fc.control_id != 0 {
                            ui::Control::from_id(fc.control_id).remove();
                            fc.control_id = 0;
                        }
                        fc.seen = true;
                        fc.doc_x = x;
                        fc.doc_y = y;
                        fc.doc_w = w;
                        fc.doc_h = h;
                    } else {
                        self.form_controls.push(FormControl {
                            control_id: 0,
                            node_id,
                            kind,
                            name: String::new(),
                            seen: true,
                            doc_x: x,
                            doc_y: y,
                            doc_w: w,
                            doc_h: h,
                        });
                    }
                    return;
                }
                let accent = self.effective_accent_color(bx);
                if let Some(fc) = self
                    .form_controls
                    .iter_mut()
                    .find(|fc| fc.node_id == node_id && fc.kind == kind)
                {
                    let ctrl = ui::Control::from_id(fc.control_id);
                    ctrl.set_position(x, y);
                    ctrl.set_size(w as u32, h as u32);
                    ctrl.set_color(accent);
                    ctrl.set_state(if bx.form_checked { 1 } else { 0 });
                    ctrl.set_enabled(!bx.form_disabled);
                    fc.seen = true;
                    fc.doc_x = x;
                    fc.doc_y = y;
                    fc.doc_w = w;
                    fc.doc_h = h;
                } else {
                    let rb = ui::RadioButton::new("");
                    rb.set_position(x, y);
                    rb.set_size(w as u32, h as u32);
                    rb.set_color(accent);
                    rb.set_state(if bx.form_checked { 1 } else { 0 });
                    rb.set_enabled(!bx.form_disabled);
                    parent.add(&rb);
                    let id = rb.id();

                    self.form_controls.push(FormControl {
                        control_id: id,
                        node_id,
                        kind,
                        name: String::new(),
                        seen: true,
                        doc_x: x,
                        doc_y: y,
                        doc_w: w,
                        doc_h: h,
                    });
                }
            }

            FormFieldKind::Textarea => {
                let bg = self.default_control_bg(bx);
                let fg = self.default_control_fg(bx);
                if let Some(fc) = self
                    .form_controls
                    .iter_mut()
                    .find(|fc| fc.node_id == node_id && fc.kind == kind)
                {
                    let ctrl = ui::Control::from_id(fc.control_id);
                    ctrl.set_position(x, y);
                    ctrl.set_size(w as u32, h as u32);
                    ctrl.set_color(bg);
                    ctrl.set_text_color(fg);
                    ctrl.set_enabled(!bx.form_disabled);
                    fc.seen = true;
                    fc.doc_x = x;
                    fc.doc_y = y;
                    fc.doc_w = w;
                    fc.doc_h = h;
                } else {
                    let ta = ui::TextArea::new();
                    ta.set_position(x, y);
                    ta.set_size(w as u32, h as u32);
                    ta.set_color(bg);
                    ta.set_text_color(fg);
                    ta.set_enabled(!bx.form_disabled);
                    parent.add(&ta);
                    let id = ta.id();

                    self.form_controls.push(FormControl {
                        control_id: id,
                        node_id,
                        kind,
                        name: String::new(),
                        seen: true,
                        doc_x: x,
                        doc_y: y,
                        doc_w: w,
                        doc_h: h,
                    });
                }
            }

            FormFieldKind::Hidden => {
                if !self
                    .form_controls
                    .iter()
                    .any(|fc| fc.node_id == node_id && fc.kind == kind)
                {
                    self.form_controls.push(FormControl {
                        control_id: 0,
                        node_id,
                        kind,
                        name: String::new(),
                        seen: true,
                        doc_x: x,
                        doc_y: y,
                        doc_w: w,
                        doc_h: h,
                    });
                } else {
                    if let Some(fc) = self
                        .form_controls
                        .iter_mut()
                        .find(|fc| fc.node_id == node_id && fc.kind == kind)
                    {
                        fc.seen = true;
                    }
                }
            }

            FormFieldKind::Select => {
                if bx.appearance_none && !bx.form_multiple && bx.form_size <= 1 {
                    self.hit_regions.push(HitRegion {
                        x,
                        y,
                        w,
                        h,
                        kind: HitKind::Select(node_id),
                    });
                    if let Some(fc) = self
                        .form_controls
                        .iter_mut()
                        .find(|fc| fc.node_id == node_id && fc.kind == kind)
                    {
                        if fc.control_id != 0 {
                            ui::Control::from_id(fc.control_id).remove();
                            fc.control_id = 0;
                        }
                        fc.seen = true;
                        fc.doc_x = x;
                        fc.doc_y = y;
                        fc.doc_w = w;
                        fc.doc_h = h;
                    } else {
                        self.form_controls.push(FormControl {
                            control_id: 0,
                            node_id,
                            kind,
                            name: String::new(),
                            seen: true,
                            doc_x: x,
                            doc_y: y,
                            doc_w: w,
                            doc_h: h,
                        });
                    }
                    return;
                }
                let bg = self.default_control_bg(bx);
                let fg = self.default_control_fg(bx);
                let use_listbox = bx.form_multiple || bx.form_size > 1;
                if let Some(fc) = self
                    .form_controls
                    .iter_mut()
                    .find(|fc| fc.node_id == node_id && fc.kind == kind)
                {
                    let ctrl = ui::Control::from_id(fc.control_id);
                    ctrl.set_position(x, y);
                    ctrl.set_size(w as u32, h as u32);
                    ctrl.set_color(bg);
                    ctrl.set_text_color(fg);
                    ctrl.set_enabled(!bx.form_disabled);
                    fc.seen = true;
                    fc.doc_x = x;
                    fc.doc_y = y;
                    fc.doc_w = w;
                    fc.doc_h = h;
                } else {
                    let items = bx.form_options.as_deref().unwrap_or("");
                    let id = if use_listbox {
                        // Multi-select or size>1: use ListBox.
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
                        // Single-select dropdown.
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

                    self.form_controls.push(FormControl {
                        control_id: id,
                        node_id,
                        kind,
                        name: String::new(),
                        seen: true,
                        doc_x: x,
                        doc_y: y,
                        doc_w: w,
                        doc_h: h,
                    });
                }
            }

            FormFieldKind::Range => {
                if bx.appearance_none {
                    self.hit_regions.push(HitRegion {
                        x,
                        y,
                        w,
                        h,
                        kind: HitKind::Range(node_id),
                    });
                    if let Some(fc) = self
                        .form_controls
                        .iter_mut()
                        .find(|fc| fc.node_id == node_id && fc.kind == kind)
                    {
                        if fc.control_id != 0 {
                            ui::Control::from_id(fc.control_id).remove();
                            fc.control_id = 0;
                        }
                        fc.seen = true;
                        fc.doc_x = x;
                        fc.doc_y = y;
                        fc.doc_w = w;
                        fc.doc_h = h;
                    } else {
                        self.form_controls.push(FormControl {
                            control_id: 0,
                            node_id,
                            kind,
                            name: String::new(),
                            seen: true,
                            doc_x: x,
                            doc_y: y,
                            doc_w: w,
                            doc_h: h,
                        });
                    }
                    return;
                }
                let accent = self.effective_accent_color(bx);
                if let Some(fc) = self
                    .form_controls
                    .iter_mut()
                    .find(|fc| fc.node_id == node_id && fc.kind == kind)
                {
                    let ctrl = ui::Control::from_id(fc.control_id);
                    ctrl.set_position(x, y);
                    ctrl.set_size(w as u32, h as u32);
                    ctrl.set_color(accent);
                    ctrl.set_enabled(!bx.form_disabled);
                    fc.seen = true;
                    fc.doc_x = x;
                    fc.doc_y = y;
                    fc.doc_w = w;
                    fc.doc_h = h;
                } else {
                    // Parse percentage from form_value (encoded as 0..1000).
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
                    let id = slider.id();

                    self.form_controls.push(FormControl {
                        control_id: id,
                        node_id,
                        kind,
                        name: String::new(),
                        seen: true,
                        doc_x: x,
                        doc_y: y,
                        doc_w: w,
                        doc_h: h,
                    });
                }
            }

            // Progress, Meter — display-list painted only (read-only indicators).
            FormFieldKind::Progress | FormFieldKind::Meter => {}

            // Number — TextField with placeholder.
            FormFieldKind::Number => {
                let bg = self.default_control_bg(bx);
                let fg = self.default_control_fg(bx);
                if let Some(fc) = self
                    .form_controls
                    .iter_mut()
                    .find(|fc| fc.node_id == node_id && fc.kind == kind)
                {
                    let ctrl = ui::Control::from_id(fc.control_id);
                    ctrl.set_position(x, y);
                    ctrl.set_size(w as u32, h as u32);
                    ctrl.set_color(bg);
                    ctrl.set_text_color(fg);
                    ctrl.set_enabled(!bx.form_disabled);
                    fc.seen = true;
                    fc.doc_x = x;
                    fc.doc_y = y;
                    fc.doc_w = w;
                    fc.doc_h = h;
                } else {
                    let tf = ui::TextField::new();
                    tf.set_position(x, y);
                    tf.set_size(w as u32, h as u32);
                    tf.set_color(bg);
                    tf.set_text_color(fg);
                    if let Some(ref ph) = bx.form_placeholder {
                        tf.set_placeholder(ph);
                    }
                    if let Some(ref val) = bx.form_value {
                        tf.set_text(val);
                    }
                    tf.set_enabled(!bx.form_disabled);
                    if let Some(cb) = submit_cb {
                        tf.on_submit_raw(cb, submit_cb_ud);
                    }
                    parent.add(&tf);
                    let id = tf.id();
                    self.form_controls.push(FormControl {
                        control_id: id,
                        node_id,
                        kind,
                        name: String::new(),
                        seen: true,
                        doc_x: x,
                        doc_y: y,
                        doc_w: w,
                        doc_h: h,
                    });
                }
            }

            // Date, Time, DatetimeLocal, Month, Week — native DateTimePicker.
            FormFieldKind::Date
            | FormFieldKind::Time
            | FormFieldKind::DatetimeLocal
            | FormFieldKind::Month
            | FormFieldKind::Week => {
                if let Some(fc) = self
                    .form_controls
                    .iter_mut()
                    .find(|fc| fc.node_id == node_id && fc.kind == kind)
                {
                    let ctrl = ui::Control::from_id(fc.control_id);
                    ctrl.set_position(x, y);
                    ctrl.set_size(w as u32, h as u32);
                    ctrl.set_enabled(!bx.form_disabled);
                    fc.seen = true;
                    fc.doc_x = x;
                    fc.doc_y = y;
                    fc.doc_w = w;
                    fc.doc_h = h;
                } else {
                    let picker = match kind {
                        FormFieldKind::Time => {
                            let tp = ui::TimePicker::new();
                            // Parse "HH:MM" from value.
                            if let Some(ref val) = bx.form_value {
                                if let Some((h, m)) = parse_time_value(val) {
                                    tp.set_time(h, m);
                                }
                            }
                            tp.set_position(x, y);
                            tp.set_size(w as u32, h as u32);
                            tp.set_enabled(!bx.form_disabled);
                            parent.add(&tp);
                            tp.id()
                        }
                        FormFieldKind::Date | FormFieldKind::Month | FormFieldKind::Week => {
                            let dp = ui::DatePicker::new();
                            if let Some(ref val) = bx.form_value {
                                if let Some((y, m, d)) = parse_date_value(val) {
                                    dp.set_date(d, m, y);
                                }
                            }
                            dp.set_position(x, y);
                            dp.set_size(w as u32, h as u32);
                            dp.set_enabled(!bx.form_disabled);
                            parent.add(&dp);
                            dp.id()
                        }
                        _ => {
                            // DatetimeLocal
                            let dtp = ui::DateTimePicker::new();
                            if let Some(ref val) = bx.form_value {
                                // Parse "YYYY-MM-DDThh:mm"
                                if let (Some((y, mo, d)), Some((h, mi))) = (
                                    parse_date_value(&val.split('T').next().unwrap_or("")),
                                    val.split('T').nth(1).and_then(|t| parse_time_value(t)),
                                ) {
                                    dtp.set_datetime(d, mo, y, h, mi);
                                }
                            }
                            dtp.set_position(x, y);
                            dtp.set_size(w as u32, h as u32);
                            dtp.set_enabled(!bx.form_disabled);
                            parent.add(&dtp);
                            dtp.id()
                        }
                    };
                    self.form_controls.push(FormControl {
                        control_id: picker,
                        node_id,
                        kind,
                        name: String::new(),
                        seen: true,
                        doc_x: x,
                        doc_y: y,
                        doc_w: w,
                        doc_h: h,
                    });
                }
            }

            // Color input — native ColorWell with color picker dialog.
            FormFieldKind::Color => {
                if bx.appearance_none {
                    self.hit_regions.push(HitRegion {
                        x,
                        y,
                        w,
                        h,
                        kind: HitKind::ColorInput(node_id),
                    });
                    if let Some(fc) = self
                        .form_controls
                        .iter_mut()
                        .find(|fc| fc.node_id == node_id && fc.kind == kind)
                    {
                        if fc.control_id != 0 {
                            ui::Control::from_id(fc.control_id).remove();
                            fc.control_id = 0;
                        }
                        fc.seen = true;
                        fc.doc_x = x;
                        fc.doc_y = y;
                        fc.doc_w = w;
                        fc.doc_h = h;
                    } else {
                        self.form_controls.push(FormControl {
                            control_id: 0,
                            node_id,
                            kind,
                            name: String::new(),
                            seen: true,
                            doc_x: x,
                            doc_y: y,
                            doc_w: w,
                            doc_h: h,
                        });
                    }
                    return;
                }
                if let Some(fc) = self
                    .form_controls
                    .iter_mut()
                    .find(|fc| fc.node_id == node_id && fc.kind == kind)
                {
                    let ctrl = ui::Control::from_id(fc.control_id);
                    ctrl.set_position(x, y);
                    ctrl.set_size(w as u32, h as u32);
                    fc.seen = true;
                    fc.doc_x = x;
                    fc.doc_y = y;
                    fc.doc_w = w;
                    fc.doc_h = h;
                } else {
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
                    let id = cw.id();
                    self.form_controls.push(FormControl {
                        control_id: id,
                        node_id,
                        kind,
                        name: String::new(),
                        seen: true,
                        doc_x: x,
                        doc_y: y,
                        doc_w: w,
                        doc_h: h,
                    });
                }
            }

            // File input — button + filename display.
            FormFieldKind::File => {
                self.hit_regions.push(HitRegion {
                    x,
                    y,
                    w,
                    h,
                    kind: HitKind::FileInput(node_id),
                });
                // Store a hidden control for form data.
                if !self
                    .form_controls
                    .iter()
                    .any(|fc| fc.node_id == node_id && fc.kind == kind)
                {
                    self.form_controls.push(FormControl {
                        control_id: 0,
                        node_id,
                        kind,
                        name: String::new(),
                        seen: true,
                        doc_x: x,
                        doc_y: y,
                        doc_w: w,
                        doc_h: h,
                    });
                } else if let Some(fc) = self
                    .form_controls
                    .iter_mut()
                    .find(|fc| fc.node_id == node_id && fc.kind == kind)
                {
                    fc.seen = true;
                }
            }

            FormFieldKind::Reset => {
                // Already handled above in the hit_regions push.
            }
        }
    }

    /// Hit-test for a focusable form control at document coordinates.
    /// Returns the control_id of the first matching control.
    pub fn hit_test_form_at(&self, x: i32, doc_y: i32) -> Option<u32> {
        for fc in &self.form_controls {
            if fc.control_id == 0 {
                continue;
            }
            match fc.kind {
                FormFieldKind::TextInput
                | FormFieldKind::Password
                | FormFieldKind::Textarea
                | FormFieldKind::Number
                | FormFieldKind::Date
                | FormFieldKind::Time
                | FormFieldKind::DatetimeLocal
                | FormFieldKind::Month
                | FormFieldKind::Week
                | FormFieldKind::Color
                | FormFieldKind::Select
                | FormFieldKind::Range => {}
                _ => continue,
            }
            if x >= fc.doc_x
                && x < fc.doc_x + fc.doc_w
                && doc_y >= fc.doc_y
                && doc_y < fc.doc_y + fc.doc_h
            {
                return Some(fc.control_id);
            }
        }
        None
    }

}
