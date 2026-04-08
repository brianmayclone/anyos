use super::math::Vec3;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Contact {
    pub i: usize,
    pub j: usize,
    pub normal: Vec3,
    pub penetration: f32,
    pub point: Vec3,
    pub friction: f32,
    pub rolling_friction: f32,
    pub restitution: f32,
}
