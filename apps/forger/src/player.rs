use crate::block;
use crate::world::World;
use libgl_client as gl;
use libphysics_client as physics;

fn floor(x: f32) -> i32 {
    let i = x as i32;
    if (i as f32) > x { i - 1 } else { i }
}

pub const EYE_HEIGHT: f32 = 1.62;
pub const PLAYER_WIDTH: f32 = 0.6;
pub const PLAYER_HEIGHT: f32 = 1.8;
pub const WALK_SPEED: f32 = 4.317;
pub const FLY_SPEED: f32 = 10.0;
pub const JUMP_VEL: f32 = 8.5;
pub const MOUSE_SENS: f32 = 0.006;
pub const KEY_LOOK_SPEED: f32 = 2.8;
pub const REACH: f32 = 5.0;

pub struct Player {
    pub body_id: u32,
    pub yaw: f32,
    pub pitch: f32,
    pub selected_block: u8,
    pub forward: bool,
    pub backward: bool,
    pub left: bool,
    pub right: bool,
    pub jump: bool,
    pub descend: bool,
    pub ascend: bool,
    pub look_left: bool,
    pub look_right: bool,
    pub look_up: bool,
    pub look_down: bool,
}

impl Player {
    /// Create a placeholder player (no physics body). Used before physics_init.
    pub fn new_uninit() -> Self {
        Self {
            body_id: u32::MAX,
            yaw: 0.0,
            pitch: 0.0,
            selected_block: block::STONE,
            forward: false,
            backward: false,
            left: false,
            right: false,
            jump: false,
            descend: false,
            ascend: false,
            look_left: false,
            look_right: false,
            look_up: false,
            look_down: false,
        }
    }

    pub fn new(x: f32, y: f32, z: f32) -> Self {
        let body_id = physics::create_player(x, y, z, PLAYER_WIDTH, PLAYER_HEIGHT);
        Self {
            body_id,
            yaw: 0.0,
            pitch: 0.0,
            selected_block: block::STONE,
            forward: false,
            backward: false,
            left: false,
            right: false,
            jump: false,
            descend: false,
            ascend: false,
            look_left: false,
            look_right: false,
            look_up: false,
            look_down: false,
        }
    }

    pub fn position(&self) -> (f32, f32, f32) {
        physics::get_position(self.body_id)
    }

    pub fn eye_position(&self) -> (f32, f32, f32) {
        let (x, y, z) = physics::get_position(self.body_id);
        (x, y + EYE_HEIGHT, z)
    }

    pub fn update(&mut self, dt: f32) {
        let mut look_dx = 0.0f32;
        let mut look_dy = 0.0f32;
        if self.look_left {
            look_dx -= KEY_LOOK_SPEED * dt;
        }
        if self.look_right {
            look_dx += KEY_LOOK_SPEED * dt;
        }
        if self.look_up {
            look_dy -= KEY_LOOK_SPEED * dt;
        }
        if self.look_down {
            look_dy += KEY_LOOK_SPEED * dt;
        }
        if look_dx != 0.0 || look_dy != 0.0 {
            self.look_by(look_dx, look_dy);
        }

        let fwd_x = gl::sin(self.yaw);
        let fwd_z = -gl::cos(self.yaw);
        let right_x = gl::cos(self.yaw);
        let right_z = gl::sin(self.yaw);

        let mut dx = 0.0f32;
        let mut dz = 0.0f32;

        if self.forward {
            dx += fwd_x;
            dz += fwd_z;
        }
        if self.backward {
            dx -= fwd_x;
            dz -= fwd_z;
        }
        if self.right {
            dx += right_x;
            dz += right_z;
        }
        if self.left {
            dx -= right_x;
            dz -= right_z;
        }

        let len = gl::sqrt(dx * dx + dz * dz);
        if len > 0.0 {
            dx /= len;
            dz /= len;
        }

        let flying = physics::is_flying(self.body_id);
        let speed = if flying { FLY_SPEED } else { WALK_SPEED };

        let vx = dx * speed;
        let vz = dz * speed;

        let vy = if flying {
            let mut v = 0.0f32;
            if self.ascend || self.jump {
                v += FLY_SPEED;
            }
            if self.descend {
                v -= FLY_SPEED;
            }
            v
        } else if self.jump && physics::is_on_ground(self.body_id) {
            JUMP_VEL
        } else {
            let (_, cur_vy, _) = physics::get_velocity(self.body_id);
            cur_vy
        };

        physics::set_velocity(self.body_id, vx, vy, vz);
        physics::step(dt);
    }

