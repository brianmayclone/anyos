//! Simple rigid-body physics engine for libgl.
//!
//! Supports:
//! - Sphere and infinite-plane colliders
//! - Gravity (configurable G vector)
//! - Mass-based acceleration (F = m * a)
//! - Elastic collision response with restitution
//! - Semi-implicit Euler integration
//!
//! Design: single global `PhysicsWorld` with up to `MAX_BODIES` rigid bodies.

use alloc::vec::Vec;
use crate::rasterizer::math;

/// Maximum number of bodies in the world.
const MAX_BODIES: usize = 64;

// ── Vec3 helper ─────────────────────────────────────────────────────────────

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

    pub fn length(self) -> f32 { math::sqrt(self.length_sq()) }

    pub fn normalized(self) -> Vec3 {
        let len = self.length();
        if len < 1e-9 { return Vec3::ZERO; }
        let inv = 1.0 / len;
        Vec3::new(self.x * inv, self.y * inv, self.z * inv)
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

    pub fn cross(self, b: Vec3) -> Vec3 {
        Vec3::new(
            self.y * b.z - self.z * b.y,
            self.z * b.x - self.x * b.z,
            self.x * b.y - self.y * b.x,
        )
    }
}

// ── Quaternion ──────────────────────────────────────────────────────────────

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

    pub fn normalize(self) -> Quat {
        let len = math::sqrt(self.w * self.w + self.x * self.x + self.y * self.y + self.z * self.z);
        if len < 1e-9 { return Quat::IDENTITY; }
        let inv = 1.0 / len;
        Quat { w: self.w * inv, x: self.x * inv, y: self.y * inv, z: self.z * inv }
    }

    /// Integrate angular velocity (omega) over dt using quaternion derivative.
    pub fn integrate(self, omega: Vec3, dt: f32) -> Quat {
        let half_dt = dt * 0.5;
        let dq = Quat {
            w: 0.0,
            x: omega.x * half_dt,
            y: omega.y * half_dt,
            z: omega.z * half_dt,
        };
        let delta = dq.mul(self);
        Quat {
            w: self.w + delta.w,
            x: self.x + delta.x,
            y: self.y + delta.y,
            z: self.z + delta.z,
        }.normalize()
    }

    /// Rotate a vector by this quaternion: q * v * q⁻¹.
    pub fn rotate_vec(self, v: Vec3) -> Vec3 {
        let qv = Vec3::new(self.x, self.y, self.z);
        let uv = qv.cross(v);
        let uuv = qv.cross(uv);
        v.add(uv.scale(2.0 * self.w)).add(uuv.scale(2.0))
    }

    /// Extract approximate Y-axis rotation angle from quaternion.
    pub fn rotation_y(self) -> f32 {
        let siny = 2.0 * (self.w * self.y + self.x * self.z);
        let cosy = 1.0 - 2.0 * (self.y * self.y + self.z * self.z);
        atan2(siny, cosy)
    }
}

/// atan2 approximation using polynomial atan on [0,1].
fn atan2(y: f32, x: f32) -> f32 {
    if x == 0.0 && y == 0.0 { return 0.0; }
    let ax = math::abs(x);
    let ay = math::abs(y);
    let mn = if ax < ay { ax } else { ay };
    let mx = if ax < ay { ay } else { ax };
    let a = mn / mx;
    let s = a * a;
    let r = ((-0.0464964749 * s + 0.15931422) * s - 0.327622764) * s * a + a;
    let r = if ay > ax { 1.5707963 - r } else { r };
    let r = if x < 0.0 { 3.1415927 - r } else { r };
    if y < 0.0 { -r } else { r }
}

// ── Collider shapes ─────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
pub enum Collider {
    /// Sphere with radius.
    Sphere { radius: f32 },
    /// Infinite plane defined by normal and distance from origin (n.dot(p) = d).
    Plane { normal: Vec3, d: f32 },
    /// Axis-aligned box (half-extents from center).
    Box { half_x: f32, half_y: f32, half_z: f32 },
}

