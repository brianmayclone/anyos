use super::*;

impl Renderer {
    fn emit_text_input_control(
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
        let bg = self.default_control_bg(bx);
        let fg = self.default_control_fg(bx);
        let w = bx.width;
        let h = bx.height;
        let font_size = form_control_font_size(bx);

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
            ctrl.set_font_size(font_size);
            if let Some(ref val) = bx.form_value {
                let mut buf = [0u8; 8];
                if ctrl.get_text(&mut buf) == 0 && !val.is_empty() {
                    ctrl.set_text(val);
                }
            }
            ctrl.set_enabled(!bx.form_disabled);
            fc.seen = true;
            fc.doc_x = x;
            fc.doc_y = y;
            fc.doc_w = w;
            fc.doc_h = h;
            return;
        }

        let id = if let Some(ref suggestions) = bx.form_datalist {
            let atf = ui::AutoCompleteTextField::new();
            atf.set_position(x, y);
            atf.set_size(w as u32, h as u32);
            atf.set_color(bg);
            atf.set_text_color(fg);
            atf.set_font_size(font_size);
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
        } else if kind == FormFieldKind::TextInput && bx.form_is_search {
            let sf = ui::SearchField::new();
            sf.set_position(x, y);
            sf.set_size(w as u32, h as u32);
            sf.set_color(bg);
            sf.set_text_color(fg);
            sf.set_font_size(font_size);
            sf.set_enabled(!bx.form_disabled);
            if let Some(ref ph) = bx.form_placeholder {
                sf.set_placeholder(ph);
            }
            if let Some(ref val) = bx.form_value {
                sf.set_text(val);
            }
            if let Some(cb) = submit_cb {
                sf.on_submit_raw(cb, submit_cb_ud);
            }
            parent.add(&sf);
            sf.id()
        } else {
            let tf = ui::TextField::new();
            if kind == FormFieldKind::Password {
                tf.set_password_mode(true);
            }
            tf.set_position(x, y);
            tf.set_size(w as u32, h as u32);
            tf.set_color(bg);
            tf.set_text_color(fg);
            tf.set_font_size(font_size);
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

    fn emit_textarea_control(
        &mut self,
        kind: FormFieldKind,
        bx: &LayoutBox,
        x: i32,
        y: i32,
        parent: &ui::View,
    ) {
        let node_id = bx.node_id.unwrap_or(0);
        let bg = self.default_control_bg(bx);
        let fg = self.default_control_fg(bx);
        let w = bx.width;
        let h = bx.height;
        let font_size = form_control_font_size(bx);

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
            ctrl.set_font_size(font_size);
            ctrl.set_enabled(!bx.form_disabled);
            fc.seen = true;
            fc.doc_x = x;
            fc.doc_y = y;
            fc.doc_w = w;
            fc.doc_h = h;
            return;
        }

        let ta = ui::TextArea::new();
        ta.set_position(x, y);
        ta.set_size(w as u32, h as u32);
        ta.set_color(bg);
        ta.set_text_color(fg);
        ta.set_font_size(font_size);
        if let Some(ref val) = bx.form_value {
            ta.set_text(val);
        }
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

    fn emit_number_control(
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
        let bg = self.default_control_bg(bx);
        let fg = self.default_control_fg(bx);
        let w = bx.width;
        let h = bx.height;
        let font_size = form_control_font_size(bx);

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
            ctrl.set_font_size(font_size);
            ctrl.set_enabled(!bx.form_disabled);
            fc.seen = true;
            fc.doc_x = x;
            fc.doc_y = y;
            fc.doc_w = w;
            fc.doc_h = h;
            return;
        }

        let tf = ui::TextField::new();
        tf.set_position(x, y);
        tf.set_size(w as u32, h as u32);
        tf.set_color(bg);
        tf.set_text_color(fg);
        tf.set_font_size(font_size);
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

    fn emit_date_like_control(
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
            return;
        }

        let control_id = match kind {
            FormFieldKind::Time => {
                let tp = ui::TimePicker::new();
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
                let dtp = ui::DateTimePicker::new();
                if let Some(ref val) = bx.form_value {
                    if let (Some((y, mo, d)), Some((h, mi))) = (
                        parse_date_value(val.split('T').next().unwrap_or("")),
                        val.split('T').nth(1).and_then(parse_time_value),
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
            control_id,
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

fn form_control_font_size(bx: &LayoutBox) -> u32 {
    bx.font_size.clamp(10, 32) as u32
}
