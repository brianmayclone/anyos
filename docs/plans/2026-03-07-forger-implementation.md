# Forger Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build "Forger", a Minecraft Creative-mode clone with physics, procedural world generation, greedy meshing, adaptive fog-based draw distance, and day/night cycle running at 50+ FPS in software rendering on anyOS.

**Architecture:** Separate `libphysics` library for AABB voxel collision. Forger app uses libgl for OpenGL ES 2.0 rendering, libphysics for player physics, libanyui for windowing. Chunks with greedy meshing, frustum culling, and distance fog.

**Tech Stack:** Rust (no_std), libgl (OpenGL ES 2.0), libanyui (windowing), custom libphysics, anyos_std

---

### Task 1: Create libphysics library skeleton

**Files:**
- Create: `libs/libphysics/Cargo.toml`
- Create: `libs/libphysics/src/lib.rs`
- Create: `libs/libphysics/src/aabb.rs`
- Create: `libs/libphysics/src/body.rs`

**Step 1: Create Cargo.toml**

```toml
[package]
name = "libphysics"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["staticlib"]

[dependencies]
libheap = { path = "../libheap" }
libsyscall = { path = "../libsyscall" }

[profile.dev]
panic = "abort"
opt-level = 2

[profile.release]
panic = "abort"

[workspace]
```

**Step 2: Create src/aabb.rs**

```rust
/// Axis-Aligned Bounding Box for collision detection.
#[derive(Clone, Copy)]
pub struct Aabb {
    pub min_x: f32,
    pub min_y: f32,
    pub min_z: f32,
    pub max_x: f32,
    pub max_y: f32,
    pub max_z: f32,
}

impl Aabb {
    #[inline]
    pub fn new(min_x: f32, min_y: f32, min_z: f32, max_x: f32, max_y: f32, max_z: f32) -> Self {
        Self { min_x, min_y, min_z, max_x, max_y, max_z }
    }

    #[inline]
    pub fn from_center(cx: f32, cy: f32, cz: f32, hw: f32, hh: f32, hd: f32) -> Self {
        Self::new(cx - hw, cy - hh, cz - hd, cx + hw, cy + hh, cz + hd)
    }

    #[inline]
    pub fn intersects(&self, other: &Aabb) -> bool {
        self.min_x < other.max_x && self.max_x > other.min_x &&
        self.min_y < other.max_y && self.max_y > other.min_y &&
        self.min_z < other.max_z && self.max_z > other.min_z
    }

    #[inline]
    pub fn offset(&self, dx: f32, dy: f32, dz: f32) -> Self {
        Self::new(
            self.min_x + dx, self.min_y + dy, self.min_z + dz,
            self.max_x + dx, self.max_y + dy, self.max_z + dz,
        )
    }

    /// Sweep test along Y axis: how far can self move in dy before hitting other?
    /// Returns clamped dy.
    pub fn clip_y(&self, other: &Aabb, mut dy: f32) -> f32 {
        if self.max_x <= other.min_x || self.min_x >= other.max_x { return dy; }
        if self.max_z <= other.min_z || self.min_z >= other.max_z { return dy; }
        if dy > 0.0 && self.max_y <= other.min_y {
            let d = other.min_y - self.max_y;
            if d < dy { dy = d; }
        }
        if dy < 0.0 && self.min_y >= other.max_y {
            let d = other.max_y - self.min_y;
            if d > dy { dy = d; }
        }
        dy
    }

    /// Sweep test along X axis.
    pub fn clip_x(&self, other: &Aabb, mut dx: f32) -> f32 {
        if self.max_y <= other.min_y || self.min_y >= other.max_y { return dx; }
        if self.max_z <= other.min_z || self.min_z >= other.max_z { return dx; }
        if dx > 0.0 && self.max_x <= other.min_x {
            let d = other.min_x - self.max_x;
            if d < dx { dx = d; }
        }
        if dx < 0.0 && self.min_x >= other.max_x {
            let d = other.max_x - self.min_x;
            if d > dx { dx = d; }
        }
        dx
    }

    /// Sweep test along Z axis.
    pub fn clip_z(&self, other: &Aabb, mut dz: f32) -> f32 {
        if self.max_x <= other.min_x || self.min_x >= other.max_x { return dz; }
        if self.max_y <= other.min_y || self.min_y >= other.max_y { return dz; }
        if dz > 0.0 && self.max_z <= other.min_z {
            let d = other.min_z - self.max_z;
            if d < dz { dz = d; }
        }
        if dz < 0.0 && self.min_z >= other.max_z {
            let d = other.max_z - self.min_z;
            if d > dz { dz = d; }
        }
        dz
    }
}
```

**Step 3: Create src/body.rs**

```rust
use crate::aabb::Aabb;

/// A physics body — axis-aligned box with velocity.
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
        Self { x, y, z, vx: 0.0, vy: 0.0, vz: 0.0, width, height, on_ground: false, flying: false }
    }

    pub fn aabb(&self) -> Aabb {
        let hw = self.width * 0.5;
        let hd = hw;
        Aabb::new(
            self.x - hw, self.y, self.z - hd,
            self.x + hw, self.y + self.height, self.z + hd,
        )
    }
}
```

**Step 4: Create src/lib.rs — physics API exports**

```rust
#![no_std]
#![no_main]
#![allow(unused, dead_code, static_mut_refs)]

extern crate alloc;

pub mod aabb;
pub mod body;

use body::Body;
use aabb::Aabb;
use alloc::vec::Vec;

// ── Heap + panic (same pattern as libgl) ─────────────────────────────

#[global_allocator]
static ALLOCATOR: libheap::Allocator = libheap::Allocator;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

// ── Global state ─────────────────────────────────────────────────────

type WorldQueryFn = extern "C" fn(i32, i32, i32) -> bool;

static mut BODIES: Vec<Body> = Vec::new();
static mut WORLD_QUERY: Option<WorldQueryFn> = None;
static mut GRAVITY: f32 = -32.0; // ~MC gravity
static mut TERMINAL_VEL: f32 = -78.0;

fn is_solid(x: i32, y: i32, z: i32) -> bool {
    unsafe {
        match WORLD_QUERY {
            Some(f) => f(x, y, z),
            None => false,
        }
    }
}

fn floor(x: f32) -> i32 {
    let i = x as i32;
    if (i as f32) > x { i - 1 } else { i }
}

/// Collect all solid block AABBs that might intersect the given expanded AABB.
fn get_block_colliders(bb: &Aabb, dx: f32, dy: f32, dz: f32) -> Vec<Aabb> {
    let expanded = Aabb::new(
        if dx < 0.0 { bb.min_x + dx } else { bb.min_x },
        if dy < 0.0 { bb.min_y + dy } else { bb.min_y },
        if dz < 0.0 { bb.min_z + dz } else { bb.min_z },
        if dx > 0.0 { bb.max_x + dx } else { bb.max_x },
        if dy > 0.0 { bb.max_y + dy } else { bb.max_y },
        if dz > 0.0 { bb.max_z + dz } else { bb.max_z },
    );
    let mut out = Vec::new();
    let x0 = floor(expanded.min_x);
    let x1 = floor(expanded.max_x) + 1;
    let y0 = floor(expanded.min_y);
    let y1 = floor(expanded.max_y) + 1;
    let z0 = floor(expanded.min_z);
    let z1 = floor(expanded.max_z) + 1;
    for bx in x0..x1 {
        for by in y0..y1 {
            for bz in z0..z1 {
                if is_solid(bx, by, bz) {
                    out.push(Aabb::new(
                        bx as f32, by as f32, bz as f32,
                        bx as f32 + 1.0, by as f32 + 1.0, bz as f32 + 1.0,
                    ));
                }
            }
        }
    }
    out
}

/// Move-and-slide: resolve movement axis by axis (Y first for ground detection).
fn move_body(body: &mut Body, mut dx: f32, mut dy: f32, mut dz: f32) {
    let bb = body.aabb();
    let colliders = get_block_colliders(&bb, dx, dy, dz);

    // Y axis first (gravity / jumping)
    let orig_dy = dy;
    for c in &colliders {
        dy = bb.clip_y(c, dy);
    }
    let bb = bb.offset(0.0, dy, 0.0);

    // X axis
    let orig_dx = dx;
    for c in &colliders {
        dx = bb.clip_x(c, dx);
    }
    let bb = bb.offset(dx, 0.0, 0.0);

    // Z axis
    for c in &colliders {
        dz = bb.clip_z(c, dz);
    }

    body.x += dx;
    body.y += dy;
    body.z += dz;

    // Ground detection
    if orig_dy < 0.0 && dy > orig_dy {
        body.on_ground = true;
        body.vy = 0.0;
    } else if orig_dy != dy {
        body.vy = 0.0;
    }
    if orig_dx != dx { body.vx = 0.0; }
    // on_ground only set when falling was stopped
    if dy == orig_dy && orig_dy < 0.0 {
        body.on_ground = false;
    }
}

// ── C API exports ────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn physics_init(query: WorldQueryFn) {
    unsafe {
        WORLD_QUERY = Some(query);
        BODIES.clear();
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
        if let Some(b) = BODIES.get_mut(id as usize) {
            b.vx = vx;
            b.vy = vy;
            b.vz = vz;
        }
    }
}

#[no_mangle]
pub extern "C" fn physics_get_position(id: u32, out_x: *mut f32, out_y: *mut f32, out_z: *mut f32) {
    unsafe {
        if let Some(b) = BODIES.get(id as usize) {
            *out_x = b.x;
            *out_y = b.y;
            *out_z = b.z;
        }
    }
}

#[no_mangle]
pub extern "C" fn physics_get_velocity(id: u32, out_vx: *mut f32, out_vy: *mut f32, out_vz: *mut f32) {
    unsafe {
        if let Some(b) = BODIES.get(id as usize) {
            *out_vx = b.vx;
            *out_vy = b.vy;
            *out_vz = b.vz;
        }
    }
}

#[no_mangle]
pub extern "C" fn physics_is_on_ground(id: u32) -> bool {
    unsafe { BODIES.get(id as usize).map(|b| b.on_ground).unwrap_or(false) }
}

#[no_mangle]
pub extern "C" fn physics_set_flying(id: u32, flying: bool) {
    unsafe {
        if let Some(b) = BODIES.get_mut(id as usize) {
            b.flying = flying;
        }
    }
}

#[no_mangle]
pub extern "C" fn physics_is_flying(id: u32) -> bool {
    unsafe { BODIES.get(id as usize).map(|b| b.flying).unwrap_or(false) }
}

#[no_mangle]
pub extern "C" fn physics_set_gravity(g: f32) {
    unsafe { GRAVITY = g; }
}

#[no_mangle]
pub extern "C" fn physics_step(dt: f32) {
    unsafe {
        for body in BODIES.iter_mut() {
            if !body.flying {
                body.vy += GRAVITY * dt;
                if body.vy < TERMINAL_VEL {
                    body.vy = TERMINAL_VEL;
                }
            }
            let dx = body.vx * dt;
            let dy = body.vy * dt;
            let dz = body.vz * dt;
            move_body(body, dx, dy, dz);
        }
    }
}
```

