use crate::rasterizer::math as scalar_math;

#[derive(Clone, Copy, Debug)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const ZERO: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 0.0 };

    pub fn new(x: f32, y: f32, z: f32) -> Self { Self { x, y, z } }

    pub fn dot(self, b: Vec3) -> f32 {
        self.x * b.x + self.y * b.y + self.z * b.z
    }

    pub fn length_sq(self) -> f32 { self.dot(self) }

    pub fn length(self) -> f32 { scalar_math::sqrt(self.length_sq()) }

    pub fn normalized(self) -> Vec3 {
        let len = self.length();
        if len < 1e-9 {
            Vec3::ZERO
        } else {
            self.scale(1.0 / len)
        }
    }

    pub fn scale(self, s: f32) -> Vec3 {
        Vec3::new(self.x * s, self.y * s, self.z * s)
    }

    pub fn add(self, b: Vec3) -> Vec3 {
        Vec3::new(self.x + b.x, self.y + b.y, self.z + b.z)
    }

    pub fn sub(self, b: Vec3) -> Vec3 {
        Vec3::new(self.x - b.x, self.y - b.y, self.z - b.z)
    }

    pub fn neg(self) -> Vec3 {
        Vec3::new(-self.x, -self.y, -self.z)
    }

    pub fn cross(self, b: Vec3) -> Vec3 {
        Vec3::new(
            self.y * b.z - self.z * b.y,
            self.z * b.x - self.x * b.z,
            self.x * b.y - self.y * b.x,
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Quat {
    pub w: f32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Quat {
    pub const IDENTITY: Quat = Quat { w: 1.0, x: 0.0, y: 0.0, z: 0.0 };

    pub fn mul(self, q: Quat) -> Quat {
        Quat {
            w: self.w * q.w - self.x * q.x - self.y * q.y - self.z * q.z,
            x: self.w * q.x + self.x * q.w + self.y * q.z - self.z * q.y,
            y: self.w * q.y - self.x * q.z + self.y * q.w + self.z * q.x,
            z: self.w * q.z + self.x * q.y - self.y * q.x + self.z * q.w,
        }
    }

    pub fn conjugate(self) -> Quat {
        Quat { w: self.w, x: -self.x, y: -self.y, z: -self.z }
    }

    pub fn normalize(self) -> Quat {
        let len = scalar_math::sqrt(self.w * self.w + self.x * self.x + self.y * self.y + self.z * self.z);
        if len < 1e-9 {
            Quat::IDENTITY
        } else {
            let inv = 1.0 / len;
            Quat { w: self.w * inv, x: self.x * inv, y: self.y * inv, z: self.z * inv }
        }
    }

    pub fn integrate(self, omega: Vec3, dt: f32) -> Quat {
        let half_dt = dt * 0.5;
        let dq = Quat {
            w: 0.0,
            x: omega.x * half_dt,
            y: omega.y * half_dt,
            z: omega.z * half_dt,
        };
        dq.mul(self).add(self).normalize()
    }

    pub fn add(self, rhs: Quat) -> Quat {
        Quat {
            w: self.w + rhs.w,
            x: self.x + rhs.x,
            y: self.y + rhs.y,
            z: self.z + rhs.z,
        }
    }

    pub fn rotate_vec(self, v: Vec3) -> Vec3 {
        let qv = Vec3::new(self.x, self.y, self.z);
        let uv = qv.cross(v);
        let uuv = qv.cross(uv);
        v.add(uv.scale(2.0 * self.w)).add(uuv.scale(2.0))
    }

    pub fn rotation_y(self) -> f32 {
        let siny = 2.0 * (self.w * self.y + self.x * self.z);
        let cosy = 1.0 - 2.0 * (self.y * self.y + self.z * self.z);
        atan2(siny, cosy)
    }
}

pub fn atan2(y: f32, x: f32) -> f32 {
    if x == 0.0 && y == 0.0 { return 0.0; }
    let ax = scalar_math::abs(x);
    let ay = scalar_math::abs(y);
    let mn = if ax < ay { ax } else { ay };
    let mx = if ax < ay { ay } else { ax };
    let a = mn / mx;
    let s = a * a;
    let r = ((-0.0464964749 * s + 0.15931422) * s - 0.327622764) * s * a + a;
    let r = if ay > ax { 1.5707963 - r } else { r };
    let r = if x < 0.0 { 3.1415927 - r } else { r };
    if y < 0.0 { -r } else { r }
}

pub fn midpoint(a: Vec3, b: Vec3) -> Vec3 {
    a.add(b).scale(0.5)
}

pub fn clamp_f32(v: f32, lo: f32, hi: f32) -> f32 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}
