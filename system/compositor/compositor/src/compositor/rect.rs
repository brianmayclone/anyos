//! Axis-aligned rectangle for damage tracking and hit testing.

/// An axis-aligned rectangle with integer coordinates.
#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Rect { x, y, width, height }
    }

    pub fn right(&self) -> i32 {
        self.x + self.width as i32
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.height as i32
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }

    /// Compute intersection of two rectangles. Returns None if no overlap.
    pub fn intersect(&self, other: &Rect) -> Option<Rect> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let r = self.right().min(other.right());
        let b = self.bottom().min(other.bottom());
        if r > x && b > y {
            Some(Rect::new(x, y, (r - x) as u32, (b - y) as u32))
        } else {
            None
        }
    }

    /// Compute bounding box union of two rectangles.
    pub fn union(&self, other: &Rect) -> Rect {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let r = self.right().max(other.right());
        let b = self.bottom().max(other.bottom());
        Rect::new(x, y, (r - x) as u32, (b - y) as u32)
    }

    /// Expand rect by `n` pixels on all sides.
    pub fn expand(&self, n: i32) -> Rect {
        Rect::new(
            self.x - n,
            self.y - n,
            (self.width as i32 + n * 2).max(0) as u32,
            (self.height as i32 + n * 2).max(0) as u32,
        )
    }

    /// Clip rect to screen bounds.
    pub fn clip_to_screen(&self, w: u32, h: u32) -> Rect {
        let x = self.x.max(0);
        let y = self.y.max(0);
        let r = self.right().min(w as i32);
        let b = self.bottom().min(h as i32);
        if r > x && b > y {
            Rect::new(x, y, (r - x) as u32, (b - y) as u32)
        } else {
            Rect::new(0, 0, 0, 0)
        }
    }
}
