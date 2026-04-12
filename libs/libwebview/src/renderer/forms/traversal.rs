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

        self.maybe_push_text_link_hit_region(bx, abs_x, abs_y);

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
            self.maybe_push_text_link_hit_region(bx, abs_x, abs_y);

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

    fn maybe_push_text_link_hit_region(&mut self, bx: &LayoutBox, abs_x: i32, abs_y: i32) {
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
    }
}
