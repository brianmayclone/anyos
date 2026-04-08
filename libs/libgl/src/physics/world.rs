use alloc::vec;
use alloc::vec::Vec;

use crate::rasterizer::math;

use super::body::{Collider, RigidBody};
use super::contact::Contact;
use super::math::{clamp_f32, Vec3};
use super::narrow::append_contacts;

const MAX_BODIES: usize = 64;
const MAX_SUBSTEPS: usize = 4;
const MAX_SUBSTEP_DT: f32 = 1.0 / 120.0;
const SOLVER_ITERATIONS: usize = 8;
const PENETRATION_SLOP: f32 = 0.002;
const POSITION_BAUMGARTE: f32 = 0.12;
const CONTACT_BAUMGARTE: f32 = 0.20;
const RESTITUTION_THRESHOLD: f32 = 1.0;
const RESTING_TANGENT_SPEED: f32 = 0.08;
const SLEEP_DELAY: f32 = 0.35;
const SLEEP_LINEAR_SPEED_SQ: f32 = 0.04 * 0.04;
const SLEEP_ANGULAR_SPEED_SQ: f32 = 0.10 * 0.10;

pub struct PhysicsWorld {
    pub bodies: Vec<RigidBody>,
    pub gravity: Vec3,
}

impl PhysicsWorld {
    pub fn new() -> Self {
        Self {
            bodies: Vec::new(),
            gravity: Vec3::new(0.0, -9.81, 0.0),
        }
    }

    pub fn add_sphere(&mut self, mass: f32, radius: f32, x: f32, y: f32, z: f32) -> u32 {
        if self.bodies.len() >= MAX_BODIES {
            return self.bodies.len().saturating_sub(1) as u32;
        }
        let id = self.bodies.len() as u32;
        self.bodies.push(RigidBody::sphere(mass, radius, Vec3::new(x, y, z)));
        id
    }

    pub fn add_plane(&mut self, nx: f32, ny: f32, nz: f32, d: f32) -> u32 {
        if self.bodies.len() >= MAX_BODIES {
            return self.bodies.len().saturating_sub(1) as u32;
        }
        let normal = Vec3::new(nx, ny, nz).normalized();
        let id = self.bodies.len() as u32;
        self.bodies.push(RigidBody::plane(normal, d));
        id
    }

    pub fn add_box(&mut self, mass: f32, hx: f32, hy: f32, hz: f32, x: f32, y: f32, z: f32) -> u32 {
        if self.bodies.len() >= MAX_BODIES {
            return self.bodies.len().saturating_sub(1) as u32;
        }
        let id = self.bodies.len() as u32;
        self.bodies.push(RigidBody::box_body(mass, hx, hy, hz, Vec3::new(x, y, z)));
        id
    }

    pub fn apply_force(&mut self, id: u32, fx: f32, fy: f32, fz: f32) {
        if let Some(b) = self.bodies.get_mut(id as usize) {
            b.force = b.force.add(Vec3::new(fx, fy, fz));
            if b.dynamic() {
                b.wake();
            }
        }
    }

    pub fn set_plane_d(&mut self, id: u32, new_d: f32) {
        if let Some(b) = self.bodies.get_mut(id as usize) {
            if let Collider::Plane { ref normal, ref mut d } = b.collider {
                *d = new_d;
                b.position = normal.scale(new_d);
            }
        }
    }

    pub fn apply_impulse(&mut self, id: u32, ix: f32, iy: f32, iz: f32) {
        if let Some(b) = self.bodies.get_mut(id as usize) {
            if b.inv_mass > 0.0 {
                b.velocity = b.velocity.add(Vec3::new(ix, iy, iz).scale(b.inv_mass));
                b.wake();
            }
        }
    }

    pub fn step(&mut self, dt: f32) {
        if dt <= 0.0 {
            return;
        }

        let mut steps = math::ceil(dt / MAX_SUBSTEP_DT) as usize;
        if steps == 0 {
            steps = 1;
        } else if steps > MAX_SUBSTEPS {
            steps = MAX_SUBSTEPS;
        }
        let sub_dt = dt / steps as f32;
        for _ in 0..steps {
            self.substep(sub_dt);
        }
    }