// ── Rigid body ──────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct RigidBody {
    pub active: bool,
    pub position: Vec3,
    pub velocity: Vec3,
    /// Accumulated force this frame (reset after each step).
    pub force: Vec3,
    /// Mass in kg. 0.0 = infinite mass (static/immovable).
    pub mass: f32,
    /// Inverse mass (cached). 0.0 for static bodies.
    pub inv_mass: f32,
    /// Coefficient of restitution (bounciness, 0.0..1.0).
    pub restitution: f32,
    /// Collider shape.
    pub collider: Collider,
    /// 3D angular velocity (rad/s).
    pub angular_vel: Vec3,
    /// Orientation quaternion.
    pub orientation: Quat,
    /// Angular damping factor (0.0 = no damping, higher = more friction). Applied per second.
    pub angular_damping: f32,
    /// Whether this body is affected by gravity.
    pub use_gravity: bool,
}

impl RigidBody {
    fn new_default() -> Self {
        Self {
            active: false,
            position: Vec3::ZERO,
            velocity: Vec3::ZERO,
            force: Vec3::ZERO,
            mass: 1.0,
            inv_mass: 1.0,
            restitution: 0.5,
            collider: Collider::Sphere { radius: 0.5 },
            angular_vel: Vec3::ZERO,
            orientation: Quat::IDENTITY,
            angular_damping: 0.0,
            use_gravity: true,
        }
    }
}

// ── Physics World ───────────────────────────────────────────────────────────

pub struct PhysicsWorld {
    pub bodies: Vec<RigidBody>,
    /// Gravity acceleration vector (default: 0, -9.81, 0).
    pub gravity: Vec3,
}

impl PhysicsWorld {
    pub fn new() -> Self {
        Self {
            bodies: Vec::new(),
            gravity: Vec3::new(0.0, -9.81, 0.0),
        }
    }

    /// Add a sphere body. Returns body index.
    pub fn add_sphere(&mut self, mass: f32, radius: f32, x: f32, y: f32, z: f32) -> u32 {
        let inv_mass = if mass <= 0.0 { 0.0 } else { 1.0 / mass };
        let body = RigidBody {
            active: true,
            position: Vec3::new(x, y, z),
            velocity: Vec3::ZERO,
            force: Vec3::ZERO,
            mass,
            inv_mass,
            restitution: 0.5,
            collider: Collider::Sphere { radius },
            angular_vel: Vec3::ZERO,
            orientation: Quat::IDENTITY,
            angular_damping: 0.0,
            use_gravity: mass > 0.0,
        };
        let id = self.bodies.len() as u32;
        self.bodies.push(body);
        id
    }

    /// Add an infinite plane (static, infinite mass). Returns body index.
    pub fn add_plane(&mut self, nx: f32, ny: f32, nz: f32, d: f32) -> u32 {
        let normal = Vec3::new(nx, ny, nz).normalized();
        let body = RigidBody {
            active: true,
            position: normal.scale(d), // position on plane for reference
            velocity: Vec3::ZERO,
            force: Vec3::ZERO,
            mass: 0.0,
            inv_mass: 0.0,
            restitution: 0.5,
            collider: Collider::Plane { normal, d },
            angular_vel: Vec3::ZERO,
            orientation: Quat::IDENTITY,
            angular_damping: 0.0,
            use_gravity: false,
        };
        let id = self.bodies.len() as u32;
        self.bodies.push(body);
        id
    }

