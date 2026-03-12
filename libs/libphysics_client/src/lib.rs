#![no_std]
#![allow(unused, dead_code, static_mut_refs)]

extern crate alloc;

pub type WorldQueryFn = extern "C" fn(i32, i32, i32) -> bool;

dynlink::dll_exports! {
    lib_path: "/Libraries/libphysics.so",
    lib_struct: LibPhysics,
    symbols: {
        physics_init(query: WorldQueryFn) -> (),
        physics_create_player(x: f32, y: f32, z: f32, width: f32, height: f32) -> u32,
        physics_set_velocity(id: u32, vx: f32, vy: f32, vz: f32) -> (),
        physics_get_position(id: u32, x: *mut f32, y: *mut f32, z: *mut f32) -> (),
        physics_get_velocity(id: u32, vx: *mut f32, vy: *mut f32, vz: *mut f32) -> (),
        physics_is_on_ground(id: u32) -> bool,
        physics_set_flying(id: u32, flying: bool) -> (),
        physics_is_flying(id: u32) -> bool,
        physics_set_gravity(g: f32) -> (),
        physics_step(dt: f32) -> (),
    }
}

// ══════════════════════════════════════════════════════════════════════════════
//  Public API — thin wrappers around function pointers
// ══════════════════════════════════════════════════════════════════════════════

/// Initialize the physics engine with a world query callback.
pub fn physics_init(query: WorldQueryFn) {
    (lib().physics_init)(query);
}

/// Create a player entity at (x, y, z) with given bounding box dimensions.
pub fn create_player(x: f32, y: f32, z: f32, width: f32, height: f32) -> u32 {
    (lib().physics_create_player)(x, y, z, width, height)
}

/// Set velocity for an entity.
pub fn set_velocity(id: u32, vx: f32, vy: f32, vz: f32) {
    (lib().physics_set_velocity)(id, vx, vy, vz);
}

/// Get position of an entity.
pub fn get_position(id: u32) -> (f32, f32, f32) {
    let (mut x, mut y, mut z) = (0.0f32, 0.0f32, 0.0f32);
    (lib().physics_get_position)(id, &mut x, &mut y, &mut z);
    (x, y, z)
}

/// Get velocity of an entity.
pub fn get_velocity(id: u32) -> (f32, f32, f32) {
    let (mut vx, mut vy, mut vz) = (0.0f32, 0.0f32, 0.0f32);
    (lib().physics_get_velocity)(id, &mut vx, &mut vy, &mut vz);
    (vx, vy, vz)
}

/// Check if an entity is on the ground.
pub fn is_on_ground(id: u32) -> bool {
    (lib().physics_is_on_ground)(id)
}

/// Set flying mode for an entity.
pub fn set_flying(id: u32, flying: bool) {
    (lib().physics_set_flying)(id, flying);
}

/// Check if an entity is flying.
pub fn is_flying(id: u32) -> bool {
    (lib().physics_is_flying)(id)
}

/// Set the global gravity value.
pub fn set_gravity(g: f32) {
    (lib().physics_set_gravity)(g);
}

/// Step the physics simulation by dt seconds.
pub fn step(dt: f32) {
    (lib().physics_step)(dt);
}
