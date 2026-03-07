use crate::aabb::AABB;

pub struct Body {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub vx: f32,
    pub vy: f32,
    pub vz: f32,
    pub width: f32,
    pub height: f32,
    pub on_ground: bool,
    pub flying: bool,
}

impl Body {
    pub fn new(x: f32, y: f32, z: f32, width: f32, height: f32) -> Self {
        Self {
            x, y, z,
            vx: 0.0, vy: 0.0, vz: 0.0,
            width, height,
            on_ground: false,
            flying: false,
        }
    }

    pub fn aabb(&self) -> AABB {
        let half_w = self.width / 2.0;
        AABB::new(
            self.x - half_w,
            self.y,
            self.z - half_w,
            self.x + half_w,
            self.y + self.height,
            self.z + half_w,
        )
    }
}