    /// Add an axis-aligned box body. Returns body index.
    pub fn add_box(&mut self, mass: f32, hx: f32, hy: f32, hz: f32, x: f32, y: f32, z: f32) -> u32 {
        let inv_mass = if mass <= 0.0 { 0.0 } else { 1.0 / mass };
        let body = RigidBody {
            active: true,
            position: Vec3::new(x, y, z),
            velocity: Vec3::ZERO,
            force: Vec3::ZERO,
            mass,
            inv_mass,
            restitution: 0.5,
            collider: Collider::Box { half_x: hx, half_y: hy, half_z: hz },
            angular_vel: Vec3::ZERO,
            orientation: Quat::IDENTITY,
            angular_damping: 0.0,
            use_gravity: mass > 0.0,
        };
        let id = self.bodies.len() as u32;
        self.bodies.push(body);
        id
    }

    /// Apply a force to a body (accumulated, applied next step).
    pub fn apply_force(&mut self, id: u32, fx: f32, fy: f32, fz: f32) {
        if let Some(b) = self.bodies.get_mut(id as usize) {
            b.force = b.force.add(Vec3::new(fx, fy, fz));
        }
    }

    /// Update the distance parameter `d` of a plane collider.
    pub fn set_plane_d(&mut self, id: u32, new_d: f32) {
        if let Some(b) = self.bodies.get_mut(id as usize) {
            if let Collider::Plane { ref normal, ref mut d } = b.collider {
                *d = new_d;
                b.position = normal.scale(new_d);
            }
        }
    }

    /// Apply an impulse (instant velocity change: dv = impulse / mass).
    pub fn apply_impulse(&mut self, id: u32, ix: f32, iy: f32, iz: f32) {
        if let Some(b) = self.bodies.get_mut(id as usize) {
            if b.inv_mass > 0.0 {
                b.velocity = b.velocity.add(Vec3::new(ix, iy, iz).scale(b.inv_mass));
            }
        }
    }

    /// Step the simulation by `dt` seconds.
    pub fn step(&mut self, dt: f32) {
        if dt <= 0.0 { return; }

        let n = self.bodies.len();

        // 1. Apply gravity and integrate forces → velocity (semi-implicit Euler)
        for b in self.bodies.iter_mut() {
            if !b.active || b.inv_mass == 0.0 { continue; }

            // Gravity: F_gravity = m * g, a = F/m = g
            if b.use_gravity {
                b.velocity = b.velocity.add(self.gravity.scale(dt));
            }

            // External forces: a = F / m
            if b.force.length_sq() > 0.0 {
                let accel = b.force.scale(b.inv_mass);
                b.velocity = b.velocity.add(accel.scale(dt));
            }

            // Clear accumulated force
            b.force = Vec3::ZERO;
        }

        // 2. Collision detection & response
        for i in 0..n {
            for j in (i + 1)..n {
                if !self.bodies[i].active || !self.bodies[j].active { continue; }
                // Skip if both are static
                if self.bodies[i].inv_mass == 0.0 && self.bodies[j].inv_mass == 0.0 { continue; }

                self.resolve_collision(i, j);
            }
        }

        // 3. Integrate velocity → position, angular velocity → orientation
        for b in self.bodies.iter_mut() {
            if !b.active || b.inv_mass == 0.0 { continue; }
            b.position = b.position.add(b.velocity.scale(dt));

            // Angular damping (exponential decay per second)
            if b.angular_damping > 0.0 {
                let factor = 1.0 - b.angular_damping * dt;
                let factor = if factor < 0.0 { 0.0 } else { factor };
                b.angular_vel = b.angular_vel.scale(factor);
            }

            // Integrate orientation from angular velocity
            let omega_len = b.angular_vel.length();
            if omega_len > 1e-6 {
                b.orientation = b.orientation.integrate(b.angular_vel, dt);
            }
        }
    }

