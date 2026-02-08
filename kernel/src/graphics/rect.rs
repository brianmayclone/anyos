/// Rectangle type for graphics operations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }

    pub fn intersects(&self, other: &Rect) -> bool {
        self.x < other.right()
            && self.right() > other.x
            && self.y < other.bottom()
            && self.bottom() > other.y
    }

    pub fn intersection(&self, other: &Rect) -> Option<Rect> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());

        if x < right && y < bottom {
            Some(Rect::new(x, y, (right - x) as u32, (bottom - y) as u32))
        } else {
            None
        }
    }

    pub fn union(&self, other: &Rect) -> Rect {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let right = self.right().max(other.right());
        let bottom = self.bottom().max(other.bottom());
        Rect::new(x, y, (right - x) as u32, (bottom - y) as u32)
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    pub fn offset(&self, dx: i32, dy: i32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.width, self.height)
    }

    pub fn inset(&self, amount: i32) -> Rect {
        Rect::new(
            self.x + amount,
            self.y + amount,
            (self.width as i32 - 2 * amount).max(0) as u32,
            (self.height as i32 - 2 * amount).max(0) as u32,
        )
    }

    /// Subtract `other` from `self`, returning exposed regions.
    /// Returns up to 4 non-overlapping rects (top, bottom, left, right strips).
    pub fn subtract(&self, other: &Rect) -> ([Rect; 4], usize) {
        let mut result = [Rect::new(0, 0, 0, 0); 4];
        let mut count = 0;

        if let Some(inter) = self.intersection(other) {
            // Top strip
            if inter.y > self.y {
                result[count] = Rect::new(self.x, self.y, self.width, (inter.y - self.y) as u32);
                count += 1;
            }
            // Bottom strip
            if inter.bottom() < self.bottom() {
                result[count] = Rect::new(self.x, inter.bottom(), self.width, (self.bottom() - inter.bottom()) as u32);
                count += 1;
            }
            // Left strip (middle height only)
            if inter.x > self.x {
                result[count] = Rect::new(self.x, inter.y, (inter.x - self.x) as u32, inter.height);
                count += 1;
            }
            // Right strip (middle height only)
            if inter.right() < self.right() {
                result[count] = Rect::new(inter.right(), inter.y, (self.right() - inter.right()) as u32, inter.height);
                count += 1;
            }
        } else {
            // No intersection — entire self is exposed
            result[0] = *self;
            count = 1;
        }

        (result, count)
    }
}
