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

    /// Get the current canvas height in pixels (may change when docked).
    pub fn get_height(&self) -> u32 {
        (lib().canvas_get_height)(self.ctrl.id)
    }

    /// Enable interactive mode: mouse_move fires EVENT_CHANGE for drag-drawing.
    pub fn set_interactive(&self, enabled: bool) {
        (lib().canvas_set_interactive)(self.ctrl.id, enabled as u32);
    }

    /// Get the last mouse position and button state.
    pub fn get_mouse(&self) -> (i32, i32, u32) {
        let mut x = 0i32;
        let mut y = 0i32;
        let mut button = 0u32;
        (lib().canvas_get_mouse)(self.ctrl.id, &mut x, &mut y, &mut button);
        (x, y, button)
    }

    /// Draw a filled ellipse.
    pub fn fill_ellipse(&self, cx: i32, cy: i32, rx: i32, ry: i32, color: u32) {
        (lib().canvas_fill_ellipse)(self.ctrl.id, cx, cy, rx, ry, color);
    }

    /// Draw an ellipse outline.
    pub fn draw_ellipse(&self, cx: i32, cy: i32, rx: i32, ry: i32, color: u32) {
        (lib().canvas_draw_ellipse)(self.ctrl.id, cx, cy, rx, ry, color);
    }

    /// Flood fill starting at (x, y) with the given color.
    pub fn flood_fill(&self, x: i32, y: i32, color: u32) {
        (lib().canvas_flood_fill)(self.ctrl.id, x, y, color);
    }

    /// Draw a thick line (filled circles along Bresenham path).
    pub fn draw_thick_line(&self, x0: i32, y0: i32, x1: i32, y1: i32, color: u32, thickness: u32) {
        (lib().canvas_draw_thick_line)(self.ctrl.id, x0, y0, x1, y1, color, thickness);
    }

    /// Read a single pixel. Returns 0 if out of bounds.
    pub fn get_pixel(&self, x: i32, y: i32) -> u32 {
        (lib().canvas_get_pixel)(self.ctrl.id, x, y)
    }

    /// Copy pixel data from a source slice into the canvas.
    pub fn copy_pixels_from(&self, src: &[u32]) {
        (lib().canvas_copy_from)(self.ctrl.id, src.as_ptr(), src.len() as u32);
    }

    /// Copy canvas pixels into a destination buffer. Returns count copied.
    pub fn copy_pixels_to(&self, dst: &mut [u32]) -> usize {
        (lib().canvas_copy_to)(self.ctrl.id, dst.as_mut_ptr(), dst.len() as u32) as usize
    }

    /// Register callback for click events.
    pub fn on_click(&self, mut f: impl FnMut(&ClickEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| f(&ClickEvent { id }));
        (lib().on_click_fn)(self.ctrl.id, thunk, ud);
    }

    /// Register callback for mouse_down events (fires on button press).
    pub fn on_mouse_down(&self, mut f: impl FnMut(i32, i32, u32) + 'static) {
        let canvas_id = self.ctrl.id;
        let (thunk, ud) = events::register(move |_id, _| {
            let mut x = 0i32;
            let mut y = 0i32;
            let mut button = 0u32;
            (lib().canvas_get_mouse)(canvas_id, &mut x, &mut y, &mut button);
            f(x, y, button);
        });
        (lib().on_event_fn)(self.ctrl.id, crate::EVENT_MOUSE_DOWN, thunk, ud);
    }

    /// Register callback for mouse_up events.
    pub fn on_mouse_up(&self, mut f: impl FnMut(i32, i32, u32) + 'static) {
        let canvas_id = self.ctrl.id;
        let (thunk, ud) = events::register(move |_id, _| {
            let mut x = 0i32;
            let mut y = 0i32;
            let mut button = 0u32;
            (lib().canvas_get_mouse)(canvas_id, &mut x, &mut y, &mut button);
            f(x, y, button);
        });
        (lib().on_event_fn)(self.ctrl.id, crate::EVENT_MOUSE_UP, thunk, ud);
    }

    /// Register callback for mouse move events (fires on every cursor movement over the canvas).
    pub fn on_mouse_move(&self, mut f: impl FnMut(i32, i32) + 'static) {
        let canvas_id = self.ctrl.id;
        let (thunk, ud) = events::register(move |_id, _| {
            let mut x = 0i32;
            let mut y = 0i32;
            let mut button = 0u32;
            (lib().canvas_get_mouse)(canvas_id, &mut x, &mut y, &mut button);
            f(x, y);
        });
        (lib().on_event_fn)(self.ctrl.id, crate::EVENT_MOUSE_MOVE, thunk, ud);
    }

    /// Register callback for mouse drag (requires `set_interactive(true)`).
    pub fn on_draw(&self, mut f: impl FnMut(i32, i32, u32) + 'static) {
        let canvas_id = self.ctrl.id;
        let (thunk, ud) = events::register(move |_id, _| {
            let mut x = 0i32;
            let mut y = 0i32;
            let mut button = 0u32;
            (lib().canvas_get_mouse)(canvas_id, &mut x, &mut y, &mut button);
            f(x, y, button);
        });
        (lib().on_change_fn)(self.ctrl.id, thunk, ud);
    }
}
