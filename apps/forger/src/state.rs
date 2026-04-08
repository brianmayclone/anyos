use alloc::vec::Vec;

use crate::inventory::Inventory;
use crate::player::Player;
use crate::render::Renderer;
use crate::world::World;

#[derive(Clone, Copy)]
pub struct MiningTarget {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub block_id: u8,
}

pub struct GameState {
    pub canvas: libanyui_client::Canvas,
    pub window: libanyui_client::Window,
    pub mode_toggle: libanyui_client::Toggle,
    pub shadow_toggle: libanyui_client::Toggle,
    pub canvas_w: u32,
    pub canvas_h: u32,
    pub fb_w: u32,
    pub fb_h: u32,
    pub render_divisor: u32,
    pub world: World,
    pub renderer: Renderer,
    pub player: Player,
    pub inventory: Inventory,
    pub fps_frame_count: u32,
    pub fps_last_ms: u32,
    pub fps_display: u32,
    pub fps_label: libanyui_client::Label,
    pub upscale_buffer: Vec<u32>,
    pub last_mouse_x: i32,
    pub last_mouse_y: i32,
    pub mouse_captured: bool,
    pub fullscreen: bool,
    pub shadows_enabled: bool,
    pub mining_active: bool,
    pub mining_target: Option<MiningTarget>,
    pub mining_progress: f32,
}

pub static mut STATE: Option<GameState> = None;

pub extern "C" fn world_query(x: i32, y: i32, z: i32) -> bool {
    unsafe { STATE.as_ref().map_or(false, |s| s.world.is_solid(x, y, z)) }
}

pub fn find_spawn_height(world: &World) -> f32 {
    for y in (1..200).rev() {
        if world.is_solid(0, y, 0) {
            return y as f32 + 2.0;
        }
    }
    80.0
}