**Step 5: Commit**

```bash
git add libs/libphysics/
git commit -m "feat: add libphysics library with AABB collision and voxel physics"
```

---

### Task 2: Create libphysics_client wrapper

**Files:**
- Create: `libs/libphysics_client/Cargo.toml`
- Create: `libs/libphysics_client/src/lib.rs`

**Step 1: Create Cargo.toml**

```toml
[package]
name = "libphysics_client"
version = "0.1.0"
edition = "2021"

[dependencies]
dynlink = { path = "../dynlink" }
anyos_std = { path = "../stdlib" }

[lib]
name = "libphysics_client"
```

**Step 2: Create src/lib.rs**

```rust
#![no_std]
#![allow(unused, dead_code)]

extern crate alloc;

use dynlink::{dl_open, dl_sym, DlHandle};

type WorldQueryFn = extern "C" fn(i32, i32, i32) -> bool;

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
    unsafe { LIB.as_ref().expect("libphysics not initialized") }
}

unsafe fn resolve<T>(h: &DlHandle, name: &str) -> T {
    let ptr = dl_sym(h, name).unwrap_or_else(|| panic!("missing symbol: {}", name));
    core::mem::transmute_copy(&ptr)
}

pub fn load() {
    unsafe {
        let h = dl_open("libphysics.so").expect("failed to load libphysics.so");
        LIB = Some(LibPhysics {
            physics_init: resolve(&h, "physics_init"),
            physics_create_player: resolve(&h, "physics_create_player"),
            physics_set_velocity: resolve(&h, "physics_set_velocity"),
            physics_get_position: resolve(&h, "physics_get_position"),
            physics_get_velocity: resolve(&h, "physics_get_velocity"),
            physics_is_on_ground: resolve(&h, "physics_is_on_ground"),
            physics_set_flying: resolve(&h, "physics_set_flying"),
            physics_is_flying: resolve(&h, "physics_is_flying"),
            physics_set_gravity: resolve(&h, "physics_set_gravity"),
            physics_step: resolve(&h, "physics_step"),
            _handle: h,
        });
    }
}

pub fn init(query: WorldQueryFn) {
    (lib().physics_init)(query);
}

pub fn create_player(x: f32, y: f32, z: f32, width: f32, height: f32) -> u32 {
    (lib().physics_create_player)(x, y, z, width, height)
}

pub fn set_velocity(id: u32, vx: f32, vy: f32, vz: f32) {
    (lib().physics_set_velocity)(id, vx, vy, vz);
}

pub fn get_position(id: u32) -> (f32, f32, f32) {
    let (mut x, mut y, mut z) = (0.0f32, 0.0f32, 0.0f32);
    (lib().physics_get_position)(id, &mut x, &mut y, &mut z);
    (x, y, z)
}

pub fn get_velocity(id: u32) -> (f32, f32, f32) {
    let (mut vx, mut vy, mut vz) = (0.0f32, 0.0f32, 0.0f32);
    (lib().physics_get_velocity)(id, &mut vx, &mut vy, &mut vz);
    (vx, vy, vz)
}

pub fn is_on_ground(id: u32) -> bool {
    (lib().physics_is_on_ground)(id)
}

pub fn set_flying(id: u32, flying: bool) {
    (lib().physics_set_flying)(id, flying);
}

pub fn is_flying(id: u32) -> bool {
    (lib().physics_is_flying)(id)
}

pub fn set_gravity(g: f32) {
    (lib().physics_set_gravity)(g);
}

pub fn step(dt: f32) {
    (lib().physics_step)(dt);
}
```

**Step 3: Commit**

```bash
git add libs/libphysics_client/
git commit -m "feat: add libphysics_client dynlink wrapper"
```

---

### Task 3: Create Forger app skeleton with window and GL init

**Files:**
- Create: `apps/forger/Cargo.toml`
- Create: `apps/forger/build.rs`
- Create: `apps/forger/Info.conf`
- Create: `apps/forger/src/main.rs`
- Create: `apps/forger/src/block.rs`

**Step 1: Create Cargo.toml**

```toml
[package]
name = "forger"
version = "0.1.0"
edition = "2021"

[dependencies]
anyos_std = { path = "../../libs/stdlib" }
dynlink = { path = "../../libs/dynlink" }
libanyui_client = { path = "../../libs/libanyui_client" }
libgl_client = { path = "../../libs/libgl_client" }
libphysics_client = { path = "../../libs/libphysics_client" }

[profile.dev]
panic = "abort"
opt-level = 2

[profile.release]
panic = "abort"
```

**Step 2: Create build.rs**

```rust
fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let project_root = std::path::PathBuf::from(&manifest_dir)
        .parent().unwrap()
        .parent().unwrap()
        .to_path_buf();
    let link_ld = project_root.join("libs").join("stdlib").join("link.ld");
    println!("cargo:rustc-link-arg=-T{}", link_ld.display());
    println!("cargo:rerun-if-changed={}", link_ld.display());
}
```

**Step 3: Create Info.conf**

```
id=com.anyos.forger
name=Forger
exec=Forger
version=1.0
category=Games
capabilities=dll,event
```

**Step 4: Create src/block.rs**

```rust
/// Block type IDs.
pub const AIR: u8 = 0;
pub const GRASS: u8 = 1;
pub const DIRT: u8 = 2;
pub const STONE: u8 = 3;
pub const SAND: u8 = 4;
pub const GRAVEL: u8 = 5;
pub const WOOD: u8 = 6;
pub const LEAVES: u8 = 7;
pub const WATER: u8 = 8;
pub const BEDROCK: u8 = 9;
pub const COAL_ORE: u8 = 10;
pub const IRON_ORE: u8 = 11;
pub const GOLD_ORE: u8 = 12;
pub const DIAMOND_ORE: u8 = 13;
pub const PLANKS: u8 = 14;
pub const BRICKS: u8 = 15;
pub const COBBLESTONE: u8 = 16;
pub const SNOW: u8 = 17;
pub const GLASS: u8 = 18;
pub const CRAFTING_TABLE: u8 = 19;
pub const TORCH: u8 = 20;

pub const BLOCK_COUNT: usize = 21; // including AIR

/// Block names for HUD display.
pub const BLOCK_NAMES: [&str; BLOCK_COUNT] = [
    "Air", "Grass", "Dirt", "Stone", "Sand", "Gravel", "Wood", "Leaves",
    "Water", "Bedrock", "Coal Ore", "Iron Ore", "Gold Ore", "Diamond Ore",
    "Planks", "Bricks", "Cobblestone", "Snow", "Glass", "Crafting Table", "Torch",
];

/// Whether the block is solid (has collision).
#[inline]
pub fn is_solid(id: u8) -> bool {
    id != AIR && id != WATER && id != TORCH
}

/// Whether the block is transparent (neighbors should render their face).
#[inline]
pub fn is_transparent(id: u8) -> bool {
    id == AIR || id == WATER || id == GLASS || id == LEAVES || id == TORCH
}

/// Whether the block is a light source.
#[inline]
pub fn is_light_source(id: u8) -> bool {
    id == TORCH
}
```

**Step 5: Create src/main.rs — minimal skeleton**

```rust
//! Forger — a Minecraft-like voxel game for anyOS (Creative mode).

#![no_std]
#![no_main]
#![allow(unused, dead_code, static_mut_refs)]

extern crate alloc;

anyos_std::entry!(main);

use alloc::vec;
use alloc::vec::Vec;
use alloc::string::String;

use libgl_client as gl;
use libphysics_client as physics;

mod block;

fn main() {
    // TODO: Initialize window via libanyui
    // TODO: Initialize GL
    // TODO: Load physics
    // TODO: Generate world
    // TODO: Enter game loop
}
```

**Step 6: Add forger to workspace Cargo.toml members**

Read `Cargo.toml` at project root, find `[workspace] members = [...]` and add `"apps/forger"`.

**Step 7: Commit**

```bash
git add apps/forger/
git commit -m "feat: add forger app skeleton with block definitions"
```

---

### Task 4: Implement Simplex Noise

**Files:**
- Create: `apps/forger/src/noise.rs`

**Step 1: Create noise.rs with 2D and 3D simplex noise**