    fn substep(&mut self, dt: f32) {
        let n = self.bodies.len();
        let mut touching_counts = vec![0u8; n];

        self.integrate_forces(dt);
        self.integrate_motion(dt);

        let contacts = self.collect_contacts();
        for c in &contacts {
            touching_counts[c.i] = touching_counts[c.i].saturating_add(1);
            touching_counts[c.j] = touching_counts[c.j].saturating_add(1);
            if self.bodies[c.i].dynamic() {
                self.bodies[c.i].wake();
            }
            if self.bodies[c.j].dynamic() {
                self.bodies[c.j].wake();
            }
        }

        self.apply_support_gravity_torque(dt, &contacts);

        for _ in 0..SOLVER_ITERATIONS {
            for contact in contacts.iter().copied() {
                self.solve_contact(contact, dt);
            }
        }

        self.update_sleep_states(dt, &touching_counts);
    }

    fn integrate_forces(&mut self, dt: f32) {
        for body in self.bodies.iter_mut() {
            if !body.dynamic() {
                body.force = Vec3::ZERO;
                continue;
            }

            if body.sleeping {
                if body.force.length_sq() > 1e-8 {
                    body.wake();
                } else {
                    body.force = Vec3::ZERO;
                    continue;
                }
            }

            if body.use_gravity {
                body.velocity = body.velocity.add(self.gravity.scale(dt));
            }

            if body.force.length_sq() > 0.0 {
                body.velocity = body.velocity.add(body.force.scale(body.inv_mass * dt));
            }

            body.force = Vec3::ZERO;

            if body.linear_damping > 0.0 {
                let factor = 1.0 / (1.0 + body.linear_damping * dt);
                body.velocity = body.velocity.scale(factor);
            }
            if body.angular_damping > 0.0 {
                let factor = 1.0 / (1.0 + body.angular_damping * dt);
                body.angular_vel = body.angular_vel.scale(factor);
            }
        }
    }

    fn integrate_motion(&mut self, dt: f32) {
        for body in self.bodies.iter_mut() {
            if !body.dynamic() || body.sleeping {
                continue;
            }
            body.position = body.position.add(body.velocity.scale(dt));
            if body.angular_vel.length_sq() > 1e-8 {
                body.orientation = body.orientation.integrate(body.angular_vel, dt);
            }
        }
    }

    fn collect_contacts(&self) -> Vec<Contact> {
        let n = self.bodies.len();
        let mut contacts = Vec::with_capacity(n.saturating_mul(4));

        for i in 0..n {
            if !self.bodies[i].active {
                continue;
            }
            for j in (i + 1)..n {
                if !self.bodies[j].active {
                    continue;
                }
                if self.bodies[i].inv_mass == 0.0 && self.bodies[j].inv_mass == 0.0 {
                    continue;
                }
                append_contacts(&self.bodies, i, j, &mut contacts);
            }
        }

        contacts
    }

