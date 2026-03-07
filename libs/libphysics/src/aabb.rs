#[derive(Clone, Copy, Debug)]
pub struct AABB {
    pub min_x: f32,
    pub min_y: f32,
    pub min_z: f32,
    pub max_x: f32,
    pub max_y: f32,
    pub max_z: f32,
}

impl AABB {
    pub fn new(min_x: f32, min_y: f32, min_z: f32, max_x: f32, max_y: f32, max_z: f32) -> Self {
        Self { min_x, min_y, min_z, max_x, max_y, max_z }
    }

    pub fn from_center(cx: f32, cy: f32, cz: f32, half_w: f32, half_h: f32, half_d: f32) -> Self {
        Self {
            min_x: cx - half_w,
            min_y: cy - half_h,
            min_z: cz - half_d,
            max_x: cx + half_w,
            max_y: cy + half_h,
            max_z: cz + half_d,
        }
    }

    pub fn intersects(&self, other: &AABB) -> bool {
        self.max_x > other.min_x && self.min_x < other.max_x &&
        self.max_y > other.min_y && self.min_y < other.max_y &&
        self.max_z > other.min_z && self.min_z < other.max_z
    }

    pub fn offset(&self, dx: f32, dy: f32, dz: f32) -> AABB {
        AABB {
            min_x: self.min_x + dx,
            min_y: self.min_y + dy,
            min_z: self.min_z + dz,
            max_x: self.max_x + dx,
            max_y: self.max_y + dy,
            max_z: self.max_z + dz,
        }
    }

    /// Clip X displacement of a moving AABB against this static AABB.
    /// Returns clamped dx.
    pub fn clip_x(&self, moving: &AABB, dx: f32) -> f32 {
        // Must overlap on Y and Z axes
        if moving.max_y <= self.min_y || moving.min_y >= self.max_y {
            return dx;
        }
        if moving.max_z <= self.min_z || moving.min_z >= self.max_z {
            return dx;
        }

        if dx > 0.0 && moving.max_x <= self.min_x {
            let clamp = self.min_x - moving.max_x;
            if clamp < dx { return clamp; }
        }
        if dx < 0.0 && moving.min_x >= self.max_x {
            let clamp = self.max_x - moving.min_x;
            if clamp > dx { return clamp; }
        }
        dx
    }

    /// Clip Y displacement of a moving AABB against this static AABB.
    /// Returns clamped dy.
    pub fn clip_y(&self, moving: &AABB, dy: f32) -> f32 {
        // Must overlap on X and Z axes
        if moving.max_x <= self.min_x || moving.min_x >= self.max_x {
            return dy;
        }
        if moving.max_z <= self.min_z || moving.min_z >= self.max_z {
            return dy;
        }

        if dy > 0.0 && moving.max_y <= self.min_y {
            let clamp = self.min_y - moving.max_y;
            if clamp < dy { return clamp; }
        }
        if dy < 0.0 && moving.min_y >= self.max_y {
            let clamp = self.max_y - moving.min_y;
            if clamp > dy { return clamp; }
        }
        dy
    }

    /// Clip Z displacement of a moving AABB against this static AABB.
    /// Returns clamped dz.
    pub fn clip_z(&self, moving: &AABB, dz: f32) -> f32 {
        // Must overlap on X and Y axes
        if moving.max_x <= self.min_x || moving.min_x >= self.max_x {
            return dz;
        }
        if moving.max_y <= self.min_y || moving.min_y >= self.max_y {
            return dz;
        }

        if dz > 0.0 && moving.max_z <= self.min_z {
            let clamp = self.min_z - moving.max_z;
            if clamp < dz { return clamp; }
        }
        if dz < 0.0 && moving.min_z >= self.max_z {
            let clamp = self.max_z - moving.min_z;
            if clamp > dz { return clamp; }
        }
        dz
    }
}