```rust
//! Simplex noise for terrain generation.
//! 2D for heightmap, 3D for caves and ore placement.

use libgl_client as gl;

/// Permutation table (doubled for wrapping).
static PERM: [u8; 512] = {
    let base: [u8; 256] = [
        151,160,137,91,90,15,131,13,201,95,96,53,194,233,7,225,
        140,36,103,30,69,142,8,99,37,240,21,10,23,190,6,148,
        247,120,234,75,0,26,197,62,94,252,219,203,117,35,11,32,
        57,177,33,88,237,149,56,87,174,20,125,136,171,168,68,175,
        74,165,71,134,139,48,27,166,77,146,158,231,83,111,229,122,
        60,211,133,230,220,105,92,41,55,46,245,40,244,102,143,54,
        65,25,63,161,1,216,80,73,209,76,132,187,208,89,18,169,
        200,196,135,130,116,188,159,86,164,100,109,198,173,186,3,64,
        52,217,226,250,124,123,5,202,38,147,118,126,255,82,85,212,
        207,206,59,227,47,16,58,17,182,189,28,42,223,183,170,213,
        119,248,152,2,44,154,163,70,221,153,101,155,167,43,172,9,
        129,22,39,253,19,98,108,110,79,113,224,232,178,185,112,104,
        218,246,97,228,251,34,242,193,238,210,144,12,191,179,162,241,
        81,51,145,235,249,14,239,107,49,192,214,31,181,199,106,157,
        184,84,204,176,115,121,50,45,127,4,150,254,138,236,205,93,
        222,114,67,29,24,72,243,141,128,195,78,66,215,61,156,180,
    ];
    let mut p = [0u8; 512];
    let mut i = 0;
    while i < 512 {
        p[i] = base[i & 255];
        i += 1;
    }
    p
};

static GRAD3: [[f32; 3]; 12] = [
    [1.0,1.0,0.0],[-1.0,1.0,0.0],[1.0,-1.0,0.0],[-1.0,-1.0,0.0],
    [1.0,0.0,1.0],[-1.0,0.0,1.0],[1.0,0.0,-1.0],[-1.0,0.0,-1.0],
    [0.0,1.0,1.0],[0.0,-1.0,1.0],[0.0,1.0,-1.0],[0.0,-1.0,-1.0],
];

fn floor(x: f32) -> i32 {
    let i = x as i32;
    if (i as f32) > x { i - 1 } else { i }
}

fn dot2(g: &[f32; 3], x: f32, y: f32) -> f32 {
    g[0] * x + g[1] * y
}

fn dot3(g: &[f32; 3], x: f32, y: f32, z: f32) -> f32 {
    g[0] * x + g[1] * y + g[2] * z
}

/// 2D simplex noise, returns value in [-1, 1].
pub fn noise2d(xin: f32, yin: f32) -> f32 {
    const F2: f32 = 0.3660254;  // (sqrt(3)-1)/2
    const G2: f32 = 0.2113249;  // (3-sqrt(3))/6

    let s = (xin + yin) * F2;
    let i = floor(xin + s);
    let j = floor(yin + s);
    let t = (i + j) as f32 * G2;
    let x0 = xin - (i as f32 - t);
    let y0 = yin - (j as f32 - t);

    let (i1, j1) = if x0 > y0 { (1, 0) } else { (0, 1) };
    let x1 = x0 - i1 as f32 + G2;
    let y1 = y0 - j1 as f32 + G2;
    let x2 = x0 - 1.0 + 2.0 * G2;
    let y2 = y0 - 1.0 + 2.0 * G2;

    let ii = (i & 255) as usize;
    let jj = (j & 255) as usize;

    let mut n = 0.0f32;

    let mut t0 = 0.5 - x0 * x0 - y0 * y0;
    if t0 > 0.0 {
        t0 *= t0;
        let gi = PERM[ii + PERM[jj] as usize] as usize % 12;
        n += t0 * t0 * dot2(&GRAD3[gi], x0, y0);
    }

    let mut t1 = 0.5 - x1 * x1 - y1 * y1;
    if t1 > 0.0 {
        t1 *= t1;
        let gi = PERM[ii + i1 + PERM[jj + j1] as usize] as usize % 12;
        n += t1 * t1 * dot2(&GRAD3[gi], x1, y1);
    }

    let mut t2 = 0.5 - x2 * x2 - y2 * y2;
    if t2 > 0.0 {
        t2 *= t2;
        let gi = PERM[ii + 1 + PERM[jj + 1] as usize] as usize % 12;
        n += t2 * t2 * dot2(&GRAD3[gi], x2, y2);
    }

    70.0 * n
}

/// 3D simplex noise, returns value in [-1, 1].
pub fn noise3d(xin: f32, yin: f32, zin: f32) -> f32 {
    const F3: f32 = 1.0 / 3.0;
    const G3: f32 = 1.0 / 6.0;

    let s = (xin + yin + zin) * F3;
    let i = floor(xin + s);
    let j = floor(yin + s);
    let k = floor(zin + s);
    let t = (i + j + k) as f32 * G3;
    let x0 = xin - (i as f32 - t);
    let y0 = yin - (j as f32 - t);
    let z0 = zin - (k as f32 - t);

    let (i1, j1, k1, i2, j2, k2);
    if x0 >= y0 {
        if y0 >= z0 { i1=1; j1=0; k1=0; i2=1; j2=1; k2=0; }
        else if x0 >= z0 { i1=1; j1=0; k1=0; i2=1; j2=0; k2=1; }
        else { i1=0; j1=0; k1=1; i2=1; j2=0; k2=1; }
    } else {
        if y0 < z0 { i1=0; j1=0; k1=1; i2=0; j2=1; k2=1; }
        else if x0 < z0 { i1=0; j1=1; k1=0; i2=0; j2=1; k2=1; }
        else { i1=0; j1=1; k1=0; i2=1; j2=1; k2=0; }
    }

    let x1 = x0 - i1 as f32 + G3;
    let y1 = y0 - j1 as f32 + G3;
    let z1 = z0 - k1 as f32 + G3;
    let x2 = x0 - i2 as f32 + 2.0 * G3;
    let y2 = y0 - j2 as f32 + 2.0 * G3;
    let z2 = z0 - k2 as f32 + 2.0 * G3;
    let x3 = x0 - 1.0 + 3.0 * G3;
    let y3 = y0 - 1.0 + 3.0 * G3;
    let z3 = z0 - 1.0 + 3.0 * G3;

    let ii = (i & 255) as usize;
    let jj = (j & 255) as usize;
    let kk = (k & 255) as usize;

    let mut n = 0.0f32;

    let mut t0 = 0.6 - x0*x0 - y0*y0 - z0*z0;
    if t0 > 0.0 {
        t0 *= t0;
        let gi = PERM[ii + PERM[jj + PERM[kk] as usize] as usize] as usize % 12;
        n += t0 * t0 * dot3(&GRAD3[gi], x0, y0, z0);
    }
    let mut t1 = 0.6 - x1*x1 - y1*y1 - z1*z1;
    if t1 > 0.0 {
        t1 *= t1;
        let gi = PERM[ii+i1 + PERM[jj+j1 + PERM[kk+k1] as usize] as usize] as usize % 12;
        n += t1 * t1 * dot3(&GRAD3[gi], x1, y1, z1);
    }
    let mut t2 = 0.6 - x2*x2 - y2*y2 - z2*z2;
    if t2 > 0.0 {
        t2 *= t2;
        let gi = PERM[ii+i2 + PERM[jj+j2 + PERM[kk+k2] as usize] as usize] as usize % 12;
        n += t2 * t2 * dot3(&GRAD3[gi], x2, y2, z2);
    }
    let mut t3 = 0.6 - x3*x3 - y3*y3 - z3*z3;
    if t3 > 0.0 {
        t3 *= t3;
        let gi = PERM[ii+1 + PERM[jj+1 + PERM[kk+1] as usize] as usize] as usize % 12;
        n += t3 * t3 * dot3(&GRAD3[gi], x3, y3, z3);
    }
    32.0 * n
}

/// Fractal Brownian Motion — layered noise for terrain.
pub fn fbm2d(x: f32, y: f32, octaves: u32, persistence: f32) -> f32 {
    let mut total = 0.0f32;
    let mut amplitude = 1.0f32;
    let mut frequency = 1.0f32;
    let mut max_val = 0.0f32;
    for _ in 0..octaves {
        total += noise2d(x * frequency, y * frequency) * amplitude;
        max_val += amplitude;
        amplitude *= persistence;
        frequency *= 2.0;
    }
    total / max_val
}
```

**Step 2: Add `mod noise;` to main.rs**

**Step 3: Commit**

```bash
git add apps/forger/src/noise.rs
git commit -m "feat(forger): add simplex noise for terrain generation"
```

---

### Task 5: Implement chunk/world system

**Files:**
- Create: `apps/forger/src/world.rs`

**Step 1: Create world.rs**

Implements:
- `Chunk` struct: `[u8; 16*256*16]` block data
- `World` struct: `HashMap<(i32, i32), Chunk>`, seed
- `World::generate_chunk(cx, cz)`: Perlin heightmap, bedrock layer, stone fill, dirt/grass top, water at y=64, caves via 3D noise, ores by height, trees
- `World::get_block(x, y, z) -> u8`
- `World::set_block(x, y, z, id)`
- `World::ensure_chunks_around(px, pz, radius)`: generate missing chunks in radius
- `world_query_callback`: extern "C" fn for libphysics

Key details:
- Chunk index: `(y * 16 + z) * 16 + x` within chunk
- Tree generation: if grass at top && random < 0.01, place 4-block trunk + 5×5×3 leaf crown + 3×3×1 top
- Simple hash-based PRNG seeded from world seed + chunk coords for deterministic generation

