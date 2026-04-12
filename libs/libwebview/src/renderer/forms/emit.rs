use super::*;

impl Renderer {
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
        match kind {
            FormFieldKind::TextInput | FormFieldKind::Password => {
                self.emit_text_input_control(kind, bx, x, y, parent, submit_cb, submit_cb_ud);
            }
            FormFieldKind::Textarea => self.emit_textarea_control(kind, bx, x, y, parent),
            FormFieldKind::Number => {
                self.emit_number_control(kind, bx, x, y, parent, submit_cb, submit_cb_ud);
            }
            FormFieldKind::Date
            | FormFieldKind::Time
            | FormFieldKind::DatetimeLocal
            | FormFieldKind::Month
            | FormFieldKind::Week => self.emit_date_like_control(kind, bx, x, y, parent),
            FormFieldKind::Checkbox => self.emit_checkbox_control(kind, bx, x, y, parent),
            FormFieldKind::Radio => self.emit_radio_control(kind, bx, x, y, parent),
            FormFieldKind::Select => self.emit_select_control(kind, bx, x, y, parent),
            FormFieldKind::Range => self.emit_range_control(kind, bx, x, y, parent),
            FormFieldKind::Color => self.emit_color_control(kind, bx, x, y, parent),
            FormFieldKind::Submit | FormFieldKind::ButtonEl => {
                self.push_hit_only_control(kind, bx, x, y, HitKind::Submit(bx.node_id.unwrap_or(0)));
            }
            FormFieldKind::Reset => {
                self.push_hit_only_control(kind, bx, x, y, HitKind::Reset(bx.node_id.unwrap_or(0)));
            }
            FormFieldKind::Hidden => self.track_hidden_control(kind, bx, x, y),
            FormFieldKind::File => self.track_file_input_control(kind, bx, x, y),
            FormFieldKind::Progress | FormFieldKind::Meter => {}
        }
    }
}