    fn solve_contact(&mut self, contact: Contact, dt: f32) {
        let i = contact.i;
        let j = contact.j;

        let inv_mass_i = self.bodies[i].inv_mass;
        let inv_mass_j = self.bodies[j].inv_mass;
        let total_inv = inv_mass_i + inv_mass_j;
        if total_inv <= 0.0 {
            return;
        }

        let normal = contact.normal;
        let r_i = contact.point.sub(self.bodies[i].position);
        let r_j = contact.point.sub(self.bodies[j].position);

        let vel_i = self.point_velocity(i, r_i);
        let vel_j = self.point_velocity(j, r_j);
        let rel_vel = vel_i.sub(vel_j);
        let vel_along_normal = rel_vel.dot(normal);

        let restitution = if math::abs(vel_along_normal) < RESTITUTION_THRESHOLD {
            0.0
        } else {
            contact.restitution
        };
        let bias = if contact.penetration > PENETRATION_SLOP && dt > 1e-6 {
            CONTACT_BAUMGARTE * (contact.penetration - PENETRATION_SLOP) / dt
        } else {
            0.0
        };

        let normal_mass = self.effective_mass(i, j, r_i, r_j, normal);
        if normal_mass <= 1e-8 {
            return;
        }

        let mut impulse_mag = (bias - (1.0 + restitution) * vel_along_normal) / normal_mass;
        if impulse_mag < 0.0 {
            impulse_mag = 0.0;
        }
        let normal_impulse = normal.scale(impulse_mag);
        self.apply_world_impulse(i, normal_impulse, r_i);
        self.apply_world_impulse(j, normal_impulse.neg(), r_j);

        let vel_i_after = self.point_velocity(i, r_i);
        let vel_j_after = self.point_velocity(j, r_j);
        let rel_after = vel_i_after.sub(vel_j_after);
        let tangent_vec = rel_after.sub(normal.scale(rel_after.dot(normal)));
        let tangent_speed_sq = tangent_vec.length_sq();
        if tangent_speed_sq > 1e-10 {
            let tangent_speed = math::sqrt(tangent_speed_sq);
            let tangent = tangent_vec.scale(1.0 / tangent_speed);
            let tangent_mass = self.effective_mass(i, j, r_i, r_j, tangent);
            if tangent_mass > 1e-8 {
                let desired_stop = if tangent_speed < RESTING_TANGENT_SPEED {
                    tangent_speed
                } else {
                    rel_after.dot(tangent)
                };
                let mut tangent_impulse_mag = -desired_stop / tangent_mass;
                let max_friction = contact.friction * impulse_mag;
                tangent_impulse_mag = clamp_f32(tangent_impulse_mag, -max_friction, max_friction);
                let tangent_impulse = tangent.scale(tangent_impulse_mag);
                self.apply_world_impulse(i, tangent_impulse, r_i);
                self.apply_world_impulse(j, tangent_impulse.neg(), r_j);
            }
        }

        self.apply_rolling_friction(i, j, normal, contact.rolling_friction, dt);
        self.apply_position_correction(i, j, normal, contact.penetration);
        self.clamp_resting_motion(i, j, normal);
    }

    fn apply_position_correction(&mut self, i: usize, j: usize, normal: Vec3, penetration: f32) {
        let inv_mass_i = self.bodies[i].inv_mass;
        let inv_mass_j = self.bodies[j].inv_mass;
        let total_inv = inv_mass_i + inv_mass_j;
        if total_inv <= 0.0 {
            return;
        }
        let penetration = penetration - PENETRATION_SLOP;
        if penetration <= 0.0 {
            return;
        }
        let correction = normal.scale((penetration * POSITION_BAUMGARTE) / total_inv);
        if inv_mass_i > 0.0 {
            self.bodies[i].position = self.bodies[i].position.add(correction.scale(inv_mass_i));
        }
        if inv_mass_j > 0.0 {
            self.bodies[j].position = self.bodies[j].position.sub(correction.scale(inv_mass_j));
        }
    }

    fn clamp_resting_motion(&mut self, i: usize, j: usize, normal: Vec3) {
        if self.bodies[j].inv_mass == 0.0 && self.bodies[i].inv_mass > 0.0 {
            let vn = self.bodies[i].velocity.dot(normal);
            if vn < 0.0 && vn > -0.08 {
                self.bodies[i].velocity = self.bodies[i].velocity.sub(normal.scale(vn));
            }
        }
        if self.bodies[i].inv_mass == 0.0 && self.bodies[j].inv_mass > 0.0 {
            let axis = normal.neg();
            let vn = self.bodies[j].velocity.dot(axis);
            if vn < 0.0 && vn > -0.08 {
                self.bodies[j].velocity = self.bodies[j].velocity.sub(axis.scale(vn));
            }
        }
    }

