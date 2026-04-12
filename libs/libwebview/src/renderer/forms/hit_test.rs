use super::*;

impl Renderer {
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
