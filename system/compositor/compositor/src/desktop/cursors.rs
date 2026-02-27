//! Cursor shapes — software and hardware cursor definitions and management.

use crate::compositor::Rect;

use super::Desktop;

// ── SW Cursor ──────────────────────────────────────────────────────────────

pub(crate) const CURSOR_W: u32 = 12;
pub(crate) const CURSOR_H: u32 = 18;

#[rustfmt::skip]
pub(crate) const CURSOR_BITMAP: [u8; (CURSOR_W * CURSOR_H) as usize] = [
    2,0,0,0,0,0,0,0,0,0,0,0,
    2,2,0,0,0,0,0,0,0,0,0,0,
    2,1,2,0,0,0,0,0,0,0,0,0,
    2,1,1,2,0,0,0,0,0,0,0,0,
    2,1,1,1,2,0,0,0,0,0,0,0,
    2,1,1,1,1,2,0,0,0,0,0,0,
    2,1,1,1,1,1,2,0,0,0,0,0,
    2,1,1,1,1,1,1,2,0,0,0,0,
    2,1,1,1,1,1,1,1,2,0,0,0,
    2,1,1,1,1,1,1,1,1,2,0,0,
    2,1,1,1,1,1,1,1,1,1,2,0,
    2,1,1,1,1,1,2,2,2,2,2,0,
    2,1,1,1,1,2,0,0,0,0,0,0,
    2,1,1,2,1,1,2,0,0,0,0,0,
    2,1,2,0,2,1,1,2,0,0,0,0,
    2,2,0,0,2,1,1,2,0,0,0,0,
    2,0,0,0,0,2,1,2,0,0,0,0,
    0,0,0,0,0,2,2,2,0,0,0,0,
];

// ── HW Cursor Shapes (ARGB8888) ───────────────────────────────────────────

const W: u32 = 0xFFFFFFFF; // white (fill)
const B: u32 = 0xFF000000; // black (outline)
const T: u32 = 0x00000000; // transparent

/// Cursor shape identifiers.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CursorShape {
    Arrow,
    ResizeNS,
    ResizeEW,
    ResizeNWSE,
    ResizeNESW,
    Move,
}

// ── Arrow Cursor ───────────────────────────────────────────────────────────

const HW_ARROW_W: u32 = 12;
const HW_ARROW_H: u32 = 18;
const HW_ARROW_HOT_X: u32 = 0;
const HW_ARROW_HOT_Y: u32 = 0;

/// Compile-time ARGB arrow cursor pixels (converted from CURSOR_BITMAP).
/// Using a static array instead of a heap-allocated Vec ensures the pointer
/// is always valid — same pattern as all other cursor shapes.
#[rustfmt::skip]
static HW_ARROW: [u32; (HW_ARROW_W * HW_ARROW_H) as usize] = {
    const fn convert() -> [u32; (12 * 18) as usize] {
        let bitmap = CURSOR_BITMAP;
        let mut pixels = [T; (12 * 18) as usize];
        let mut i = 0;
        while i < pixels.len() {
            pixels[i] = match bitmap[i] {
                1 => W,
                2 => B,
                _ => T,
            };
            i += 1;
        }
        pixels
    }
    convert()
};

// ── N-S Resize Cursor ──────────────────────────────────────────────────────

const HW_RESIZE_NS_W: u32 = 11;
const HW_RESIZE_NS_H: u32 = 16;
const HW_RESIZE_NS_HOT_X: u32 = 5;
const HW_RESIZE_NS_HOT_Y: u32 = 8;

#[rustfmt::skip]
static HW_RESIZE_NS: [u32; (11 * 16) as usize] = [
    T,T,T,T,B,B,B,T,T,T,T,
    T,T,T,B,W,W,W,B,T,T,T,
    T,T,B,W,W,W,W,W,B,T,T,
    T,B,W,W,W,W,W,W,W,B,T,
    B,B,B,B,W,W,W,B,B,B,B,
    T,T,T,B,W,W,W,B,T,T,T,
    T,T,T,B,W,W,W,B,T,T,T,
    T,T,T,B,W,W,W,B,T,T,T,
    T,T,T,B,W,W,W,B,T,T,T,
    T,T,T,B,W,W,W,B,T,T,T,
    T,T,T,B,W,W,W,B,T,T,T,
    B,B,B,B,W,W,W,B,B,B,B,
    T,B,W,W,W,W,W,W,W,B,T,
    T,T,B,W,W,W,W,W,B,T,T,
    T,T,T,B,W,W,W,B,T,T,T,
    T,T,T,T,B,B,B,T,T,T,T,
];

