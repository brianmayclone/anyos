#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

mod aabb;
mod body;

use aabb::AABB;
use body::Body;

libheap::dll_allocator!(libsyscall::sbrk, libsyscall::mmap, libsyscall::munmap);

static mut BODIES: Vec<Body> = Vec::new();
static mut WORLD_QUERY: Option<extern "C" fn(i32, i32, i32) -> bool> = None;
static mut GRAVITY: f32 = -32.0;
static mut TERMINAL_VEL: f32 = -78.0;
static INITIALIZED: AtomicBool = AtomicBool::new(false);

fn floorf(x: f32) -> f32 {
    let i = x as i32;
    if (i as f32) > x { (i - 1) as f32 } else { i as f32 }
}

fn is_solid(x: i32, y: i32, z: i32) -> bool {
    unsafe {
        if let Some(query) = WORLD_QUERY {
            query(x, y, z)
        } else {
            false
        }
    }
}

fn get_block_colliders(bb: &AABB, dx: f32, dy: f32, dz: f32) -> Vec<AABB> {
    let mut colliders = Vec::new();

    // Expand the AABB by velocity to find all potentially colliding blocks
    let min_x = if dx < 0.0 { bb.min_x + dx } else { bb.min_x };
    let max_x = if dx > 0.0 { bb.max_x + dx } else { bb.max_x };
    let min_y = if dy < 0.0 { bb.min_y + dy } else { bb.min_y };
    let max_y = if dy > 0.0 { bb.max_y + dy } else { bb.max_y };
    let min_z = if dz < 0.0 { bb.min_z + dz } else { bb.min_z };
    let max_z = if dz > 0.0 { bb.max_z + dz } else { bb.max_z };

    let ix0 = floorf(min_x) as i32;
    let ix1 = floorf(max_x) as i32;
    let iy0 = floorf(min_y) as i32;
    let iy1 = floorf(max_y) as i32;
    let iz0 = floorf(min_z) as i32;
    let iz1 = floorf(max_z) as i32;

    for bx in ix0..=ix1 {
        for by in iy0..=iy1 {
            for bz in iz0..=iz1 {
                if is_solid(bx, by, bz) {
                    colliders.push(AABB::new(
                        bx as f32,
                        by as f32,
                        bz as f32,
                        bx as f32 + 1.0,
                        by as f32 + 1.0,
                        bz as f32 + 1.0,
                    ));
                }
            }
        }
    }

    colliders
}

fn move_body(body: &mut Body, dx: f32, dy: f32, dz: f32) {
    let bb = body.aabb();
    let colliders = get_block_colliders(&bb, dx, dy, dz);

    let orig_dy = dy;
    let mut dx = dx;
    let mut dy = dy;
    let mut dz = dz;

    // Sweep Y first (gravity axis)
    let mut current = bb;
    for c in &colliders {
        dy = c.clip_y(&current, dy);
    }
    current = current.offset(0.0, dy, 0.0);

    // Sweep X
    for c in &colliders {
        dx = c.clip_x(&current, dx);
    }
    current = current.offset(dx, 0.0, 0.0);

    // Sweep Z
    for c in &colliders {
        dz = c.clip_z(&current, dz);
    }
    let _ = current;

    body.x += dx;
    body.y += dy;
    body.z += dz;

    // If X motion was clamped, zero X velocity
    if dx.abs() < orig_dy.abs().max(0.0001) * 0.0001 && body.vx.abs() > 0.0001 {
        // Only zero if it was truly clamped (not just zero input)
    }

    // Ground detection: was moving down, but Y got clamped
    if orig_dy < 0.0 && dy > orig_dy {
        body.on_ground = true;
        body.vy = 0.0;
    } else if orig_dy >= 0.0 {
        body.on_ground = false;
    }

    // If moving up and got clamped, zero upward velocity
    if orig_dy > 0.0 && dy < orig_dy {
        body.vy = 0.0;
    }
}

// --- C API ---

#[no_mangle]
pub extern "C" fn physics_init(query: extern "C" fn(i32, i32, i32) -> bool) {
    unsafe {
        WORLD_QUERY = Some(query);
        if !INITIALIZED.load(Ordering::Relaxed) {
            BODIES = Vec::new();
            INITIALIZED.store(true, Ordering::Relaxed);
        }
    }
}

#[no_mangle]
pub extern "C" fn physics_create_player(x: f32, y: f32, z: f32, width: f32, height: f32) -> u32 {
    unsafe {
        let id = BODIES.len() as u32;
        BODIES.push(Body::new(x, y, z, width, height));
        id
    }
}

#[no_mangle]
pub extern "C" fn physics_set_velocity(id: u32, vx: f32, vy: f32, vz: f32) {
    unsafe {
        if let Some(body) = BODIES.get_mut(id as usize) {
            body.vx = vx;
            body.vy = vy;
            body.vz = vz;
        }
    }
}

#[no_mangle]
pub extern "C" fn physics_get_position(id: u32, out_x: *mut f32, out_y: *mut f32, out_z: *mut f32) {
    unsafe {
        if let Some(body) = BODIES.get(id as usize) {
            if !out_x.is_null() { *out_x = body.x; }
            if !out_y.is_null() { *out_y = body.y; }
            if !out_z.is_null() { *out_z = body.z; }
        }
    }
}

#[no_mangle]
pub extern "C" fn physics_get_velocity(id: u32, out_vx: *mut f32, out_vy: *mut f32, out_vz: *mut f32) {
    unsafe {
        if let Some(body) = BODIES.get(id as usize) {
            if !out_vx.is_null() { *out_vx = body.vx; }
            if !out_vy.is_null() { *out_vy = body.vy; }
            if !out_vz.is_null() { *out_vz = body.vz; }
        }
    }
}

#[no_mangle]
pub extern "C" fn physics_is_on_ground(id: u32) -> bool {
    unsafe {
        BODIES.get(id as usize).map_or(false, |b| b.on_ground)
    }
}

#[no_mangle]
pub extern "C" fn physics_set_flying(id: u32, flying: bool) {
    unsafe {
        if let Some(body) = BODIES.get_mut(id as usize) {
            body.flying = flying;
        }
    }
}

#[no_mangle]
pub extern "C" fn physics_is_flying(id: u32) -> bool {
    unsafe {
        BODIES.get(id as usize).map_or(false, |b| b.flying)
    }
}

#[no_mangle]
pub extern "C" fn physics_set_gravity(g: f32) {
    unsafe {
        GRAVITY = g;
    }
}

#[no_mangle]
pub extern "C" fn physics_step(dt: f32) {
    unsafe {
        let gravity = GRAVITY;
        let terminal = TERMINAL_VEL;

        for body in BODIES.iter_mut() {
            // Apply gravity if not flying
            if !body.flying {
                body.vy += gravity * dt;
                if body.vy < terminal {
                    body.vy = terminal;
                }
            }

            let dx = body.vx * dt;
            let dy = body.vy * dt;
            let dz = body.vz * dt;

            move_body(body, dx, dy, dz);
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
