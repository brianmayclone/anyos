use super::*;

impl Renderer {
    fn push_hit_only_control(
        &mut self,
        kind: FormFieldKind,
        bx: &LayoutBox,
        x: i32,
        y: i32,
        hit_kind: HitKind,
    ) {
        let node_id = bx.node_id.unwrap_or(0);
        self.hit_regions.push(HitRegion {
            x,
            y,
            w: bx.width,
            h: bx.height,
            kind: hit_kind,
        });
        if !matches!(kind, FormFieldKind::Submit | FormFieldKind::Reset | FormFieldKind::ButtonEl) {
            return;
        }
        if let Some(fc) = self.find_control_mut(node_id, kind) {
            if fc.control_id != 0 {
                ui::Control::from_id(fc.control_id).remove();
                fc.control_id = 0;
            }
            Self::update_control_bounds(fc, x, y, bx.width, bx.height);
        }
    }

    fn track_hidden_control(&mut self, kind: FormFieldKind, bx: &LayoutBox, x: i32, y: i32) {
        let node_id = bx.node_id.unwrap_or(0);
        if let Some(fc) = self.find_control_mut(node_id, kind) {
            Self::update_control_bounds(fc, x, y, bx.width, bx.height);
        } else {
            self.push_form_control(0, node_id, kind, x, y, bx.width, bx.height);
        }
    }

    fn track_file_input_control(&mut self, kind: FormFieldKind, bx: &LayoutBox, x: i32, y: i32) {
        let node_id = bx.node_id.unwrap_or(0);
        self.hit_regions.push(HitRegion {
            x,
            y,
            w: bx.width,
            h: bx.height,
            kind: HitKind::FileInput(node_id),
        });
        if let Some(fc) = self.find_control_mut(node_id, kind) {
            Self::update_control_bounds(fc, x, y, bx.width, bx.height);
        } else {
            self.push_form_control(0, node_id, kind, x, y, bx.width, bx.height);
        }
    }

    fn track_canvas_only_control(
        &mut self,
        kind: FormFieldKind,
        node_id: usize,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        hit_kind: HitKind,
    ) {
        self.hit_regions.push(HitRegion {
            x,
            y,
            w,
            h,
            kind: hit_kind,
        });

        if let Some(fc) = self.find_control_mut(node_id, kind) {
            if fc.control_id != 0 {
                ui::Control::from_id(fc.control_id).remove();
                fc.control_id = 0;
            }
            Self::update_control_bounds(fc, x, y, w, h);
        } else {
            self.push_form_control(0, node_id, kind, x, y, w, h);
        }
    }

    fn find_control_mut(&mut self, node_id: usize, kind: FormFieldKind) -> Option<&mut FormControl> {
        self.form_controls
            .iter_mut()
            .find(|fc| fc.node_id == node_id && fc.kind == kind)
    }

    fn update_control_bounds(fc: &mut FormControl, x: i32, y: i32, w: i32, h: i32) {
        fc.seen = true;
        fc.doc_x = x;
        fc.doc_y = y;
        fc.doc_w = w;
        fc.doc_h = h;
    }

    fn push_form_control(
        &mut self,
        control_id: u32,
        node_id: usize,
        kind: FormFieldKind,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    ) {
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