// ── E-W Resize Cursor ──────────────────────────────────────────────────────

const HW_RESIZE_EW_W: u32 = 16;
const HW_RESIZE_EW_H: u32 = 11;
const HW_RESIZE_EW_HOT_X: u32 = 8;
const HW_RESIZE_EW_HOT_Y: u32 = 5;

#[rustfmt::skip]
static HW_RESIZE_EW: [u32; (16 * 11) as usize] = [
    T,T,T,T,T,T,T,T,T,T,T,T,T,T,T,T,
    T,T,T,B,T,T,T,T,T,T,T,T,B,T,T,T,
    T,T,B,W,B,T,T,T,T,T,T,B,W,B,T,T,
    T,B,W,W,B,T,T,T,T,T,T,B,W,W,B,T,
    B,W,W,W,B,B,B,B,B,B,B,B,W,W,W,B,
    B,W,W,W,W,W,W,W,W,W,W,W,W,W,W,B,
    B,W,W,W,B,B,B,B,B,B,B,B,W,W,W,B,
    T,B,W,W,B,T,T,T,T,T,T,B,W,W,B,T,
    T,T,B,W,B,T,T,T,T,T,T,B,W,B,T,T,
    T,T,T,B,T,T,T,T,T,T,T,T,B,T,T,T,
    T,T,T,T,T,T,T,T,T,T,T,T,T,T,T,T,
];

// ── NW-SE Resize Cursor ────────────────────────────────────────────────────

const HW_RESIZE_NWSE_W: u32 = 14;
const HW_RESIZE_NWSE_H: u32 = 14;
const HW_RESIZE_NWSE_HOT_X: u32 = 7;
const HW_RESIZE_NWSE_HOT_Y: u32 = 7;

#[rustfmt::skip]
static HW_RESIZE_NWSE: [u32; (14 * 14) as usize] = [
    B,B,B,B,B,B,B,T,T,T,T,T,T,T,
    B,W,W,W,W,B,T,T,T,T,T,T,T,T,
    B,W,W,W,B,T,T,T,T,T,T,T,T,T,
    B,W,W,W,W,B,T,T,T,T,T,T,T,T,
    B,W,B,W,W,W,B,T,T,T,T,T,T,T,
    B,B,T,B,W,W,W,B,T,T,T,T,T,T,
    B,T,T,T,B,W,W,W,B,T,T,T,T,T,
    T,T,T,T,T,B,W,W,W,B,T,T,T,B,
    T,T,T,T,T,T,B,W,W,W,B,T,B,B,
    T,T,T,T,T,T,T,B,W,W,W,B,W,B,
    T,T,T,T,T,T,T,T,B,W,W,W,W,B,
    T,T,T,T,T,T,T,T,T,B,W,W,W,B,
    T,T,T,T,T,T,T,T,B,W,W,W,W,B,
    T,T,T,T,T,T,T,B,B,B,B,B,B,B,
];

// ── NE-SW Resize Cursor ────────────────────────────────────────────────────

const HW_RESIZE_NESW_W: u32 = 14;
const HW_RESIZE_NESW_H: u32 = 14;
const HW_RESIZE_NESW_HOT_X: u32 = 7;
const HW_RESIZE_NESW_HOT_Y: u32 = 7;

#[rustfmt::skip]
static HW_RESIZE_NESW: [u32; (14 * 14) as usize] = [
    T,T,T,T,T,T,T,B,B,B,B,B,B,B,
    T,T,T,T,T,T,T,T,B,W,W,W,W,B,
    T,T,T,T,T,T,T,T,T,B,W,W,W,B,
    T,T,T,T,T,T,T,T,B,W,W,W,W,B,
    T,T,T,T,T,T,T,B,W,W,W,B,W,B,
    T,T,T,T,T,T,B,W,W,W,B,T,B,B,
    T,T,T,T,T,B,W,W,W,B,T,T,T,B,
    B,T,T,T,B,W,W,W,B,T,T,T,T,T,
    B,B,T,B,W,W,W,B,T,T,T,T,T,T,
    B,W,B,W,W,W,B,T,T,T,T,T,T,T,
    B,W,W,W,W,B,T,T,T,T,T,T,T,T,
    B,W,W,W,B,T,T,T,T,T,T,T,T,T,
    B,W,W,W,W,B,T,T,T,T,T,T,T,T,
    B,B,B,B,B,B,B,T,T,T,T,T,T,T,
];