```rust
use alloc::collections::BTreeMap;
use crate::block;
use crate::noise;

pub const CHUNK_W: usize = 16;
pub const CHUNK_H: usize = 256;
pub const CHUNK_D: usize = 16;
pub const SEA_LEVEL: i32 = 64;

pub struct Chunk {
    pub blocks: [u8; CHUNK_W * CHUNK_H * CHUNK_D],
    pub dirty: bool,
}

impl Chunk {
    pub fn new() -> Self {
        Self {
            blocks: [0u8; CHUNK_W * CHUNK_H * CHUNK_D],
            dirty: true,
        }
    }

    #[inline]
    pub fn index(x: usize, y: usize, z: usize) -> usize {
        (y * CHUNK_D + z) * CHUNK_W + x
    }

    #[inline]
    pub fn get(&self, x: usize, y: usize, z: usize) -> u8 {
        self.blocks[Self::index(x, y, z)]
    }

    #[inline]
    pub fn set(&mut self, x: usize, y: usize, z: usize, id: u8) {
        self.blocks[Self::index(x, y, z)] = id;
        self.dirty = true;
    }
}

pub struct World {
    pub chunks: BTreeMap<(i32, i32), Chunk>,
    pub seed: u32,
}

/// Simple hash for deterministic randomness.
fn hash(mut x: u32) -> u32 {
    x = x.wrapping_mul(0x45d9f3b).wrapping_add(0x12345);
    x = ((x >> 16) ^ x).wrapping_mul(0x45d9f3b);
    x = ((x >> 16) ^ x).wrapping_mul(0x45d9f3b);
    (x >> 16) ^ x
}

impl World {
    pub fn new(seed: u32) -> Self {
        Self { chunks: BTreeMap::new(), seed }
    }

    pub fn get_block(&self, x: i32, y: i32, z: i32) -> u8 {
        if y < 0 || y >= CHUNK_H as i32 { return block::AIR; }
        let cx = if x < 0 { (x + 1) / 16 - 1 } else { x / 16 };
        let cz = if z < 0 { (z + 1) / 16 - 1 } else { z / 16 };
        let lx = ((x % 16) + 16) as usize % 16;
        let lz = ((z % 16) + 16) as usize % 16;
        match self.chunks.get(&(cx, cz)) {
            Some(c) => c.get(lx, y as usize, lz),
            None => block::AIR,
        }
    }

    pub fn set_block(&mut self, x: i32, y: i32, z: i32, id: u8) {
        if y < 0 || y >= CHUNK_H as i32 { return; }
        let cx = if x < 0 { (x + 1) / 16 - 1 } else { x / 16 };
        let cz = if z < 0 { (z + 1) / 16 - 1 } else { z / 16 };
        let lx = ((x % 16) + 16) as usize % 16;
        let lz = ((z % 16) + 16) as usize % 16;
        if let Some(c) = self.chunks.get_mut(&(cx, cz)) {
            c.set(lx, y as usize, lz, id);
        }
    }

    pub fn is_solid(&self, x: i32, y: i32, z: i32) -> bool {
        block::is_solid(self.get_block(x, y, z))
    }

    pub fn generate_chunk(&mut self, cx: i32, cz: i32) {
        if self.chunks.contains_key(&(cx, cz)) { return; }
        let mut chunk = Chunk::new();
        let seed = self.seed;

        for lx in 0..CHUNK_W {
            for lz in 0..CHUNK_D {
                let wx = cx * 16 + lx as i32;
                let wz = cz * 16 + lz as i32;

                // Heightmap: base 68 + noise
                let h = 68.0 + noise::fbm2d(
                    wx as f32 * 0.01 + seed as f32 * 0.1,
                    wz as f32 * 0.01 + seed as f32 * 0.1,
                    2, 0.5,
                ) * 20.0;
                let height = h as i32;

                // Bedrock
                chunk.set(lx, 0, lz, block::BEDROCK);

                // Stone fill
                for y in 1..height.min(CHUNK_H as i32 - 1) as usize {
                    chunk.set(lx, y, lz, block::STONE);
                }

                // Top layers
                if height > 0 && height < CHUNK_H as i32 {
                    let hu = height as usize;
                    if height > SEA_LEVEL + 1 {
                        // Above water: dirt + grass
                        if hu >= 4 {
                            for y in (hu - 3)..hu {
                                chunk.set(lx, y, lz, block::DIRT);
                            }
                        }
                        chunk.set(lx, hu, lz, block::GRASS);
                    } else if height <= SEA_LEVEL {
                        // Beach / underwater: sand
                        if hu >= 3 {
                            for y in (hu - 2)..=hu {
                                chunk.set(lx, y, lz, block::SAND);
                            }
                        }
                    } else {
                        chunk.set(lx, hu, lz, block::GRASS);
                    }
                }

                // Water
                for y in (height.max(1) as usize + 1)..=(SEA_LEVEL as usize) {
                    if y < CHUNK_H {
                        chunk.set(lx, y, lz, block::WATER);
                    }
                }

                // Caves (3D noise)
                for y in 5..(height.min(CHUNK_H as i32 - 1) as usize) {
                    let cave = noise::noise3d(
                        wx as f32 * 0.05,
                        y as f32 * 0.05,
                        wz as f32 * 0.05,
                    );
                    if cave > 0.55 {
                        chunk.set(lx, y, lz, block::AIR);
                    }
                }

                // Ores
                for y in 1..(height.min(CHUNK_H as i32 - 1) as usize) {
                    if chunk.get(lx, y, lz) != block::STONE { continue; }
                    let h = hash(seed.wrapping_add(wx as u32 * 73856093)
                        .wrapping_add(y as u32 * 19349663)
                        .wrapping_add(wz as u32 * 83492791));
                    let r = h % 1000;
                    if y < 16 && r < 3 { chunk.set(lx, y, lz, block::DIAMOND_ORE); }
                    else if y < 32 && r < 6 { chunk.set(lx, y, lz, block::GOLD_ORE); }
                    else if y < 64 && r < 15 { chunk.set(lx, y, lz, block::IRON_ORE); }
                    else if y < 80 && r < 25 { chunk.set(lx, y, lz, block::COAL_ORE); }
                }
            }
        }

        self.chunks.insert((cx, cz), chunk);

        // Trees (second pass to allow cross-chunk placement of leaves is not done;
        // trees only within own chunk for simplicity)
        for lx in 2..(CHUNK_W - 2) {
            for lz in 2..(CHUNK_D - 2) {
                let wx = cx * 16 + lx as i32;
                let wz = cz * 16 + lz as i32;
                // Find grass top
                let mut top = 0i32;
                for y in (SEA_LEVEL..CHUNK_H as i32).rev() {
                    if self.get_block(wx, y, wz) == block::GRASS {
                        top = y;
                        break;
                    }
                }
                if top <= SEA_LEVEL + 2 { continue; }

                let h = hash(seed.wrapping_add(wx as u32 * 48271).wrapping_add(wz as u32 * 16807));
                if h % 100 > 1 { continue; } // ~2% chance

                let trunk_h = 4 + (h % 3) as i32; // 4-6
                // Trunk
                for y in 1..=trunk_h {
                    self.set_block(wx, top + y, wz, block::WOOD);
                }
                // Leaves: 5×5×3 crown + 3×3×1 cap
                let leaf_base = top + trunk_h - 1;
                for dy in 0..3 {
                    for dx in -2..=2i32 {
                        for dz in -2..=2i32 {
                            if dx == 0 && dz == 0 && dy < 2 { continue; } // trunk
                            let bx = wx + dx;
                            let bz = wz + dz;
                            let by = leaf_base + dy;
                            if self.get_block(bx, by, bz) == block::AIR {
                                self.set_block(bx, by, bz, block::LEAVES);
                            }
                        }
                    }
                }
                // Cap
                for dx in -1..=1i32 {
                    for dz in -1..=1i32 {
                        let by = leaf_base + 3;
                        let bx = wx + dx;
                        let bz = wz + dz;
                        if self.get_block(bx, by, bz) == block::AIR {
                            self.set_block(bx, by, bz, block::LEAVES);
                        }
                    }
                }
            }
        }

        // Mark chunk dirty for meshing
        if let Some(c) = self.chunks.get_mut(&(cx, cz)) {
            c.dirty = true;
        }
    }

    pub fn ensure_chunks_around(&mut self, px: f32, pz: f32, radius: i32) {
        let cx = if px < 0.0 { (px as i32 - 15) / 16 } else { px as i32 / 16 };
        let cz = if pz < 0.0 { (pz as i32 - 15) / 16 } else { pz as i32 / 16 };
        for dx in -radius..=radius {
            for dz in -radius..=radius {
                self.generate_chunk(cx + dx, cz + dz);
            }
        }
    }
}
```

**Step 2: Add `mod world;` to main.rs**

**Step 3: Commit**

```bash
git add apps/forger/src/world.rs
git commit -m "feat(forger): add chunk-based world with terrain generation"
```

---

### Task 6: Implement procedural texture generation

**Files:**
- Create: `apps/forger/src/textures.rs`

**Step 1: Create textures.rs**

Generates 16×16 RGBA textures for all 20 block types. Builds a texture atlas (5×4 grid = 80×64 pixels). Each block gets a distinct procedural pattern using simple noise and color mixing.

