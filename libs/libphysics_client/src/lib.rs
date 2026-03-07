#![no_std]
#![allow(unused, dead_code, static_mut_refs)]

extern crate alloc;

use dynlink::{dl_open, dl_sym, DlHandle};

pub type WorldQueryFn = extern "C" fn(i32, i32, i32) -> bool;

struct LibPhysics {
    _handle: DlHandle,
    physics_init: extern "C" fn(WorldQueryFn),
    physics_create_player: extern "C" fn(f32, f32, f32, f32, f32) -> u32,
    physics_set_velocity: extern "C" fn(u32, f32, f32, f32),
    physics_get_position: extern "C" fn(u32, *mut f32, *mut f32, *mut f32),
    physics_get_velocity: extern "C" fn(u32, *mut f32, *mut f32, *mut f32),
    physics_is_on_ground: extern "C" fn(u32) -> bool,
    physics_set_flying: extern "C" fn(u32, bool),
    physics_is_flying: extern "C" fn(u32) -> bool,
    physics_set_gravity: extern "C" fn(f32),
    physics_step: extern "C" fn(f32),
}

static mut LIB: Option<LibPhysics> = None;

fn lib() -> &'static LibPhysics {
    unsafe { LIB.as_ref().expect("libphysics not loaded — call init() first") }
}

/// Resolve a function pointer from the loaded library.
unsafe fn resolve<T: Copy>(handle: &DlHandle, name: &str) -> T {
    let ptr = dl_sym(handle, name)
        .unwrap_or_else(|| panic!("libphysics: symbol not found: {}", name));
    unsafe { core::mem::transmute_copy::<*const (), T>(&ptr) }
}

// ── Initialization ──────────────────────────────────────────────────────────

/// Load libphysics.so and cache all function pointers. Returns true on success.
pub fn init() -> bool {
    let handle = match dl_open("/Libraries/libphysics.so") {
        Some(h) => h,
        None => return false,
    };

    unsafe {
        let lib = LibPhysics {
            physics_init: resolve(&handle, "physics_init"),
            physics_create_player: resolve(&handle, "physics_create_player"),
            physics_set_velocity: resolve(&handle, "physics_set_velocity"),
            physics_get_position: resolve(&handle, "physics_get_position"),
            physics_get_velocity: resolve(&handle, "physics_get_velocity"),
            physics_is_on_ground: resolve(&handle, "physics_is_on_ground"),
            physics_set_flying: resolve(&handle, "physics_set_flying"),
            physics_is_flying: resolve(&handle, "physics_is_flying"),
            physics_set_gravity: resolve(&handle, "physics_set_gravity"),
            physics_step: resolve(&handle, "physics_step"),
            _handle: handle,
        };
        LIB = Some(lib);
    }
    true
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