// ── Move/Grab Cursor ───────────────────────────────────────────────────────

const HW_MOVE_W: u32 = 15;
const HW_MOVE_H: u32 = 15;
const HW_MOVE_HOT_X: u32 = 7;
const HW_MOVE_HOT_Y: u32 = 7;

#[rustfmt::skip]
static HW_MOVE: [u32; (15 * 15) as usize] = [
    T,T,T,T,T,T,B,B,B,T,T,T,T,T,T,
    T,T,T,T,T,B,W,W,W,B,T,T,T,T,T,
    T,T,T,T,B,W,W,W,W,W,B,T,T,T,T,
    T,T,T,T,T,B,W,W,W,B,T,T,T,T,T,
    T,T,B,T,T,B,W,W,W,B,T,T,B,T,T,
    T,B,W,B,B,B,W,W,W,B,B,B,W,B,T,
    B,W,W,W,W,W,W,W,W,W,W,W,W,W,B,
    B,W,W,W,W,W,W,W,W,W,W,W,W,W,B,
    B,W,W,W,W,W,W,W,W,W,W,W,W,W,B,
    T,B,W,B,B,B,W,W,W,B,B,B,W,B,T,
    T,T,B,T,T,B,W,W,W,B,T,T,B,T,T,
    T,T,T,T,T,B,W,W,W,B,T,T,T,T,T,
    T,T,T,T,B,W,W,W,W,W,B,T,T,T,T,
    T,T,T,T,T,B,W,W,W,B,T,T,T,T,T,
    T,T,T,T,T,T,B,B,B,T,T,T,T,T,T,
];

// ── Desktop Cursor Methods ─────────────────────────────────────────────────

impl Desktop {
    /// Draw the software cursor onto the back buffer AFTER compositing.
    /// This should be called only if HW cursor is not available.
    pub fn draw_sw_cursor(&mut self) {
        if self.compositor.has_hw_cursor() {
            return;
        }

        let mx = self.mouse_x;
        let my = self.mouse_y;
        let stride = self.compositor.width() as usize;
        let bb = &mut self.compositor.back_buffer;

        // Save pixels under cursor
        for cy in 0..CURSOR_H as i32 {
            for cx in 0..CURSOR_W as i32 {
                let px = mx + cx;
                let py = my + cy;
                let idx = (cy * CURSOR_W as i32 + cx) as usize;
                if px >= 0
                    && py >= 0
                    && (px as u32) < self.screen_width
                    && (py as u32) < self.screen_height
                {
                    let di = py as usize * stride + px as usize;
                    self.cursor_save[idx] = bb[di];
                } else {
                    self.cursor_save[idx] = 0;
                }
            }
        }

        // Draw cursor
        for cy in 0..CURSOR_H as i32 {
            for cx in 0..CURSOR_W as i32 {
                let idx = (cy * CURSOR_W as i32 + cx) as usize;
                let pixel = CURSOR_BITMAP[idx];
                if pixel == 0 {
                    continue;
                }
                let color = if pixel == 1 { 0xFFFFFFFF } else { 0xFF000000 };
                let px = mx + cx;
                let py = my + cy;
                if px >= 0
                    && py >= 0
                    && (px as u32) < self.screen_width
                    && (py as u32) < self.screen_height
                {
                    let di = py as usize * stride + px as usize;
                    bb[di] = color;
                }
            }
        }

        self.cursor_drawn = true;
        self.prev_cursor_x = mx;
        self.prev_cursor_y = my;
    }