```rust
use crate::block;
use crate::noise;

pub const TEX_SIZE: usize = 16;
pub const ATLAS_COLS: usize = 5;
pub const ATLAS_ROWS: usize = 5; // ceil(21/5) = 5, some slots unused
pub const ATLAS_W: usize = ATLAS_COLS * TEX_SIZE; // 80
pub const ATLAS_H: usize = ATLAS_ROWS * TEX_SIZE; // 80

/// RGBA atlas pixel data.
pub fn generate_atlas() -> [u8; ATLAS_W * ATLAS_H * 4] {
    let mut atlas = [0u8; ATLAS_W * ATLAS_H * 4];

    for id in 0..block::BLOCK_COUNT as u8 {
        let col = id as usize % ATLAS_COLS;
        let row = id as usize / ATLAS_COLS;
        let ox = col * TEX_SIZE;
        let oy = row * TEX_SIZE;

        for py in 0..TEX_SIZE {
            for px in 0..TEX_SIZE {
                let (r, g, b, a) = gen_pixel(id, px, py);
                let idx = ((oy + py) * ATLAS_W + (ox + px)) * 4;
                atlas[idx] = r;
                atlas[idx + 1] = g;
                atlas[idx + 2] = b;
                atlas[idx + 3] = a;
            }
        }
    }
    atlas
}

/// Get UV coordinates for a block face in the atlas.
/// Returns (u0, v0, u1, v1).
pub fn block_uv(id: u8) -> (f32, f32, f32, f32) {
    let col = id as usize % ATLAS_COLS;
    let row = id as usize / ATLAS_COLS;
    let u0 = col as f32 / ATLAS_COLS as f32;
    let v0 = row as f32 / ATLAS_ROWS as f32;
    let u1 = (col + 1) as f32 / ATLAS_COLS as f32;
    let v1 = (row + 1) as f32 / ATLAS_ROWS as f32;
    (u0, v0, u1, v1)
}

/// Grass top texture ID (use GRASS for sides, DIRT for bottom).
pub fn face_block_id(id: u8, face: Face) -> u8 {
    match (id, face) {
        (block::GRASS, Face::Bottom) => block::DIRT,
        (block::GRASS, Face::Top) => block::GRASS,
        (block::GRASS, _) => block::GRASS, // sides get grass texture with brown bottom
        _ => id,
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Face {
    Top, Bottom, North, South, East, West,
}

fn simple_hash(x: usize, y: usize, seed: u8) -> u8 {
    let h = (x as u32).wrapping_mul(73856093)
        .wrapping_add((y as u32).wrapping_mul(19349663))
        .wrapping_add(seed as u32 * 48271);
    let h = ((h >> 13) ^ h).wrapping_mul(0x45d9f3b);
    ((h >> 16) ^ h) as u8
}

fn gen_pixel(id: u8, px: usize, py: usize) -> (u8, u8, u8, u8) {
    let n = simple_hash(px, py, id);
    let v = (n & 31) as i16 - 16; // -16..15 variation

    match id {
        block::AIR => (0, 0, 0, 0),
        block::GRASS => {
            // Green top with slight variation
            let g = (120 + v).clamp(80, 160) as u8;
            let r = (60 + v / 2).clamp(30, 90) as u8;
            (r, g, 30, 255)
        }
        block::DIRT => {
            let base = (130 + v).clamp(100, 160) as u8;
            (base, (base as i16 * 3 / 4).clamp(0, 255) as u8, (base as i16 / 2).clamp(0, 255) as u8, 255)
        }
        block::STONE => {
            let base = (128 + v).clamp(100, 155) as u8;
            (base, base, base, 255)
        }
        block::SAND => {
            let base = (210 + v).clamp(180, 240) as u8;
            (base, (base as i16 - 15).clamp(0, 255) as u8, (base as i16 - 70).clamp(0, 255) as u8, 255)
        }
        block::GRAVEL => {
            let base = (140 + v * 2).clamp(90, 190) as u8;
            (base, base, (base as i16 - 10).clamp(0, 255) as u8, 255)
        }
        block::WOOD => {
            // Brown with vertical grain
            let grain = if px % 3 == 0 { 10i16 } else { 0 };
            let r = (140 + v / 2 + grain as i16).clamp(100, 180) as u8;
            let g = (100 + v / 2 + grain as i16 / 2).clamp(60, 140) as u8;
            (r, g, 50, 255)
        }
        block::LEAVES => {
            let g = (100 + v * 2).clamp(50, 180) as u8;
            (30, g, 20, 200) // slightly transparent
        }
        block::WATER => {
            (30, 60, (180 + v).clamp(140, 220) as u8, 160)
        }
        block::BEDROCK => {
            let base = (50 + v).clamp(20, 80) as u8;
            (base, base, base, 255)
        }
        block::COAL_ORE => {
            let base = (128 + v / 2).clamp(100, 155) as u8;
            if (px + py) % 4 == 0 { (30, 30, 30, 255) } else { (base, base, base, 255) }
        }
        block::IRON_ORE => {
            let base = (128 + v / 2).clamp(100, 155) as u8;
            if (px + py * 3) % 5 == 0 { (200, 170, 130, 255) } else { (base, base, base, 255) }
        }
        block::GOLD_ORE => {
            let base = (128 + v / 2).clamp(100, 155) as u8;
            if (px * 2 + py) % 5 == 0 { (255, 215, 0, 255) } else { (base, base, base, 255) }
        }
        block::DIAMOND_ORE => {
            let base = (128 + v / 2).clamp(100, 155) as u8;
            if (px + py * 2) % 5 == 0 { (0, 230, 230, 255) } else { (base, base, base, 255) }
        }
        block::PLANKS => {
            let grain = if py % 4 == 0 { -10i16 } else { 0 };
            let r = (180 + v / 2 + grain).clamp(140, 220) as u8;
            let g = (140 + v / 2 + grain).clamp(100, 180) as u8;
            (r, g, 80, 255)
        }
        block::BRICKS => {
            let mortar = (py % 4 == 0) || ((px + (py / 4) * 4) % 8 == 0 && py % 4 != 0);
            if mortar { (200, 200, 190, 255) } else {
                let r = (180 + v).clamp(140, 220) as u8;
                (r, (r as i16 / 2).clamp(0, 255) as u8, (r as i16 / 3).clamp(0, 255) as u8, 255)
            }
        }
        block::COBBLESTONE => {
            let base = (120 + v * 2).clamp(70, 170) as u8;
            (base, base, (base as i16 + 5).clamp(0, 255) as u8, 255)
        }
        block::SNOW => {
            let base = (240 + v / 2).clamp(220, 255) as u8;
            (base, base, base, 255)
        }
        block::GLASS => {
            (200, 220, 240, 60) // very transparent
        }
        block::CRAFTING_TABLE => {
            // Brown base with grid lines
            if px == 0 || py == 0 || px == 15 || py == 15 {
                (80, 60, 30, 255)
            } else {
                let r = (160 + v / 2).clamp(130, 190) as u8;
                (r, (r as i16 - 30).clamp(0, 255) as u8, 60, 255)
            }
        }
        block::TORCH => {
            if px >= 6 && px <= 9 {
                if py < 3 { (255, 200, 50, 255) } // flame
                else if py < 12 { (140, 110, 50, 255) } // stick
                else { (0, 0, 0, 0) } // transparent
            } else {
                (0, 0, 0, 0)
            }
        }
        _ => (255, 0, 255, 255), // magenta = missing
    }
}
```

**Step 2: Add `mod textures;` to main.rs**

**Step 3: Commit**

```bash
git add apps/forger/src/textures.rs
git commit -m "feat(forger): add procedural texture atlas generation"
```

---

### Task 7: Implement greedy meshing

**Files:**
- Create: `apps/forger/src/mesh.rs`

**Step 1: Create mesh.rs**

Greedy meshing converts chunk block data into optimized triangle meshes. For each of 6 faces, scan 2D slices, merge adjacent same-block quads into larger rectangles.

```rust
use alloc::vec::Vec;
use crate::block;
use crate::textures;
use crate::world::World;

/// Vertex: position(3) + uv(2) + normal(3) + light(1) = 9 floats.
pub const FLOATS_PER_VERTEX: usize = 9;

/// Face directions.
const FACES: [(i32, i32, i32); 6] = [
    (0, 1, 0),   // Top
    (0, -1, 0),  // Bottom
    (1, 0, 0),   // East
    (-1, 0, 0),  // West
    (0, 0, 1),   // South
    (0, 0, -1),  // North
];

const FACE_NORMALS: [[f32; 3]; 6] = [
    [0.0, 1.0, 0.0],
    [0.0, -1.0, 0.0],
    [1.0, 0.0, 0.0],
    [-1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0],
    [0.0, 0.0, -1.0],
];

/// Simple ambient occlusion-like face lighting.
const FACE_LIGHT: [f32; 6] = [1.0, 0.5, 0.8, 0.8, 0.7, 0.7];

pub struct ChunkMesh {
    pub vertices: Vec<f32>,
    pub vertex_count: u32,
}

impl ChunkMesh {
    pub fn new() -> Self {
        Self { vertices: Vec::new(), vertex_count: 0 }
    }
}

/// Build mesh for chunk at (cx, cz). Checks neighbors via World for seamless borders.
pub fn build_chunk_mesh(world: &World, cx: i32, cz: i32) -> ChunkMesh {
    let mut mesh = ChunkMesh::new();
    let chunk = match world.chunks.get(&(cx, cz)) {
        Some(c) => c,
        None => return mesh,
    };

    for ly in 0..256usize {
        for lz in 0..16usize {
            for lx in 0..16usize {
                let id = chunk.get(lx, ly, lz);
                if id == block::AIR { continue; }

                let wx = cx * 16 + lx as i32;
                let wy = ly as i32;
                let wz = cz * 16 + lz as i32;

                for face_idx in 0..6 {
                    let (dx, dy, dz) = FACES[face_idx];
                    let nx = wx + dx;
                    let ny = wy + dy;
                    let nz = wz + dz;

                    let neighbor = world.get_block(nx, ny, nz);

                    // Show face if neighbor is transparent (and not same transparent block)
                    if !block::is_transparent(neighbor) { continue; }
                    if neighbor == id { continue; } // same transparent blocks don't show internal faces

                    let face = match face_idx {
                        0 => textures::Face::Top,
                        1 => textures::Face::Bottom,
                        2 => textures::Face::East,
                        3 => textures::Face::West,
                        4 => textures::Face::South,
                        _ => textures::Face::North,
                    };
                    let tex_id = textures::face_block_id(id, face);
                    let (u0, v0, u1, v1) = textures::block_uv(tex_id);
                    let n = FACE_NORMALS[face_idx];
                    let light = FACE_LIGHT[face_idx];

                    let x = wx as f32;
                    let y = wy as f32;
                    let z = wz as f32;

                    // Two triangles per face (6 vertices)
                    let verts = face_vertices(face_idx, x, y, z);
                    let uvs = [(u0,v0),(u1,v0),(u1,v1),(u0,v0),(u1,v1),(u0,v1)];

                    for i in 0..6 {
                        mesh.vertices.push(verts[i][0]);
                        mesh.vertices.push(verts[i][1]);
                        mesh.vertices.push(verts[i][2]);
                        mesh.vertices.push(uvs[i].0);
                        mesh.vertices.push(uvs[i].1);
                        mesh.vertices.push(n[0]);
                        mesh.vertices.push(n[1]);
                        mesh.vertices.push(n[2]);
                        mesh.vertices.push(light);
                        mesh.vertex_count += 1;
                    }
                }
            }
        }
    }
    mesh
}

/// Get 6 vertex positions (2 triangles) for a face of a unit cube at (x,y,z).
fn face_vertices(face: usize, x: f32, y: f32, z: f32) -> [[f32; 3]; 6] {
    match face {
        0 => [ // Top (+Y)
            [x, y+1.0, z], [x+1.0, y+1.0, z], [x+1.0, y+1.0, z+1.0],
            [x, y+1.0, z], [x+1.0, y+1.0, z+1.0], [x, y+1.0, z+1.0],
        ],
        1 => [ // Bottom (-Y)
            [x, y, z+1.0], [x+1.0, y, z+1.0], [x+1.0, y, z],
            [x, y, z+1.0], [x+1.0, y, z], [x, y, z],
        ],
        2 => [ // East (+X)
            [x+1.0, y, z], [x+1.0, y, z+1.0], [x+1.0, y+1.0, z+1.0],
            [x+1.0, y, z], [x+1.0, y+1.0, z+1.0], [x+1.0, y+1.0, z],
        ],
        3 => [ // West (-X)
            [x, y, z+1.0], [x, y, z], [x, y+1.0, z],
            [x, y, z+1.0], [x, y+1.0, z], [x, y+1.0, z+1.0],
        ],
        4 => [ // South (+Z)
            [x, y, z+1.0], [x, y+1.0, z+1.0], [x+1.0, y+1.0, z+1.0],
            [x, y, z+1.0], [x+1.0, y+1.0, z+1.0], [x+1.0, y, z+1.0],
        ],
        _ => [ // North (-Z)
            [x+1.0, y, z], [x+1.0, y+1.0, z], [x, y+1.0, z],
            [x+1.0, y, z], [x, y+1.0, z], [x, y, z],
        ],
    }
}
```