    pub fn mouse_move(&mut self, dx: f32, dy: f32) {
        self.look_by(dx * MOUSE_SENS, dy * MOUSE_SENS);
    }

    pub fn look_by(&mut self, dx: f32, dy: f32) {
        self.yaw += dx;
        self.pitch += dy;
        if self.pitch < -1.5 {
            self.pitch = -1.5;
        }
        if self.pitch > 1.5 {
            self.pitch = 1.5;
        }
    }

    pub fn toggle_fly(&mut self) {
        let flying = physics::is_flying(self.body_id);
        physics::set_flying(self.body_id, !flying);
    }

    pub fn set_flying(&mut self, flying: bool) {
        physics::set_flying(self.body_id, flying);
    }

    pub fn is_flying(&self) -> bool {
        physics::is_flying(self.body_id)
    }

    pub fn scroll_block(&mut self, delta: i32) {
        let min = 1i32;
        let max = block::TORCH as i32;
        let range = max - min + 1;
        let mut cur = self.selected_block as i32 - min;
        cur = ((cur + delta) % range + range) % range;
        self.selected_block = (cur + min) as u8;
    }
}

pub struct RayHit {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub prev_x: i32,
    pub prev_y: i32,
    pub prev_z: i32,
}

pub fn raycast(world: &World, ox: f32, oy: f32, oz: f32, yaw: f32, pitch: f32) -> Option<RayHit> {
    let dir_x = gl::sin(yaw) * gl::cos(pitch);
    let dir_y = -gl::sin(pitch);
    let dir_z = -gl::cos(yaw) * gl::cos(pitch);

    let mut x = floor(ox);
    let mut y = floor(oy);
    let mut z = floor(oz);

    let step_x: i32 = if dir_x > 0.0 { 1 } else { -1 };
    let step_y: i32 = if dir_y > 0.0 { 1 } else { -1 };
    let step_z: i32 = if dir_z > 0.0 { 1 } else { -1 };

    let t_delta_x = if dir_x != 0.0 { (1.0 / dir_x).abs() } else { f32::MAX };
    let t_delta_y = if dir_y != 0.0 { (1.0 / dir_y).abs() } else { f32::MAX };
    let t_delta_z = if dir_z != 0.0 { (1.0 / dir_z).abs() } else { f32::MAX };

    let mut t_max_x = if dir_x > 0.0 {
        ((x as f32 + 1.0) - ox) * t_delta_x
    } else {
        (ox - x as f32) * t_delta_x
    };
    let mut t_max_y = if dir_y > 0.0 {
        ((y as f32 + 1.0) - oy) * t_delta_y
    } else {
        (oy - y as f32) * t_delta_y
    };
    let mut t_max_z = if dir_z > 0.0 {
        ((z as f32 + 1.0) - oz) * t_delta_z
    } else {
        (oz - z as f32) * t_delta_z
    };

    let mut prev_x = x;
    let mut prev_y = y;
    let mut prev_z = z;

    let reach_sq = REACH * REACH;

    loop {
        let ddx = x as f32 + 0.5 - ox;
        let ddy = y as f32 + 0.5 - oy;
        let ddz = z as f32 + 0.5 - oz;
        if ddx * ddx + ddy * ddy + ddz * ddz > reach_sq + 1.0 {
            return None;
        }

        let b = world.get_block(x, y, z);
        if block::is_solid(b) {
            return Some(RayHit {
                x,
                y,
                z,
                prev_x,
                prev_y,
                prev_z,
            });
        }

        prev_x = x;
        prev_y = y;
        prev_z = z;

        if t_max_x < t_max_y {
            if t_max_x < t_max_z {
                x += step_x;
                t_max_x += t_delta_x;
            } else {
                z += step_z;
                t_max_z += t_delta_z;
            }
        } else if t_max_y < t_max_z {
            y += step_y;
            t_max_y += t_delta_y;
        } else {
            z += step_z;
            t_max_z += t_delta_z;
        }
    }
}
