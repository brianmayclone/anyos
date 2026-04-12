impl Renderer {
    pub fn hit_test_link_at(&self, x: i32, doc_y: i32) -> Option<&str> {
        for region in &self.hit_regions {
            if x >= region.x
                && x < region.x + region.w
                && doc_y >= region.y
                && doc_y < region.y + region.h
            {
                if let HitKind::Link(ref url) = region.kind {
                    return Some(url.as_str());
                }
            }
        }
        None
    }

    pub fn hit_test_submit_at(&self, x: i32, doc_y: i32) -> Option<usize> {
        for region in &self.hit_regions {
            if x >= region.x
                && x < region.x + region.w
                && doc_y >= region.y
                && doc_y < region.y + region.h
            {
                if let HitKind::Submit(node_id) = region.kind {
                    return Some(node_id);
                }
            }
        }
        None
    }

    pub fn hit_test_reset_at(&self, x: i32, doc_y: i32) -> Option<usize> {
        for region in &self.hit_regions {
            if x >= region.x
                && x < region.x + region.w
                && doc_y >= region.y
                && doc_y < region.y + region.h
            {
                if let HitKind::Reset(node_id) = region.kind {
                    return Some(node_id);
                }
            }
        }
        None
    }

    pub fn hit_test_checkbox_at(&self, x: i32, doc_y: i32) -> Option<usize> {
        for region in &self.hit_regions {
            if x >= region.x
                && x < region.x + region.w
                && doc_y >= region.y
                && doc_y < region.y + region.h
            {
                if let HitKind::Checkbox(node_id) = region.kind {
                    return Some(node_id);
                }
            }
        }
        None
    }

    pub fn hit_test_select_at(&self, x: i32, doc_y: i32) -> Option<usize> {
        for region in &self.hit_regions {
            if x >= region.x
                && x < region.x + region.w
                && doc_y >= region.y
                && doc_y < region.y + region.h
            {
                if let HitKind::Select(node_id) = region.kind {
                    return Some(node_id);
                }
            }
        }
        None
    }

    pub fn hit_test_radio_at(&self, x: i32, doc_y: i32) -> Option<usize> {
        for region in &self.hit_regions {
            if x >= region.x
                && x < region.x + region.w
                && doc_y >= region.y
                && doc_y < region.y + region.h
            {
                if let HitKind::Radio(node_id) = region.kind {
                    return Some(node_id);
                }
            }
        }
        None
    }

    pub fn hit_test_range_at(&self, x: i32, doc_y: i32) -> Option<usize> {
        for region in &self.hit_regions {
            if x >= region.x
                && x < region.x + region.w
                && doc_y >= region.y
                && doc_y < region.y + region.h
            {
                if let HitKind::Range(node_id) = region.kind {
                    return Some(node_id);
                }
            }
        }
        None
    }

    pub fn hit_test_file_input_at(&self, x: i32, doc_y: i32) -> Option<usize> {
        for region in &self.hit_regions {
            if x >= region.x
                && x < region.x + region.w
                && doc_y >= region.y
                && doc_y < region.y + region.h
            {
                if let HitKind::FileInput(node_id) = region.kind {
                    return Some(node_id);
                }
            }
        }
        None
    }

    pub fn hit_test_color_input_at(&self, x: i32, doc_y: i32) -> Option<usize> {
        for region in &self.hit_regions {
            if x >= region.x
                && x < region.x + region.w
                && doc_y >= region.y
                && doc_y < region.y + region.h
            {
                if let HitKind::ColorInput(node_id) = region.kind {
                    return Some(node_id);
                }
            }
        }
        None
    }

    // ─────────────────────────────────────────────────────────────────────
    // Full render (relayout path)
}