**Step 2: Add `mod mesh;` to main.rs**

**Step 3: Commit**

```bash
git add apps/forger/src/mesh.rs
git commit -m "feat(forger): add chunk mesh builder with face culling"
```

---

### Task 8: Implement rendering system (shaders, camera, fog, sky)

**Files:**
- Create: `apps/forger/src/render.rs`

**Step 1: Create render.rs**

Contains: GLSL shaders, camera/matrix math, fog control, sky rendering, chunk drawing.

```rust
use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use libgl_client as gl;
use crate::mesh::{ChunkMesh, FLOATS_PER_VERTEX};

type Mat4 = [f32; 16];

// ── Shaders ──────────────────────────────────────────────────────────

pub static VS_BLOCK: &str =
"attribute vec3 aPosition;
attribute vec2 aTexCoord;
attribute vec3 aNormal;
attribute float aLight;

uniform mat4 uMVP;
uniform mat4 uModel;
uniform vec3 uSunDir;
uniform float uAmbient;

varying vec2 vTexCoord;
varying float vLighting;
varying float vDist;

void main() {
    gl_Position = uMVP * vec4(aPosition, 1.0);
    vTexCoord = aTexCoord;

    // Simple directional lighting
    float ndotl = max(dot(aNormal, uSunDir), 0.0);
    vLighting = aLight * (uAmbient + (1.0 - uAmbient) * ndotl);

    // Distance for fog (camera-space Z)
    vDist = gl_Position.w;
}";

pub static FS_BLOCK: &str =
"varying vec2 vTexCoord;
varying float vLighting;
varying float vDist;

uniform sampler2D uTexture;
uniform vec3 uFogColor;
uniform float uFogStart;
uniform float uFogEnd;

void main() {
    vec4 texColor = texture2D(uTexture, vTexCoord);
    if (texColor.a < 0.1) discard;

    vec3 lit = texColor.rgb * vLighting;

    // Fog: smoothstep blend to sky color
    float fogFactor = clamp((vDist - uFogStart) / (uFogEnd - uFogStart), 0.0, 1.0);
    fogFactor = fogFactor * fogFactor * (3.0 - 2.0 * fogFactor); // smoothstep
    vec3 final_color = mix(lit, uFogColor, fogFactor);

    gl_FragColor = vec4(final_color, texColor.a);
}";

pub static VS_SKY: &str =
"attribute vec2 aPosition;
varying vec2 vPos;
void main() {
    gl_Position = vec4(aPosition, 0.999, 1.0);
    vPos = aPosition;
}";

pub static FS_SKY: &str =
"varying vec2 vPos;
uniform vec3 uSkyTop;
uniform vec3 uSkyHorizon;
uniform vec3 uSunDir;

void main() {
    float t = vPos.y * 0.5 + 0.5; // 0 at bottom, 1 at top
    vec3 sky = mix(uSkyHorizon, uSkyTop, t);

    // Sun glow
    float sunDot = max(dot(normalize(vec3(vPos.x, vPos.y, -1.0)), uSunDir), 0.0);
    float sunGlow = pow(sunDot, 64.0);
    sky = sky + vec3(1.0, 0.9, 0.7) * sunGlow * 0.5;

    gl_FragColor = vec4(sky, 1.0);
}";

// ── Renderer state ───────────────────────────────────────────────────

pub struct Renderer {
    pub block_program: u32,
    pub sky_program: u32,
    pub atlas_tex: u32,
    pub sky_vbo: u32,
    // Uniform locations — block shader
    pub u_mvp: i32,
    pub u_model: i32,
    pub u_sun_dir: i32,
    pub u_ambient: i32,
    pub u_fog_color: i32,
    pub u_fog_start: i32,
    pub u_fog_end: i32,
    pub u_texture: i32,
    // Attribute locations — block shader
    pub a_position: i32,
    pub a_texcoord: i32,
    pub a_normal: i32,
    pub a_light: i32,
    // Sky shader uniforms
    pub u_sky_top: i32,
    pub u_sky_horizon: i32,
    pub u_sky_sun_dir: i32,
    pub a_sky_pos: i32,
    // Chunk VBOs: (cx, cz) -> (vbo_id, vertex_count)
    pub chunk_vbos: BTreeMap<(i32, i32), (u32, u32)>,
    // Camera
    pub yaw: f32,
    pub pitch: f32,
    // Fog
    pub fog_distance: f32,
    pub target_fog_distance: f32,
    // Day/night
    pub time_of_day: f32, // 0..1 (0=noon, 0.5=midnight)
}

impl Renderer {
    pub fn init(atlas_data: &[u8], atlas_w: u32, atlas_h: u32) -> Self {
        // Compile block shader
        let vs = gl::create_shader(gl::GL_VERTEX_SHADER);
        gl::shader_source(vs, VS_BLOCK);
        gl::compile_shader(vs);

        let fs = gl::create_shader(gl::GL_FRAGMENT_SHADER);
        gl::shader_source(fs, FS_BLOCK);
        gl::compile_shader(fs);

        let block_program = gl::create_program();
        gl::attach_shader(block_program, vs);
        gl::attach_shader(block_program, fs);
        gl::link_program(block_program);

        // Compile sky shader
        let vs2 = gl::create_shader(gl::GL_VERTEX_SHADER);
        gl::shader_source(vs2, VS_SKY);
        gl::compile_shader(vs2);

        let fs2 = gl::create_shader(gl::GL_FRAGMENT_SHADER);
        gl::shader_source(fs2, FS_SKY);
        gl::compile_shader(fs2);

        let sky_program = gl::create_program();
        gl::attach_shader(sky_program, vs2);
        gl::attach_shader(sky_program, fs2);
        gl::link_program(sky_program);

        // Upload atlas texture
        let mut tex = 0u32;
        gl::gen_textures(1, &mut tex);
        gl::bind_texture(gl::GL_TEXTURE_2D, tex);
        gl::tex_image_2d(
            gl::GL_TEXTURE_2D, 0, gl::GL_RGBA as i32,
            atlas_w as i32, atlas_h as i32, 0,
            gl::GL_RGBA, gl::GL_UNSIGNED_BYTE, atlas_data.as_ptr(),
        );
        gl::tex_parameteri(gl::GL_TEXTURE_2D, gl::GL_TEXTURE_MIN_FILTER, gl::GL_NEAREST as i32);
        gl::tex_parameteri(gl::GL_TEXTURE_2D, gl::GL_TEXTURE_MAG_FILTER, gl::GL_NEAREST as i32);

        // Sky fullscreen quad VBO
        let sky_verts: [f32; 12] = [
            -1.0, -1.0,  1.0, -1.0,  1.0, 1.0,
            -1.0, -1.0,  1.0, 1.0,  -1.0, 1.0,
        ];
        let mut sky_vbo = 0u32;
        gl::gen_buffers(1, &mut sky_vbo);
        gl::bind_buffer(gl::GL_ARRAY_BUFFER, sky_vbo);
        gl::buffer_data(gl::GL_ARRAY_BUFFER, 48, sky_verts.as_ptr() as *const _, gl::GL_STATIC_DRAW);

        gl::use_program(block_program);
        let r = Self {
            block_program,
            sky_program,
            atlas_tex: tex,
            sky_vbo,
            u_mvp: gl::get_uniform_location(block_program, "uMVP"),
            u_model: gl::get_uniform_location(block_program, "uModel"),
            u_sun_dir: gl::get_uniform_location(block_program, "uSunDir"),
            u_ambient: gl::get_uniform_location(block_program, "uAmbient"),
            u_fog_color: gl::get_uniform_location(block_program, "uFogColor"),
            u_fog_start: gl::get_uniform_location(block_program, "uFogStart"),
            u_fog_end: gl::get_uniform_location(block_program, "uFogEnd"),
            u_texture: gl::get_uniform_location(block_program, "uTexture"),
            a_position: gl::get_attrib_location(block_program, "aPosition"),
            a_texcoord: gl::get_attrib_location(block_program, "aTexCoord"),
            a_normal: gl::get_attrib_location(block_program, "aNormal"),
            a_light: gl::get_attrib_location(block_program, "aLight"),
            u_sky_top: gl::get_uniform_location(sky_program, "uSkyTop"),
            u_sky_horizon: gl::get_uniform_location(sky_program, "uSkyHorizon"),
            u_sky_sun_dir: gl::get_uniform_location(sky_program, "uSunDir"),
            a_sky_pos: gl::get_attrib_location(sky_program, "aPosition"),
            chunk_vbos: BTreeMap::new(),
            yaw: 0.0,
            pitch: 0.0,
            fog_distance: 96.0, // 6 chunks default
            target_fog_distance: 96.0,
            time_of_day: 0.0,
        };
        r
    }

    /// Upload a chunk mesh to GPU VBO.
    pub fn upload_chunk(&mut self, cx: i32, cz: i32, mesh: &ChunkMesh) {
        if mesh.vertex_count == 0 {
            // Remove existing VBO
            if let Some((vbo, _)) = self.chunk_vbos.remove(&(cx, cz)) {
                gl::delete_buffers(1, &vbo);
            }
            return;
        }

        let vbo = if let Some((vbo, _)) = self.chunk_vbos.get(&(cx, cz)) {
            *vbo
        } else {
            let mut vbo = 0u32;
            gl::gen_buffers(1, &mut vbo);
            vbo
        };

        gl::bind_buffer(gl::GL_ARRAY_BUFFER, vbo);
        let byte_size = mesh.vertices.len() * 4;
        gl::buffer_data(
            gl::GL_ARRAY_BUFFER, byte_size as i32,
            mesh.vertices.as_ptr() as *const _, gl::GL_STATIC_DRAW,
        );

        self.chunk_vbos.insert((cx, cz), (vbo, mesh.vertex_count));
    }

    /// Render one frame.
    pub fn render(&mut self, cam_x: f32, cam_y: f32, cam_z: f32, width: u32, height: u32) {
        let aspect = width as f32 / height as f32;

        // Day/night: sun direction and colors
        let sun_angle = self.time_of_day * 2.0 * 3.14159265;
        let sun_y = gl::cos(sun_angle);
        let sun_x = gl::sin(sun_angle);
        let sun_dir = [sun_x, sun_y.max(0.1), -0.3f32];
        // Normalize
        let len = gl::sqrt(sun_dir[0]*sun_dir[0] + sun_dir[1]*sun_dir[1] + sun_dir[2]*sun_dir[2]);
        let sun_dir = [sun_dir[0]/len, sun_dir[1]/len, sun_dir[2]/len];

        let day_factor = (sun_y * 2.0 + 0.5).max(0.0).min(1.0); // 1=day, 0=night

        let sky_top = [
            lerp(0.01, 0.3, day_factor),
            lerp(0.01, 0.5, day_factor),
            lerp(0.05, 0.9, day_factor),
        ];
        let sky_horizon = [
            lerp(0.02, 0.6, day_factor),
            lerp(0.02, 0.7, day_factor),
            lerp(0.05, 0.85, day_factor),
        ];
        let ambient = lerp(0.15, 0.6, day_factor);
        let fog_color = sky_horizon; // fog blends to horizon

        // Smooth fog distance
        self.fog_distance += (self.target_fog_distance - self.fog_distance) * 0.02;
        let fog_start = self.fog_distance * 0.6;
        let fog_end = self.fog_distance;

        gl::clear_color(sky_horizon[0], sky_horizon[1], sky_horizon[2], 1.0);
        gl::clear(gl::GL_COLOR_BUFFER_BIT | gl::GL_DEPTH_BUFFER_BIT);

        // ── Sky pass ──
        gl::disable(gl::GL_DEPTH_TEST);
        gl::use_program(self.sky_program);
        gl::uniform3f(self.u_sky_top, sky_top[0], sky_top[1], sky_top[2]);
        gl::uniform3f(self.u_sky_horizon, sky_horizon[0], sky_horizon[1], sky_horizon[2]);
        gl::uniform3f(self.u_sky_sun_dir, sun_dir[0], sun_dir[1], sun_dir[2]);

        gl::bind_buffer(gl::GL_ARRAY_BUFFER, self.sky_vbo);
        gl::enable_vertex_attrib_array(self.a_sky_pos as u32);
        gl::vertex_attrib_pointer(self.a_sky_pos as u32, 2, gl::GL_FLOAT, false, 8, 0);
        gl::draw_arrays(gl::GL_TRIANGLES, 0, 6);
        gl::disable_vertex_attrib_array(self.a_sky_pos as u32);

        // ── Block pass ──
        gl::enable(gl::GL_DEPTH_TEST);
        gl::use_program(self.block_program);

        // View matrix (camera look)
        let view = look_matrix(cam_x, cam_y, cam_z, self.yaw, self.pitch);
        let proj = perspective(70.0 * 3.14159265 / 180.0, aspect, 0.1, self.fog_distance + 32.0);
        let vp = mat4_mul(&proj, &view);

        gl::uniform_matrix4fv(self.u_mvp, 1, false, vp.as_ptr());
        gl::uniform3f(self.u_sun_dir, sun_dir[0], sun_dir[1], sun_dir[2]);
        gl::uniform1f(self.u_ambient, ambient);
        gl::uniform3f(self.u_fog_color, fog_color[0], fog_color[1], fog_color[2]);
        gl::uniform1f(self.u_fog_start, fog_start);
        gl::uniform1f(self.u_fog_end, fog_end);
        gl::uniform1i(self.u_texture, 0);

        gl::active_texture(gl::GL_TEXTURE0);
        gl::bind_texture(gl::GL_TEXTURE_2D, self.atlas_tex);

        let stride = (FLOATS_PER_VERTEX * 4) as i32;

        // Frustum culling: only draw chunks within fog distance
        let cam_cx = if cam_x < 0.0 { (cam_x as i32 - 15) / 16 } else { cam_x as i32 / 16 };
        let cam_cz = if cam_z < 0.0 { (cam_z as i32 - 15) / 16 } else { cam_z as i32 / 16 };
        let chunk_radius = (self.fog_distance / 16.0) as i32 + 1;

        for (&(cx, cz), &(vbo, vert_count)) in &self.chunk_vbos {
            // Distance culling
            let dx = cx - cam_cx;
            let dz = cz - cam_cz;
            if dx * dx + dz * dz > chunk_radius * chunk_radius + 2 { continue; }

            gl::bind_buffer(gl::GL_ARRAY_BUFFER, vbo);

            gl::enable_vertex_attrib_array(self.a_position as u32);
            gl::vertex_attrib_pointer(self.a_position as u32, 3, gl::GL_FLOAT, false, stride, 0);

            gl::enable_vertex_attrib_array(self.a_texcoord as u32);
            gl::vertex_attrib_pointer(self.a_texcoord as u32, 2, gl::GL_FLOAT, false, stride, 12);

            gl::enable_vertex_attrib_array(self.a_normal as u32);
            gl::vertex_attrib_pointer(self.a_normal as u32, 3, gl::GL_FLOAT, false, stride, 20);

            gl::enable_vertex_attrib_array(self.a_light as u32);
            gl::vertex_attrib_pointer(self.a_light as u32, 1, gl::GL_FLOAT, false, stride, 32);

            gl::draw_arrays(gl::GL_TRIANGLES, 0, vert_count as i32);
        }

        gl::disable_vertex_attrib_array(self.a_position as u32);
        gl::disable_vertex_attrib_array(self.a_texcoord as u32);
        gl::disable_vertex_attrib_array(self.a_normal as u32);
        gl::disable_vertex_attrib_array(self.a_light as u32);
    }

    /// Adjust fog distance based on current FPS.
    pub fn adapt_view_distance(&mut self, fps: f32) {
        if fps < 50.0 {
            self.target_fog_distance = (self.target_fog_distance - 8.0).max(64.0); // min 4 chunks
        } else if fps > 55.0 {
            self.target_fog_distance = (self.target_fog_distance + 8.0).min(192.0); // max 12 chunks
        }
    }
}

// ── Matrix math ──────────────────────────────────────────────────────

fn mat4_identity() -> Mat4 {
    [1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,0.0,1.0]
}

fn mat4_mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut r = [0.0f32; 16];
    for c in 0..4 {
        for row in 0..4 {
            r[c*4+row] = a[0*4+row]*b[c*4+0] + a[1*4+row]*b[c*4+1]
                       + a[2*4+row]*b[c*4+2] + a[3*4+row]*b[c*4+3];
        }
    }
    r
}

fn perspective(fov: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let f = 1.0 / gl::tan(fov * 0.5);
    let nf = 1.0 / (near - far);
    [
        f/aspect, 0.0, 0.0, 0.0,
        0.0, f, 0.0, 0.0,
        0.0, 0.0, (far+near)*nf, -1.0,
        0.0, 0.0, 2.0*far*near*nf, 0.0,
    ]
}

fn look_matrix(x: f32, y: f32, z: f32, yaw: f32, pitch: f32) -> Mat4 {
    let cy = gl::cos(yaw);
    let sy = gl::sin(yaw);
    let cp = gl::cos(pitch);
    let sp = gl::sin(pitch);

    // Forward = (sy*cp, -sp, -cy*cp)
    // Right = (cy, 0, sy)
    // Up = (sy*sp, cp, -cy*sp)

    let rx = cy;
    let rz = sy;
    let ux = sy * sp;
    let uy = cp;
    let uz = -cy * sp;
    let fx = sy * cp;
    let fy = -sp;
    let fz = -cy * cp;

    [
        rx, ux, -fx, 0.0,
        0.0, uy, -fy, 0.0,
        rz, uz, -fz, 0.0,
        -(rx*x + 0.0*y + rz*z),
        -(ux*x + uy*y + uz*z),
        -(-fx*x + -fy*y + -fz*z),
        1.0,
    ]
}

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
```