    /// Detect and resolve collision between bodies i and j.
    fn resolve_collision(&mut self, i: usize, j: usize) {
        // Get collider info without borrowing self.bodies mutably
        let col_i = self.bodies[i].collider;
        let col_j = self.bodies[j].collider;
        let pos_i = self.bodies[i].position;
        let pos_j = self.bodies[j].position;

        match (col_i, col_j) {
            // Sphere vs Sphere
            (Collider::Sphere { radius: r1 }, Collider::Sphere { radius: r2 }) => {
                let diff = pos_i.sub(pos_j);
                let dist_sq = diff.length_sq();
                let min_dist = r1 + r2;
                if dist_sq < min_dist * min_dist && dist_sq > 1e-12 {
                    let dist = math::sqrt(dist_sq);
                    let normal = diff.scale(1.0 / dist);
                    let penetration = min_dist - dist;
                    self.resolve_contact(i, j, normal, penetration);
                }
            }

            // Sphere vs Plane
            (Collider::Sphere { radius }, Collider::Plane { normal, d }) => {
                let dist = pos_i.dot(normal) - d;
                if dist < radius {
                    let penetration = radius - dist;
                    self.resolve_contact(i, j, normal, penetration);
                }
            }
            // Plane vs Sphere (reversed)
            (Collider::Plane { normal, d }, Collider::Sphere { radius }) => {
                let dist = pos_j.dot(normal) - d;
                if dist < radius {
                    let penetration = radius - dist;
                    // Normal points from j to i, but plane normal points from plane
                    self.resolve_contact(j, i, normal, penetration);
                }
            }

            // Box vs Plane (OBB — accounts for box orientation)
            (Collider::Box { half_x, half_y, half_z }, Collider::Plane { normal, d }) => {
                let orient_i = self.bodies[i].orientation;
                let ax = orient_i.rotate_vec(Vec3::new(half_x, 0.0, 0.0));
                let ay = orient_i.rotate_vec(Vec3::new(0.0, half_y, 0.0));
                let az = orient_i.rotate_vec(Vec3::new(0.0, 0.0, half_z));
                let extent = math::abs(normal.dot(ax))
                           + math::abs(normal.dot(ay))
                           + math::abs(normal.dot(az));
                let dist = pos_i.dot(normal) - d;
                if dist < extent {
                    let penetration = extent - dist;
                    self.resolve_contact(i, j, normal, penetration);
                }
            }
            // Plane vs Box (reversed, OBB)
            (Collider::Plane { normal, d }, Collider::Box { half_x, half_y, half_z }) => {
                let orient_j = self.bodies[j].orientation;
                let ax = orient_j.rotate_vec(Vec3::new(half_x, 0.0, 0.0));
                let ay = orient_j.rotate_vec(Vec3::new(0.0, half_y, 0.0));
                let az = orient_j.rotate_vec(Vec3::new(0.0, 0.0, half_z));
                let extent = math::abs(normal.dot(ax))
                           + math::abs(normal.dot(ay))
                           + math::abs(normal.dot(az));
                let dist = pos_j.dot(normal) - d;
                if dist < extent {
                    let penetration = extent - dist;
                    self.resolve_contact(j, i, normal, penetration);
                }
            }

            // Sphere vs Box (simplified: treat box as AABB)
            (Collider::Sphere { radius }, Collider::Box { half_x, half_y, half_z }) => {
                if let Some((normal, pen)) = sphere_vs_aabb(pos_i, radius, pos_j, half_x, half_y, half_z) {
                    self.resolve_contact(i, j, normal, pen);
                }
            }
            // Box vs Sphere (reversed)
            (Collider::Box { half_x, half_y, half_z }, Collider::Sphere { radius }) => {
                if let Some((normal, pen)) = sphere_vs_aabb(pos_j, radius, pos_i, half_x, half_y, half_z) {
                    self.resolve_contact(j, i, normal, pen);
                }
            }

            // Box vs Box (simplified AABB)
            (Collider::Box { half_x: hx1, half_y: hy1, half_z: hz1 },
             Collider::Box { half_x: hx2, half_y: hy2, half_z: hz2 }) => {
                if let Some((normal, pen)) = aabb_vs_aabb(pos_i, hx1, hy1, hz1, pos_j, hx2, hy2, hz2) {
                    self.resolve_contact(i, j, normal, pen);
                }
            }

            // Plane vs Plane — skip
            (Collider::Plane { .. }, Collider::Plane { .. }) => {}
        }
    }

