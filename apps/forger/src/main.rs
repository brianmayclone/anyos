#![no_std]
#![no_main]
#![allow(unused, dead_code, static_mut_refs)]

anyos_std::entry!(main);

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use libgl_client as gl;
use libphysics_client as physics;

mod block;
mod game;
mod inventory;
mod menu;
mod mesh;
mod noise;
mod player;
mod render;
mod save;
mod settings;
mod state;
mod textures;
mod ui;
mod world;

use inventory::Inventory;
use player::Player;
use render::Renderer;
use settings::GameSettings;
use state::{find_spawn_height, world_query, AppMode, GameState, STATE};
use world::World;

fn capture_mouse(s: &mut GameState, x: i32, y: i32) {
    s.mouse_captured = true;
    s.last_mouse_x = x;
    s.last_mouse_y = y;
    s.window.set_cursor_visible(false);
    gl::set_cursor_captured(true);
}

fn release_mouse(s: &mut GameState) {
    s.mouse_captured = false;
    s.window.set_cursor_visible(true);
    gl::set_cursor_captured(false);
}

fn place_targeted_block(s: &mut GameState) {
    let (ex, ey, ez) = s.player.eye_position();
    if let Some(hit) = player::raycast(&s.world, ex, ey, ez, s.player.yaw, s.player.pitch) {
        if let Some(block_id) = s.inventory.selected_block() {
            if s.world.get_block(hit.prev_x, hit.prev_y, hit.prev_z) == block::AIR
                && s.inventory.consume_selected()
            {
                s.world
                    .set_block(hit.prev_x, hit.prev_y, hit.prev_z, block_id);
                game::sync_selected_block(&s.inventory, &mut s.player);
            }
        }
    }
}

fn save_if_needed(s: &GameState) {
    if s.app_mode != AppMode::InGame || s.current_world_id.is_empty() {
        return;
    }
    if let Some(player) = state::current_player_snapshot(s) {
        let _ = save::save_runtime_world(
            &s.current_world_id,
            &s.current_world_name,
            &s.world,
            &player,
            &s.inventory,
        );
    }
}