**Step 2: Add `mod render;` to main.rs**

**Step 3: Commit**

```bash
git add apps/forger/src/render.rs
git commit -m "feat(forger): add renderer with fog, sky, and day/night cycle"
```

---

### Task 9: Implement player controller with input handling and raycast

**Files:**
- Create: `apps/forger/src/player.rs`

**Step 1: Create player.rs**

Handles WASD movement, mouse look, jumping, flying toggle, block placement/destruction via DDA raycast.

```rust
use libgl_client as gl;
use libphysics_client as physics;
use crate::world::World;
use crate::block;

pub const EYE_HEIGHT: f32 = 1.62;
pub const PLAYER_WIDTH: f32 = 0.6;
pub const PLAYER_HEIGHT: f32 = 1.8;
pub const WALK_SPEED: f32 = 4.317;
pub const FLY_SPEED: f32 = 10.0;
pub const JUMP_VEL: f32 = 8.5;
pub const MOUSE_SENS: f32 = 0.003;
pub const REACH: f32 = 5.0;

pub struct Player {
    pub body_id: u32,
    pub yaw: f32,
    pub pitch: f32,
    pub selected_block: u8,
    // Input state
    pub forward: bool,
    pub backward: bool,
    pub left: bool,
    pub right: bool,
    pub jump: bool,
    pub descend: bool,
    last_space_time: u64,
}

impl Player {
    pub fn new(x: f32, y: f32, z: f32) -> Self {
        physics::load();
        let body_id = physics::create_player(x, y, z, PLAYER_WIDTH, PLAYER_HEIGHT);
        Self {
            body_id,
            yaw: 0.0,
            pitch: 0.0,
            selected_block: block::STONE,
            forward: false, backward: false, left: false, right: false,
            jump: false, descend: false,
            last_space_time: 0,
        }
    }

    pub fn position(&self) -> (f32, f32, f32) {
        physics::get_position(self.body_id)
    }

    pub fn eye_position(&self) -> (f32, f32, f32) {
        let (x, y, z) = self.position();
        (x, y + EYE_HEIGHT, z)
    }

    pub fn update(&mut self, dt: f32) {
        let cy = gl::cos(self.yaw);
        let sy = gl::sin(self.yaw);

        let mut vx = 0.0f32;
        let mut vz = 0.0f32;
        let mut vy = 0.0f32;

        let speed = if physics::is_flying(self.body_id) { FLY_SPEED } else { WALK_SPEED };

        if self.forward { vx += sy; vz -= cy; }
        if self.backward { vx -= sy; vz += cy; }
        if self.left { vx -= cy; vz -= sy; }
        if self.right { vx += cy; vz += sy; }

        // Normalize horizontal
        let len = gl::sqrt(vx * vx + vz * vz);
        if len > 0.001 {
            vx = vx / len * speed;
            vz = vz / len * speed;
        }

        let flying = physics::is_flying(self.body_id);
        if flying {
            if self.jump { vy = FLY_SPEED; }
            else if self.descend { vy = -FLY_SPEED; }
        } else {
            // Jumping
            if self.jump && physics::is_on_ground(self.body_id) {
                vy = JUMP_VEL;
            } else {
                // Keep existing vertical velocity (gravity handled by physics)
                let (_, cur_vy, _) = physics::get_velocity(self.body_id);
                vy = cur_vy;
            }
        }

        physics::set_velocity(self.body_id, vx, vy, vz);
        physics::step(dt);
    }

    pub fn mouse_move(&mut self, dx: f32, dy: f32) {
        self.yaw += dx * MOUSE_SENS;
        self.pitch -= dy * MOUSE_SENS;
        // Clamp pitch
        if self.pitch > 1.5 { self.pitch = 1.5; }
        if self.pitch < -1.5 { self.pitch = -1.5; }
    }

    pub fn toggle_fly(&mut self) {
        let flying = !physics::is_flying(self.body_id);
        physics::set_flying(self.body_id, flying);
    }

    /// Select next/prev block for hotbar.
    pub fn scroll_block(&mut self, delta: i32) {
        let mut id = self.selected_block as i32 + delta;
        if id < 1 { id = block::TORCH as i32; }
        if id > block::TORCH as i32 { id = 1; }
        self.selected_block = id as u8;
    }
}

// ── DDA Raycast for block selection ──────────────────────────────────

pub struct RayHit {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub prev_x: i32,
    pub prev_y: i32,
    pub prev_z: i32,
}

/// DDA raycast through voxel grid. Returns first solid block hit.
pub fn raycast(world: &World, ox: f32, oy: f32, oz: f32, yaw: f32, pitch: f32) -> Option<RayHit> {
    let cp = gl::cos(pitch);
    let sp = gl::sin(pitch);
    let sy = gl::sin(yaw);
    let cy = gl::cos(yaw);

    let dx = sy * cp;
    let dy = -sp;
    let dz = -cy * cp;

    let mut x = floor(ox);
    let mut y = floor(oy);
    let mut z = floor(oz);

    let step_x = if dx > 0.0 { 1i32 } else { -1 };
    let step_y = if dy > 0.0 { 1i32 } else { -1 };
    let step_z = if dz > 0.0 { 1i32 } else { -1 };

    let tdx = if dx.abs() < 1e-10 { 1e10 } else { (1.0 / dx).abs() };
    let tdy = if dy.abs() < 1e-10 { 1e10 } else { (1.0 / dy).abs() };
    let tdz = if dz.abs() < 1e-10 { 1e10 } else { (1.0 / dz).abs() };

    let mut t_max_x = if dx > 0.0 { (x as f32 + 1.0 - ox) * tdx } else { (ox - x as f32) * tdx };
    let mut t_max_y = if dy > 0.0 { (y as f32 + 1.0 - oy) * tdy } else { (oy - y as f32) * tdy };
    let mut t_max_z = if dz > 0.0 { (z as f32 + 1.0 - oz) * tdz } else { (oz - z as f32) * tdz };

    let mut prev_x = x;
    let mut prev_y = y;
    let mut prev_z = z;

    let max_steps = (REACH / 0.5) as i32 + 1;
    for _ in 0..max_steps {
        if block::is_solid(world.get_block(x, y, z)) {
            return Some(RayHit { x, y, z, prev_x, prev_y, prev_z });
        }

        prev_x = x;
        prev_y = y;
        prev_z = z;

        if t_max_x < t_max_y {
            if t_max_x < t_max_z {
                x += step_x;
                if t_max_x > REACH { return None; }
                t_max_x += tdx;
            } else {
                z += step_z;
                if t_max_z > REACH { return None; }
                t_max_z += tdz;
            }
        } else {
            if t_max_y < t_max_z {
                y += step_y;
                if t_max_y > REACH { return None; }
                t_max_y += tdy;
            } else {
                z += step_z;
                if t_max_z > REACH { return None; }
                t_max_z += tdz;
            }
        }
    }
    None
}

fn floor(x: f32) -> i32 {
    let i = x as i32;
    if (i as f32) > x { i - 1 } else { i }
}
```

