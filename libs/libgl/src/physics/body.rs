use super::math::{Quat, Vec3};

#[derive(Clone, Copy, Debug)]
pub enum Collider {
    Sphere { radius: f32 },
    Plane { normal: Vec3, d: f32 },
    Box { half_x: f32, half_y: f32, half_z: f32 },
}

#[derive(Clone, Debug)]
pub struct RigidBody {
    pub active: bool,
    pub position: Vec3,
    pub velocity: Vec3,
    pub force: Vec3,
    pub mass: f32,
    pub inv_mass: f32,
    pub restitution: f32,
    pub collider: Collider,
    pub angular_vel: Vec3,
    pub orientation: Quat,
    pub angular_damping: f32,
    pub linear_damping: f32,
    pub friction: f32,
    pub rolling_friction: f32,
    pub use_gravity: bool,
    pub soft_body: bool,
    pub softness: f32,
    pub deformation_recovery: f32,
    pub max_deformation: f32,
    pub deformation: Vec3,
    pub deformation_velocity: Vec3,
    pub(crate) sleep_timer: f32,
    pub(crate) sleeping: bool,
}

impl RigidBody {
    pub(crate) fn sphere(mass: f32, radius: f32, position: Vec3) -> Self {
        let inv_mass = if mass <= 0.0 { 0.0 } else { 1.0 / mass };
        Self {
            active: true,
            position,
            velocity: Vec3::ZERO,
            force: Vec3::ZERO,
            mass,
            inv_mass,
            restitution: 0.25,
            collider: Collider::Sphere { radius },
            angular_vel: Vec3::ZERO,
            orientation: Quat::IDENTITY,
            angular_damping: 0.18,
            linear_damping: 0.06,
            friction: 0.72,
            rolling_friction: 0.04,
            use_gravity: mass > 0.0,
            soft_body: false,
            softness: 0.0,
            deformation_recovery: 10.0,
            max_deformation: 0.0,
            deformation: Vec3::ZERO,
            deformation_velocity: Vec3::ZERO,
            sleep_timer: 0.0,
            sleeping: false,
        }
    }

    pub(crate) fn plane(normal: Vec3, d: f32) -> Self {
        Self {
            active: true,
            position: normal.scale(d),
            velocity: Vec3::ZERO,
            force: Vec3::ZERO,
            mass: 0.0,
            inv_mass: 0.0,
            restitution: 0.0,
            collider: Collider::Plane { normal, d },
            angular_vel: Vec3::ZERO,
            orientation: Quat::IDENTITY,
            angular_damping: 0.0,
            linear_damping: 0.0,
            friction: 0.85,
            rolling_friction: 0.02,
            use_gravity: false,
            soft_body: false,
            softness: 0.0,
            deformation_recovery: 10.0,
            max_deformation: 0.0,
            deformation: Vec3::ZERO,
            deformation_velocity: Vec3::ZERO,
            sleep_timer: 0.0,
            sleeping: false,
        }
    }

    pub(crate) fn box_body(mass: f32, hx: f32, hy: f32, hz: f32, position: Vec3) -> Self {
        let inv_mass = if mass <= 0.0 { 0.0 } else { 1.0 / mass };
        Self {
            active: true,
            position,
            velocity: Vec3::ZERO,
            force: Vec3::ZERO,
            mass,
            inv_mass,
            restitution: 0.10,
            collider: Collider::Box { half_x: hx, half_y: hy, half_z: hz },
            angular_vel: Vec3::ZERO,
            orientation: Quat::IDENTITY,
            angular_damping: 0.45,
            linear_damping: 0.10,
            friction: 0.82,
            rolling_friction: 0.12,
            use_gravity: mass > 0.0,
            soft_body: false,
            softness: 0.0,
            deformation_recovery: 10.0,
            max_deformation: 0.0,
            deformation: Vec3::ZERO,
            deformation_velocity: Vec3::ZERO,
            sleep_timer: 0.0,
            sleeping: false,
        }
    }

    pub(crate) fn dynamic(&self) -> bool {
        self.active && self.inv_mass > 0.0
    }

    pub fn wake(&mut self) {
        self.sleeping = false;
        self.sleep_timer = 0.0;
    }
}