    fn apply_rolling_friction(&mut self, i: usize, j: usize, normal: Vec3, rolling_friction: f32, dt: f32) {
        if rolling_friction <= 0.0 {
            return;
        }

        if self.bodies[i].inv_mass > 0.0 {
            let spin_tangent = self.bodies[i].angular_vel.sub(normal.scale(self.bodies[i].angular_vel.dot(normal)));
            if spin_tangent.length() > 1e-6 {
                let decay = 1.0 / (1.0 + rolling_friction * dt * 8.0);
                self.bodies[i].angular_vel = self.bodies[i]
                    .angular_vel
                    .sub(spin_tangent)
                    .add(spin_tangent.scale(decay));
            }
        }
        if self.bodies[j].inv_mass > 0.0 {
            let axis = normal.neg();
            let spin_tangent = self.bodies[j].angular_vel.sub(axis.scale(self.bodies[j].angular_vel.dot(axis)));
            if spin_tangent.length() > 1e-6 {
                let decay = 1.0 / (1.0 + rolling_friction * dt * 8.0);
                self.bodies[j].angular_vel = self.bodies[j]
                    .angular_vel
                    .sub(spin_tangent)
                    .add(spin_tangent.scale(decay));
            }
        }
    }

    fn apply_support_gravity_torque(&mut self, dt: f32, contacts: &[Contact]) {
        if contacts.is_empty() || self.gravity.length_sq() <= 1e-8 {
            return;
        }

        let body_count = self.bodies.len();
        let mut support_points = vec![Vec3::ZERO; body_count];
        let mut support_normals = vec![Vec3::ZERO; body_count];
        let mut support_counts = vec![0u8; body_count];
        let gravity_up = self.gravity.neg().normalized();

        for contact in contacts.iter().copied() {
            if self.bodies[contact.i].inv_mass > 0.0
                && self.bodies[contact.j].inv_mass == 0.0
                && matches!(self.bodies[contact.i].collider, Collider::Box { .. })
                && contact.normal.dot(gravity_up) > 0.25
            {
                support_points[contact.i] = support_points[contact.i].add(contact.point);
                support_normals[contact.i] = support_normals[contact.i].add(contact.normal);
                support_counts[contact.i] = support_counts[contact.i].saturating_add(1);
            }

            if self.bodies[contact.j].inv_mass > 0.0
                && self.bodies[contact.i].inv_mass == 0.0
                && matches!(self.bodies[contact.j].collider, Collider::Box { .. })
            {
                let normal_on_j = contact.normal.neg();
                if normal_on_j.dot(gravity_up) > 0.25 {
                    support_points[contact.j] = support_points[contact.j].add(contact.point);
                    support_normals[contact.j] = support_normals[contact.j].add(normal_on_j);
                    support_counts[contact.j] = support_counts[contact.j].saturating_add(1);
                }
            }
        }

        for idx in 0..body_count {
            let count = support_counts[idx];
            if count == 0 {
                continue;
            }

            let body = &self.bodies[idx];
            if !body.dynamic() || !body.use_gravity || body.mass <= 0.0 {
                continue;
            }

            let inv_count = 1.0 / count as f32;
            let support_center = support_points[idx].scale(inv_count);
            let support_normal = support_normals[idx].scale(inv_count).normalized();
            let lever = body.position.sub(support_center);
            let planar_lever = lever.sub(support_normal.scale(lever.dot(support_normal)));
            if planar_lever.length_sq() <= 1e-8 {
                continue;
            }

            let gravity_force = self.gravity.scale(body.mass);
            let torque = planar_lever.cross(gravity_force);
            if torque.length_sq() <= 1e-8 {
                continue;
            }

            let angular_accel = self.inverse_inertia_world_mul(idx, torque);
            self.bodies[idx].angular_vel = self.bodies[idx].angular_vel.add(angular_accel.scale(dt));
            self.bodies[idx].wake();
        }
    }

    fn point_velocity(&self, idx: usize, r: Vec3) -> Vec3 {
        self.bodies[idx].velocity.add(self.bodies[idx].angular_vel.cross(r))
    }