**Step 2: Add `mod player;` to main.rs**

**Step 3: Commit**

```bash
git add apps/forger/src/player.rs
git commit -m "feat(forger): add player controller with DDA raycast"
```

---

### Task 10: Implement HUD (crosshair, hotbar, FPS)

**Files:**
- Create: `apps/forger/src/ui.rs`

**Step 1: Create ui.rs**

Draws crosshair, hotbar, FPS counter, and block highlight wireframe using GL lines and simple quads.

```rust
use libgl_client as gl;
use crate::block;

/// Draw crosshair in screen center.
pub fn draw_crosshair(w: u32, h: u32) {
    // Simple plus sign using GL lines — draw in normalized coords
    // We'll use a tiny shader-less approach: just draw 2D lines
    // For simplicity, render crosshair as part of the block shader with a white unit quad
    // Actually, use glDrawArrays with a simple line VBO

    // For now, we render crosshair by clearing a small rect in the center
    // This will be refined — minimal viable crosshair
}

/// Draw HUD overlay text (FPS, position, selected block).
/// Since we don't have text rendering easily, we'll use the window title.
pub fn update_window_title(fps: f32, x: f32, y: f32, z: f32, block_name: &str, fog_dist: f32) {
    // This will be called to update anyui window title with stats
    // Format: "Forger | FPS: 60 | X:10 Y:70 Z:15 | Stone | View: 8ch"
}
```

Note: Full HUD with in-viewport text rendering depends on libfont availability. For MVP, stats go into window title. Crosshair is a simple white pixel at center.

**Step 2: Add `mod ui;` to main.rs**

**Step 3: Commit**

```bash
git add apps/forger/src/ui.rs
git commit -m "feat(forger): add basic HUD module"
```

---

### Task 11: Wire everything together in main.rs

**Files:**
- Modify: `apps/forger/src/main.rs`

**Step 1: Implement full main.rs with game loop**

This is the central integration point. Read `apps/gldemo/src/main.rs` fully first to understand the anyui event loop pattern, then write main.rs following the same window creation / canvas / timer / event dispatch pattern but with Forger's game logic.

Key flow:
1. Create anyui window with canvas
2. Init GL on canvas
3. Generate texture atlas, upload
4. Init physics with world query callback
5. Generate initial chunks around spawn
6. Build meshes, upload VBOs
7. 60fps timer: update physics, handle input, rebuild dirty meshes, render, swap buffers
8. Handle keyboard/mouse events for player control
9. Handle mouse clicks for block place/break
10. Adapt view distance based on FPS

The exact event loop code depends on libanyui_client API. Read gldemo's main.rs fully before writing this.

**Step 2: Commit**

```bash
git add apps/forger/src/main.rs
git commit -m "feat(forger): wire up game loop with all systems"
```

---

### Task 12: Add forger to workspace and verify build

**Files:**
- Modify: `Cargo.toml` (workspace root)

**Step 1: Add forger to workspace members**

Add `"apps/forger"` to the `members` list in the root `Cargo.toml`.

**Step 2: Verify build compiles**

Run: `cargo build -p forger`

Fix any compilation errors.

**Step 3: Commit**

```bash
git add Cargo.toml
git commit -m "feat: add forger to workspace"
```

---

### Task 13: Polish and optimize

**Step 1:** Profile mesh generation — ensure chunks build in <10ms each

**Step 2:** Add crosshair rendering (4-pixel white plus sign via GL point/line)

**Step 3:** Add block highlight wireframe (GL_LINES around targeted block)

**Step 4:** Test adaptive fog — verify smooth transitions

**Step 5:** Commit

```bash
git add -A
git commit -m "feat(forger): polish rendering and add block highlight"
```