    /// Resolve a contact: separate overlapping bodies and apply impulse.
    /// `normal` points from body j toward body i.
    fn resolve_contact(&mut self, i: usize, j: usize, normal: Vec3, penetration: f32) {
        let inv_mass_i = self.bodies[i].inv_mass;
        let inv_mass_j = self.bodies[j].inv_mass;
        let total_inv = inv_mass_i + inv_mass_j;
        if total_inv <= 0.0 { return; }

        // Positional correction (prevent sinking)
        let correction = normal.scale(penetration * 0.8 / total_inv);
        if inv_mass_i > 0.0 {
            self.bodies[i].position = self.bodies[i].position.add(correction.scale(inv_mass_i));
        }
        if inv_mass_j > 0.0 {
            self.bodies[j].position = self.bodies[j].position.sub(correction.scale(inv_mass_j));
        }

        // Relative velocity along collision normal
        let rel_vel = self.bodies[i].velocity.sub(self.bodies[j].velocity);
        let vel_along_normal = rel_vel.dot(normal);

        // Only resolve if bodies are approaching
        if vel_along_normal > 0.0 { return; }

        // Restitution: use minimum of both
        let e = if self.bodies[i].restitution < self.bodies[j].restitution {
            self.bodies[i].restitution
        } else {
            self.bodies[j].restitution
        };

        // Impulse magnitude: j = -(1 + e) * v_rel.n / (1/m_i + 1/m_j)
        let impulse_mag = -(1.0 + e) * vel_along_normal / total_inv;
        let impulse = normal.scale(impulse_mag);

        if inv_mass_i > 0.0 {
            self.bodies[i].velocity = self.bodies[i].velocity.add(impulse.scale(inv_mass_i));
        }
        if inv_mass_j > 0.0 {
            self.bodies[j].velocity = self.bodies[j].velocity.sub(impulse.scale(inv_mass_j));
        }

        // 3D angular impulse from contact
        // Contact point offset from center of mass (approximate)
        // τ = r × J → Δω = I⁻¹ × τ
        let spin_factor = 0.3; // friction coefficient for spin transfer

        if inv_mass_i > 0.0 {
            let r_i = match self.bodies[i].collider {
                Collider::Sphere { radius } => radius,
                Collider::Box { half_x, half_y, half_z } => {
                    // Use average half-extent for inertia approximation
                    (half_x + half_y + half_z) / 3.0
                }
                _ => 0.5,
            };
            let (inertia_x, inertia_y, inertia_z) = match self.bodies[i].collider {
                Collider::Sphere { radius } => {
                    let i = 0.4 * self.bodies[i].mass * radius * radius;
                    (i, i, i)
                }
                Collider::Box { half_x, half_y, half_z } => {
                    let m = self.bodies[i].mass;
                    let f = m / 3.0; // m/12 * 4 = m/3 (using full extents = 2*half)
                    (f * (half_y * half_y + half_z * half_z),
                     f * (half_x * half_x + half_z * half_z),
                     f * (half_x * half_x + half_y * half_y))
                }
                _ => (1.0, 1.0, 1.0),
            };
            // Contact point is at surface in direction of -normal
            let contact_r = normal.scale(-r_i);
            let torque = contact_r.cross(impulse);
            let ang_impulse = Vec3::new(
                if inertia_x > 0.001 { torque.x * spin_factor / inertia_x } else { 0.0 },
                if inertia_y > 0.001 { torque.y * spin_factor / inertia_y } else { 0.0 },
                if inertia_z > 0.001 { torque.z * spin_factor / inertia_z } else { 0.0 },
            );
            self.bodies[i].angular_vel = self.bodies[i].angular_vel.add(ang_impulse);
        }

        if inv_mass_j > 0.0 {
            let r_j = match self.bodies[j].collider {
                Collider::Sphere { radius } => radius,
                Collider::Box { half_x, half_y, half_z } => {
                    (half_x + half_y + half_z) / 3.0
                }
                _ => 0.5,
            };
            let (inertia_x, inertia_y, inertia_z) = match self.bodies[j].collider {
                Collider::Sphere { radius } => {
                    let i = 0.4 * self.bodies[j].mass * radius * radius;
                    (i, i, i)
                }
                Collider::Box { half_x, half_y, half_z } => {
                    let m = self.bodies[j].mass;
                    let f = m / 3.0;
                    (f * (half_y * half_y + half_z * half_z),
                     f * (half_x * half_x + half_z * half_z),
                     f * (half_x * half_x + half_y * half_y))
                }
                _ => (1.0, 1.0, 1.0),
            };
            let contact_r = normal.scale(r_j);
            let torque = contact_r.cross(impulse);
            let ang_impulse = Vec3::new(
                if inertia_x > 0.001 { torque.x * spin_factor / inertia_x } else { 0.0 },
                if inertia_y > 0.001 { torque.y * spin_factor / inertia_y } else { 0.0 },
                if inertia_z > 0.001 { torque.z * spin_factor / inertia_z } else { 0.0 },
            );
            self.bodies[j].angular_vel = self.bodies[j].angular_vel.sub(ang_impulse);
        }
    }
}