    fn apply_world_impulse(&mut self, idx: usize, impulse: Vec3, r: Vec3) {
        if self.bodies[idx].inv_mass <= 0.0 {
            return;
        }
        self.bodies[idx].velocity = self.bodies[idx].velocity.add(impulse.scale(self.bodies[idx].inv_mass));
        let angular_impulse = self.inverse_inertia_world_mul(idx, r.cross(impulse));
        self.bodies[idx].angular_vel = self.bodies[idx].angular_vel.add(angular_impulse);
        self.bodies[idx].wake();
    }

    fn effective_mass(&self, i: usize, j: usize, r_i: Vec3, r_j: Vec3, axis: Vec3) -> f32 {
        let inv_mass = self.bodies[i].inv_mass + self.bodies[j].inv_mass;
        let ang_i = if self.bodies[i].inv_mass > 0.0 {
            let rn = r_i.cross(axis);
            self.inverse_inertia_world_mul(i, rn).cross(r_i).dot(axis)
        } else {
            0.0
        };
        let ang_j = if self.bodies[j].inv_mass > 0.0 {
            let rn = r_j.cross(axis);
            self.inverse_inertia_world_mul(j, rn).cross(r_j).dot(axis)
        } else {
            0.0
        };
        inv_mass + ang_i + ang_j
    }

    fn inverse_inertia_world_mul(&self, idx: usize, world_vec: Vec3) -> Vec3 {
        let body = &self.bodies[idx];
        if body.inv_mass <= 0.0 {
            return Vec3::ZERO;
        }
        let inv_orientation = body.orientation.conjugate();
        let local = inv_orientation.rotate_vec(world_vec);
        let inv_local = self.inv_inertia_local(idx);
        let local_scaled = Vec3::new(
            local.x * inv_local.0,
            local.y * inv_local.1,
            local.z * inv_local.2,
        );
        body.orientation.rotate_vec(local_scaled)
    }

    fn inv_inertia_local(&self, idx: usize) -> (f32, f32, f32) {
        let mass = self.bodies[idx].mass;
        if mass <= 0.0 {
            return (0.0, 0.0, 0.0);
        }
        match self.bodies[idx].collider {
            Collider::Sphere { radius } => {
                let i = 0.4 * mass * radius * radius;
                let inv = if i > 1e-6 { 1.0 / i } else { 0.0 };
                (inv, inv, inv)
            }
            Collider::Box { half_x, half_y, half_z } => {
                let ix = (mass / 3.0) * (half_y * half_y + half_z * half_z);
                let iy = (mass / 3.0) * (half_x * half_x + half_z * half_z);
                let iz = (mass / 3.0) * (half_x * half_x + half_y * half_y);
                (
                    if ix > 1e-6 { 1.0 / ix } else { 0.0 },
                    if iy > 1e-6 { 1.0 / iy } else { 0.0 },
                    if iz > 1e-6 { 1.0 / iz } else { 0.0 },
                )
            }
            Collider::Plane { .. } => (0.0, 0.0, 0.0),
        }
    }

    fn update_sleep_states(&mut self, dt: f32, touching_counts: &[u8]) {
        for (idx, body) in self.bodies.iter_mut().enumerate() {
            if !body.dynamic() {
                body.sleeping = false;
                body.sleep_timer = 0.0;
                continue;
            }

            let can_sleep = touching_counts.get(idx).copied().unwrap_or(0) > 0 || !body.use_gravity;
            let calm = body.velocity.length_sq() <= SLEEP_LINEAR_SPEED_SQ
                && body.angular_vel.length_sq() <= SLEEP_ANGULAR_SPEED_SQ
                && body.force.length_sq() <= 1e-8;

            if can_sleep && calm {
                body.sleep_timer += dt;
                if body.sleep_timer >= SLEEP_DELAY {
                    body.sleeping = true;
                    body.velocity = Vec3::ZERO;
                    body.angular_vel = Vec3::ZERO;
                }
            } else {
                body.sleep_timer = 0.0;
                body.sleeping = false;
            }
        }
    }
}