    /// Damage the cursor region (call before compositing to include cursor in flush).
    pub fn damage_cursor(&mut self) {
        if !self.compositor.has_hw_cursor() {
            // Damage old cursor position
            self.compositor.add_damage(Rect::new(
                self.prev_cursor_x,
                self.prev_cursor_y,
                CURSOR_W + 1,
                CURSOR_H + 1,
            ));
            // Damage new cursor position
            self.compositor.add_damage(Rect::new(
                self.mouse_x,
                self.mouse_y,
                CURSOR_W + 1,
                CURSOR_H + 1,
            ));
        }
    }

    /// Initialize the HW cursor: define arrow shape, show, and position.
    pub fn init_hw_cursor(&mut self) {
        self.compositor.define_hw_cursor(
            HW_ARROW_W,
            HW_ARROW_H,
            HW_ARROW_HOT_X,
            HW_ARROW_HOT_Y,
            &HW_ARROW,
        );
        self.compositor.enable_hw_cursor();
        self.compositor
            .move_hw_cursor(self.mouse_x, self.mouse_y);
        self.compositor.flush_gpu();
        self.current_cursor = CursorShape::Arrow;
    }

    /// Change cursor shape if different from current. Defines new shape + flushes.
    pub fn set_cursor_shape(&mut self, shape: CursorShape) {
        if self.current_cursor == shape {
            return;
        }
        self.current_cursor = shape;
        if !self.compositor.has_hw_cursor() {
            return;
        }
        match shape {
            CursorShape::Arrow => {
                self.compositor.define_hw_cursor(
                    HW_ARROW_W,
                    HW_ARROW_H,
                    HW_ARROW_HOT_X,
                    HW_ARROW_HOT_Y,
                    &HW_ARROW,
                );
            }
            CursorShape::ResizeNS => {
                self.compositor.define_hw_cursor(
                    HW_RESIZE_NS_W,
                    HW_RESIZE_NS_H,
                    HW_RESIZE_NS_HOT_X,
                    HW_RESIZE_NS_HOT_Y,
                    &HW_RESIZE_NS,
                );
            }
            CursorShape::ResizeEW => {
                self.compositor.define_hw_cursor(
                    HW_RESIZE_EW_W,
                    HW_RESIZE_EW_H,
                    HW_RESIZE_EW_HOT_X,
                    HW_RESIZE_EW_HOT_Y,
                    &HW_RESIZE_EW,
                );
            }
            CursorShape::ResizeNWSE => {
                self.compositor.define_hw_cursor(
                    HW_RESIZE_NWSE_W,
                    HW_RESIZE_NWSE_H,
                    HW_RESIZE_NWSE_HOT_X,
                    HW_RESIZE_NWSE_HOT_Y,
                    &HW_RESIZE_NWSE,
                );
            }
            CursorShape::ResizeNESW => {
                self.compositor.define_hw_cursor(
                    HW_RESIZE_NESW_W,
                    HW_RESIZE_NESW_H,
                    HW_RESIZE_NESW_HOT_X,
                    HW_RESIZE_NESW_HOT_Y,
                    &HW_RESIZE_NESW,
                );
            }
            CursorShape::Move => {
                self.compositor.define_hw_cursor(
                    HW_MOVE_W,
                    HW_MOVE_H,
                    HW_MOVE_HOT_X,
                    HW_MOVE_HOT_Y,
                    &HW_MOVE,
                );
            }
        }
        // Re-assert cursor position after shape change to ensure visibility.
        // Without this, some GPU backends (VirtIO, VMware SVGA) may briefly
        // hide or misplace the cursor during the shape transition.
        self.compositor
            .move_hw_cursor(self.mouse_x, self.mouse_y);
        self.compositor.flush_gpu();
    }

    /// Determine the correct cursor shape from a HitTest result.
    pub(crate) fn cursor_for_hit(&self, hit: super::window::HitTest) -> CursorShape {
        match hit {
            super::window::HitTest::ResizeTop | super::window::HitTest::ResizeBottom => {
                CursorShape::ResizeNS
            }
            super::window::HitTest::ResizeLeft | super::window::HitTest::ResizeRight => {
                CursorShape::ResizeEW
            }
            super::window::HitTest::ResizeTopLeft
            | super::window::HitTest::ResizeBottomRight => CursorShape::ResizeNWSE,
            super::window::HitTest::ResizeTopRight
            | super::window::HitTest::ResizeBottomLeft => CursorShape::ResizeNESW,
            _ => CursorShape::Arrow,
        }
    }
}