// ── Sphere vs AABB helper ───────────────────────────────────────────────────

fn sphere_vs_aabb(
    sphere_pos: Vec3, radius: f32,
    box_pos: Vec3, hx: f32, hy: f32, hz: f32,
) -> Option<(Vec3, f32)> {
    // Find closest point on AABB to sphere center
    let dx = sphere_pos.x - box_pos.x;
    let dy = sphere_pos.y - box_pos.y;
    let dz = sphere_pos.z - box_pos.z;

    let cx = math::clamp(dx, -hx, hx);
    let cy = math::clamp(dy, -hy, hy);
    let cz = math::clamp(dz, -hz, hz);

    let diff = Vec3::new(dx - cx, dy - cy, dz - cz);
    let dist_sq = diff.length_sq();

    if dist_sq < radius * radius && dist_sq > 1e-12 {
        let dist = math::sqrt(dist_sq);
        let normal = diff.scale(1.0 / dist);
        let penetration = radius - dist;
        Some((normal, penetration))
    } else {
        None
    }
}

// ── AABB vs AABB helper ────────────────────────────────────────────────────

fn aabb_vs_aabb(
    pos_a: Vec3, hx_a: f32, hy_a: f32, hz_a: f32,
    pos_b: Vec3, hx_b: f32, hy_b: f32, hz_b: f32,
) -> Option<(Vec3, f32)> {
    let dx = pos_b.x - pos_a.x;
    let dy = pos_b.y - pos_a.y;
    let dz = pos_b.z - pos_a.z;

    let overlap_x = (hx_a + hx_b) - math::abs(dx);
    if overlap_x <= 0.0 { return None; }

    let overlap_y = (hy_a + hy_b) - math::abs(dy);
    if overlap_y <= 0.0 { return None; }

    let overlap_z = (hz_a + hz_b) - math::abs(dz);
    if overlap_z <= 0.0 { return None; }

    // Minimum penetration axis
    if overlap_x <= overlap_y && overlap_x <= overlap_z {
        let sign = if dx > 0.0 { 1.0 } else { -1.0 };
        Some((Vec3::new(sign, 0.0, 0.0), overlap_x))
    } else if overlap_y <= overlap_z {
        let sign = if dy > 0.0 { 1.0 } else { -1.0 };
        Some((Vec3::new(0.0, sign, 0.0), overlap_y))
    } else {
        let sign = if dz > 0.0 { 1.0 } else { -1.0 };
        Some((Vec3::new(0.0, 0.0, sign), overlap_z))
    }
}