fn main() {
    if !libanyui_client::init() {
        anyos_std::println!("forger: FATAL - failed to load libanyui.so");
        return;
    }
    anyos_std::i18n::init();
    anyos_std::println!("forger: anyui initialized");

    let window = libanyui_client::Window::new("Forger", 50, 50, 800, 600);
    let canvas = libanyui_client::Canvas::new(800, 600);
    canvas.set_dock(libanyui_client::DOCK_FILL);
    canvas.set_interactive(true);
    window.add(&canvas);
    window.set_visible(true);

    let settings = GameSettings::load();

    let canvas_w = canvas.get_stride();
    let canvas_h = canvas.get_height();
    let render_divisor = settings.render_divisor();
    let fb_w = (canvas_w / render_divisor).max(1);
    let fb_h = (canvas_h / render_divisor).max(1);

    if !gl::init() {
        anyos_std::println!("forger: FATAL - failed to load libgl.so");
        return;
    }
    anyos_std::println!(
        "forger: libgl loaded, canvas={}x{} render={}x{}",
        canvas_w,
        canvas_h,
        fb_w,
        fb_h
    );
    gl::gl_init(fb_w, fb_h);
    gl::enable(gl::GL_DEPTH_TEST);
    gl::depth_func(gl::GL_LESS);
    gl::enable(gl::GL_CULL_FACE);
    gl::cull_face(gl::GL_BACK);

    if !physics::init() {
        anyos_std::println!("forger: FATAL - failed to load libphysics.so");
        return;
    }
    anyos_std::println!("forger: libphysics loaded");

    let atlas_data = textures::generate_atlas();
    let mut renderer = Renderer::init(
        &atlas_data,
        textures::ATLAS_W as u32,
        textures::ATLAS_H as u32,
    );

    let mut preview_world = World::new(42);
    anyos_std::println!("forger: generating preview chunks...");
    preview_world.ensure_chunks_around(0, 0, 2);

    let keys: Vec<(i32, i32)> = preview_world.chunks.keys().copied().collect();
    let mut total_verts: u32 = 0;
    for (cx, cz) in &keys {
        let m = mesh::build_chunk_mesh(&preview_world, *cx, *cz);
        let vc = m.vertex_count;
        total_verts += vc;
        renderer.upload_chunk(*cx, *cz, &m);
        if let Some(chunk) = preview_world.chunks.get_mut(&(*cx, *cz)) {
            chunk.dirty = false;
        }
    }

    let mode_label = libanyui_client::Label::new("Fly");
    mode_label.set_position(6, 6);
    mode_label.set_text_color(0xFFFFFFFF);
    mode_label.set_font_size(13);
    window.add(&mode_label);

    let mode_toggle = libanyui_client::Toggle::new(false);
    mode_toggle.set_position(30, 4);
    mode_toggle.on_checked_changed(|e| {
        let s = unsafe { STATE.as_mut().unwrap() };
        if s.player.body_id != u32::MAX {
            s.player.set_flying(e.checked);
        }
    });
    window.add(&mode_toggle);

    let shadow_label = libanyui_client::Label::new("Shadow");
    shadow_label.set_position(62, 6);
    shadow_label.set_text_color(0xFFFFFFFF);
    shadow_label.set_font_size(13);
    window.add(&shadow_label);

    let shadow_toggle = libanyui_client::Toggle::new(settings.shadows_enabled);
    shadow_toggle.set_position(114, 4);
    shadow_toggle.on_checked_changed(|e| {
        let s = unsafe { STATE.as_mut().unwrap() };
        s.shadows_enabled = e.checked;
        s.settings.shadows_enabled = e.checked;
    });
    window.add(&shadow_toggle);

    let fps_label = libanyui_client::Label::new("FPS: --");
    fps_label.set_position(150, 4);
    fps_label.set_text_color(0xFFFFFFFF);
    fps_label.set_font_size(14);
    window.add(&fps_label);

    let sun_debug_label = libanyui_client::Label::new("Zeit --:-- | Sonne --");
    sun_debug_label.set_position(500, 4);
    sun_debug_label.set_text_color(0xFFFFFFFF);
    sun_debug_label.set_font_size(13);
    window.add(&sun_debug_label);

    let menu_ui = menu::build(&window, canvas_w, canvas_h, &settings);
    let world_summaries = save::load_world_summaries();

    unsafe {
        STATE = Some(GameState {
            canvas,
            window,
            mode_toggle,
            shadow_toggle,
            menu_ui,
            app_mode: AppMode::MainMenu,
            settings,
            world_summaries,
            current_world_id: String::new(),
            current_world_name: String::new(),
            canvas_w,
            canvas_h,
            fb_w,
            fb_h,
            render_divisor,
            world: preview_world,
            renderer,
            player: Player::new_uninit(),
            inventory: Inventory::new(),
            fps_frame_count: 0,
            fps_last_ms: anyos_std::sys::uptime_ms(),
            fps_display: 0,
            fps_label,
            sun_debug_label,
            upscale_buffer: vec![0u32; (canvas_w * canvas_h) as usize],
            last_mouse_x: 400,
            last_mouse_y: 300,
            mouse_captured: false,
            fullscreen: false,
            shadows_enabled: false,
            mining_active: false,
            mining_target: None,
            mining_progress: 0.0,
            autosave_at_ms: anyos_std::sys::uptime_ms().wrapping_add(4000),
        });
    }

    physics::physics_init(world_query);

    unsafe {
        let s = STATE.as_mut().unwrap();
        state::apply_settings(s);
        menu::refresh_menu_state(s);
    }

    let window_ref = unsafe { &STATE.as_ref().unwrap().window };
    window_ref.on_close(|_| {
        let s = unsafe { STATE.as_ref().unwrap() };
        save_if_needed(s);
    });

    window_ref.on_key_down(|ke| {
        let s = unsafe { STATE.as_mut().unwrap() };
        if s.app_mode != AppMode::InGame {
            if ke.keycode == libanyui_client::KEY_ESCAPE {
                s.app_mode = AppMode::MainMenu;
                menu::refresh_menu_state(s);
            }
            return;
        }

        let ch = ke.char_code;
        match ch {
            c if c == b'w' as u32 || c == b'W' as u32 => s.player.forward = true,
            c if c == b's' as u32 || c == b'S' as u32 => s.player.backward = true,
            c if c == b'a' as u32 || c == b'A' as u32 => s.player.left = true,
            c if c == b'd' as u32 || c == b'D' as u32 => s.player.right = true,
            c if c == b' ' as u32 => s.player.jump = true,
            c if c == b'f' as u32 || c == b'F' as u32 => s.player.ascend = true,
            c if c == b'c' as u32 || c == b'C' as u32 => s.player.descend = true,
            c if c == b'g' as u32 || c == b'G' as u32 => {
                s.player.toggle_fly();
                s.mode_toggle
                    .set_state(if s.player.is_flying() { 1 } else { 0 });
            }
            c if (b'1' as u32..=b'9' as u32).contains(&c) => {
                s.inventory.set_selected_slot((c - b'1' as u32) as usize);
                game::sync_selected_block(&s.inventory, &mut s.player);
            }
            _ => match ke.keycode {
                libanyui_client::KEY_ESCAPE => {
                    if s.mouse_captured {
                        release_mouse(s);
                    } else {
                        save_if_needed(s);
                        s.app_mode = AppMode::MainMenu;
                        menu::refresh_menu_state(s);
                    }
                }
                libanyui_client::KEY_LEFT => s.player.look_left = true,
                libanyui_client::KEY_RIGHT => s.player.look_right = true,
                libanyui_client::KEY_UP => s.player.look_up = true,
                libanyui_client::KEY_DOWN => s.player.look_down = true,
                _ => {}
            },
        }
    });

    window_ref.on_key_up(|ke| {
        let s = unsafe { STATE.as_mut().unwrap() };
        if s.app_mode != AppMode::InGame {
            return;
        }
        let ch = ke.char_code;
        match ch {
            c if c == b'w' as u32 || c == b'W' as u32 => s.player.forward = false,
            c if c == b's' as u32 || c == b'S' as u32 => s.player.backward = false,
            c if c == b'a' as u32 || c == b'A' as u32 => s.player.left = false,
            c if c == b'd' as u32 || c == b'D' as u32 => s.player.right = false,
            c if c == b' ' as u32 => s.player.jump = false,
            c if c == b'f' as u32 || c == b'F' as u32 => s.player.ascend = false,
            c if c == b'c' as u32 || c == b'C' as u32 => s.player.descend = false,
            _ => match ke.keycode {
                libanyui_client::KEY_LEFT => s.player.look_left = false,
                libanyui_client::KEY_RIGHT => s.player.look_right = false,
                libanyui_client::KEY_UP => s.player.look_up = false,
                libanyui_client::KEY_DOWN => s.player.look_down = false,
                _ => {}
            },
        }
    });

    let canvas_ref = unsafe { &STATE.as_ref().unwrap().canvas };
    canvas_ref.on_mouse_down(|x, y, button| {
        let s = unsafe { STATE.as_mut().unwrap() };
        if s.app_mode != AppMode::InGame {
            return;
        }
        if !s.mouse_captured {
            capture_mouse(s, x, y);
            return;
        }
        let (ex, ey, ez) = s.player.eye_position();
        if let Some(hit) = player::raycast(&s.world, ex, ey, ez, s.player.yaw, s.player.pitch) {
            if button == 0 {
                s.mining_active = true;
                game::reset_mining(s);
            } else if button == 2 {
                place_targeted_block(s);
            }
        }
    });

    canvas_ref.on_mouse_up(|_x, _y, button| {
        let s = unsafe { STATE.as_mut().unwrap() };
        if s.app_mode != AppMode::InGame {
            return;
        }
        if button == 0 {
            s.mining_active = false;
            game::reset_mining(s);
        }
    });

    canvas_ref.on_mouse_move(|x, y| {
        let s = unsafe { STATE.as_mut().unwrap() };
        if s.app_mode != AppMode::InGame || !s.mouse_captured {
            return;
        }
        let dx = x - s.last_mouse_x;
        let dy = y - s.last_mouse_y;
        s.last_mouse_x = x;
        s.last_mouse_y = y;
        s.player.mouse_move(dx as f32, dy as f32);
    });

    window_ref.set_fullscreen_capable(false);

    window_ref.on_fullscreen_enter(|_| {
        let s = unsafe { STATE.as_mut().unwrap() };
        s.fullscreen = true;
        if let Some(info) = libanyui_client::get_fullscreen_info() {
            s.canvas_w = info.width;
            s.canvas_h = info.height;
            s.upscale_buffer
                .resize((info.width * info.height) as usize, 0);
            s.fb_w = (info.width / s.render_divisor).max(1);
            s.fb_h = (info.height / s.render_divisor).max(1);
            gl::gl_resize(s.fb_w, s.fb_h);
            gl::viewport(0, 0, s.fb_w as i32, s.fb_h as i32);
            menu::layout(&s.menu_ui, s.canvas_w, s.canvas_h);
        }
        if s.app_mode == AppMode::InGame {
            capture_mouse(s, s.canvas_w as i32 / 2, s.canvas_h as i32 / 2);
        }
    });

    window_ref.on_fullscreen_exit(|_| {
        let s = unsafe { STATE.as_mut().unwrap() };
        s.fullscreen = false;
        gl::gl_exit_fullscreen();
        release_mouse(s);
        let w = s.canvas.get_stride();
        let h = s.canvas.get_height();
        if w > 0 && h > 0 {
            s.canvas_w = w;
            s.canvas_h = h;
            s.upscale_buffer.resize((w * h) as usize, 0);
            s.fb_w = (w / s.render_divisor).max(1);
            s.fb_h = (h / s.render_divisor).max(1);
            gl::gl_resize(s.fb_w, s.fb_h);
            gl::viewport(0, 0, s.fb_w as i32, s.fb_h as i32);
            menu::layout(&s.menu_ui, s.canvas_w, s.canvas_h);
        }
    });

    unsafe {
        STATE.as_mut().unwrap().fps_last_ms = anyos_std::sys::uptime_ms();
    }

    anyos_std::println!("forger: entering event loop");
    libanyui_client::set_timer(33, || {
        game::game_tick();
    });
    libanyui_client::run();
}
