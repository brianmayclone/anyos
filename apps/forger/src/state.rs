use alloc::string::String;
use alloc::vec::Vec;

use crate::inventory::Inventory;
use crate::menu::MenuUi;
use crate::player::Player;
use crate::render::Renderer;
use crate::save::{PlayerSnapshot, WorldSnapshot, WorldSummary};
use crate::settings::GameSettings;
use crate::world::World;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    MainMenu,
    WorldSelect,
    Settings,
    InGame,
}

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
    pub menu_ui: MenuUi,
    pub app_mode: AppMode,
    pub settings: GameSettings,
    pub world_summaries: Vec<WorldSummary>,
    pub current_world_id: String,
    pub current_world_name: String,
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
    pub sun_debug_label: libanyui_client::Label,
    pub upscale_buffer: Vec<u32>,
    pub last_mouse_x: i32,
    pub last_mouse_y: i32,
    pub mouse_captured: bool,
    pub fullscreen: bool,
    pub shadows_enabled: bool,
    pub mining_active: bool,
    pub mining_target: Option<MiningTarget>,
    pub mining_progress: f32,
    pub place_key_down: bool,
    pub autosave_at_ms: u32,
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

pub fn apply_settings(s: &mut GameState) {
    s.render_divisor = s.settings.render_divisor();
    s.fb_w = (s.canvas_w / s.render_divisor).max(1);
    s.fb_h = (s.canvas_h / s.render_divisor).max(1);
    s.shadows_enabled = s.settings.shadows_enabled;
    s.shadow_toggle
        .set_state(if s.settings.shadows_enabled { 1 } else { 0 });
    s.renderer.set_view_quality(s.settings.fog_distance());
    s.renderer.set_shadow_quality(
        s.settings.shadow_strength(),
        s.settings.shadow_softness_scale(),
    );
    libgl_client::gl_resize(s.fb_w, s.fb_h);
    libgl_client::viewport(0, 0, s.fb_w as i32, s.fb_h as i32);
}

pub fn load_runtime_world(s: &mut GameState, snapshot: WorldSnapshot) {
    let mut world = World::new(snapshot.summary.seed);
    world.modifications = snapshot.modifications;
    let spawn_x = snapshot.player.x as i32;
    let spawn_z = snapshot.player.z as i32;
    world.ensure_chunks_around(spawn_x, spawn_z, 2);

    s.world = world;
    s.renderer.clear_chunks();
    let keys: Vec<(i32, i32)> = s.world.chunks.keys().copied().collect();
    for (cx, cz) in keys {
        let mesh = crate::mesh::build_chunk_mesh(&s.world, cx, cz);
        s.renderer.upload_chunk(cx, cz, &mesh);
        if let Some(chunk) = s.world.chunks.get_mut(&(cx, cz)) {
            chunk.dirty = false;
        }
    }

    s.inventory.restore_snapshot(
        snapshot.inventory_counts,
        snapshot.inventory_hotbar,
        snapshot.inventory_selected_slot,
    );

    s.player = Player::new(snapshot.player.x, snapshot.player.y, snapshot.player.z);
    s.player.yaw = snapshot.player.yaw;
    s.player.pitch = snapshot.player.pitch;
    s.player.set_flying(snapshot.player.flying);
    s.mode_toggle
        .set_state(if snapshot.player.flying { 1 } else { 0 });
    crate::game::sync_selected_block(&s.inventory, &mut s.player);

    s.current_world_id = snapshot.summary.id;
    s.current_world_name = snapshot.summary.name;
    s.autosave_at_ms = anyos_std::sys::uptime_ms().wrapping_add(4000);
}

pub fn current_player_snapshot(s: &GameState) -> Option<PlayerSnapshot> {
    if s.player.body_id == u32::MAX {
        return None;
    }
    let (x, y, z) = s.player.position();
    Some(PlayerSnapshot {
        x,
        y,
        z,
        yaw: s.player.yaw,
        pitch: s.player.pitch,
        flying: s.player.is_flying(),
    })
}
