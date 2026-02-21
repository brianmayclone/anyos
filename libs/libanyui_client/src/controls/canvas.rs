use crate::{Control, Widget, lib, events, KIND_CANVAS};
use crate::events::ClickEvent;

leaf_control!(Canvas, KIND_CANVAS);

impl Canvas {
    pub fn new(w: u32, h: u32) -> Self {
        let id = (lib().create_control)(KIND_CANVAS, core::ptr::null(), 0);
        (lib().set_size)(id, w, h);
        Self { ctrl: Control { id } }
    }

    pub fn set_pixel(&self, x: i32, y: i32, color: u32) {
        (lib().canvas_set_pixel)(self.ctrl.id, x, y, color);
    }

    pub fn clear(&self, color: u32) {
        (lib().canvas_clear)(self.ctrl.id, color);
    }

    pub fn fill_rect(&self, x: i32, y: i32, w: u32, h: u32, color: u32) {
        (lib().canvas_fill_rect)(self.ctrl.id, x, y, w, h, color);
    }

    pub fn draw_line(&self, x0: i32, y0: i32, x1: i32, y1: i32, color: u32) {
        (lib().canvas_draw_line)(self.ctrl.id, x0, y0, x1, y1, color);
    }

    pub fn draw_rect(&self, x: i32, y: i32, w: u32, h: u32, color: u32, thickness: u32) {
        (lib().canvas_draw_rect)(self.ctrl.id, x, y, w, h, color, thickness);
    }

    pub fn draw_circle(&self, cx: i32, cy: i32, radius: i32, color: u32) {
        (lib().canvas_draw_circle)(self.ctrl.id, cx, cy, radius, color);
    }

    pub fn fill_circle(&self, cx: i32, cy: i32, radius: i32, color: u32) {
        (lib().canvas_fill_circle)(self.ctrl.id, cx, cy, radius, color);
    }

    pub fn get_buffer(&self) -> *mut u32 {
        (lib().canvas_get_buffer)(self.ctrl.id)
    }

    pub fn get_stride(&self) -> u32 {
        (lib().canvas_get_stride)(self.ctrl.id)
    }

    pub fn on_click(&self, mut f: impl FnMut(&ClickEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| f(&ClickEvent { id }));
        (lib().on_click_fn)(self.ctrl.id, thunk, ud);
    }
}
