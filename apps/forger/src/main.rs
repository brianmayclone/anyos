#![no_std]
#![no_main]
#![allow(unused, dead_code, static_mut_refs)]

anyos_std::entry!(main);

use alloc::vec;
use alloc::vec::Vec;

use libgl_client as gl;
use libphysics_client as physics;

mod block;
mod noise;
mod world;
mod textures;
mod mesh;
mod render;
mod player;
mod ui;
mod state;
mod game;
mod inventory;

use world::World;
use render::Renderer;
use player::Player;
use inventory::Inventory;
use state::{GameState, STATE, world_query, find_spawn_height};

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

    let canvas_w = canvas.get_stride();
    let canvas_h = canvas.get_height();
    let render_divisor = 2;
    let fb_w = (canvas_w / render_divisor).max(1);
    let fb_h = (canvas_h / render_divisor).max(1);

    if !gl::init() {
        anyos_std::println!("forger: FATAL - failed to load libgl.so");
        return;
    }
    anyos_std::println!("forger: libgl loaded, canvas={}x{} render={}x{}", canvas_w, canvas_h, fb_w, fb_h);
    gl::gl_init(fb_w, fb_h);
    gl::enable(gl::GL_DEPTH_TEST);
    gl::depth_func(gl::GL_LESS);
    gl::enable(gl::GL_CULL_FACE);
    gl::cull_face(gl::GL_BACK);
    // Blending disabled for performance (all fragments output alpha=1.0)

    if !physics::init() {
        anyos_std::println!("forger: FATAL - failed to load libphysics.so");
        return;
    }
    anyos_std::println!("forger: libphysics loaded");

    let atlas_data = textures::generate_atlas();
    let renderer = Renderer::init(&atlas_data, textures::ATLAS_W as u32, textures::ATLAS_H as u32);

    let mut world = World::new(42);
    anyos_std::println!("forger: generating chunks...");
    world.ensure_chunks_around(0, 0, 2);
    anyos_std::println!("forger: {} chunks generated", world.chunks.len());

    // Build and upload initial chunk meshes
    let mut renderer = renderer;
    let keys: Vec<(i32, i32)> = world.chunks.keys().copied().collect();
    let mut total_verts: u32 = 0;
    for (cx, cz) in &keys {
        let m = mesh::build_chunk_mesh(&world, *cx, *cz);
        let vc = m.vertex_count;
        total_verts += vc;
        renderer.upload_chunk(*cx, *cz, &m);
        if let Some(chunk) = world.chunks.get_mut(&(*cx, *cz)) {
            chunk.dirty = false;
        }
    }

    let spawn_y = find_spawn_height(&world);
    anyos_std::println!("forger: {} chunks, {} VBOs, {} total verts, spawn_y={}", world.chunks.len(), renderer.chunk_vbos.len(), total_verts, spawn_y as i32);

    let mode_label = libanyui_client::Label::new("Fly");
    mode_label.set_position(6, 6);
    mode_label.set_text_color(0xFFFFFFFF);
    mode_label.set_font_size(13);
    window.add(&mode_label);

    let mode_toggle = libanyui_client::Toggle::new(false);
    mode_toggle.set_position(30, 4);
    mode_toggle.on_checked_changed(|e| {
        let s = unsafe { STATE.as_mut().unwrap() };
        s.player.set_flying(e.checked);
    });
    window.add(&mode_toggle);

    let shadow_label = libanyui_client::Label::new("Shadow");
    shadow_label.set_position(62, 6);
    shadow_label.set_text_color(0xFFFFFFFF);
    shadow_label.set_font_size(13);
    window.add(&shadow_label);

    let shadow_toggle = libanyui_client::Toggle::new(false);
    shadow_toggle.set_position(114, 4);
    shadow_toggle.on_checked_changed(|e| {
        let s = unsafe { STATE.as_mut().unwrap() };
        s.shadows_enabled = e.checked;
    });
    window.add(&shadow_toggle);

    let fps_label = libanyui_client::Label::new("FPS: --");
    fps_label.set_position(150, 4);
    fps_label.set_text_color(0xFFFFFFFF);
    fps_label.set_font_size(14);
    window.add(&fps_label);

    // Store state with a dummy player first so world_query can access STATE.world
    unsafe {
        STATE = Some(GameState {
            canvas,
            window,
            mode_toggle,
            shadow_toggle,
            canvas_w,
            canvas_h,
            fb_w,
            fb_h,
            render_divisor,
            world,
            renderer,
            player: Player::new_uninit(),
            inventory: Inventory::new(),
            fps_frame_count: 0,
            fps_last_ms: anyos_std::sys::uptime_ms(),
            fps_display: 0,
            fps_label,
            upscale_buffer: vec![0u32; (canvas_w * canvas_h) as usize],
            last_mouse_x: 400,
            last_mouse_y: 300,
            mouse_captured: false,
            fullscreen: false,
            shadows_enabled: false,
            mining_active: false,
            mining_target: None,
            mining_progress: 0.0,
        });
    }

    // Now physics_init can use world_query which reads STATE.world
    physics::physics_init(world_query);

    // Create the actual player body (after physics is initialized)
    unsafe {
        let s = STATE.as_mut().unwrap();
        s.player = Player::new(0.0, spawn_y, 0.0);
        physics::set_flying(s.player.body_id, false);
        s.mode_toggle.set_state(0);
        game::sync_selected_block(&s.inventory, &mut s.player);
    }

    // Keyboard handlers
    let window_ref = unsafe { &STATE.as_ref().unwrap().window };
    window_ref.on_key_down(|ke| {
        let s = unsafe { STATE.as_mut().unwrap() };
        // Use char_code for character keys (ASCII), keycode for special keys (scancodes)
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
                s.mode_toggle.set_state(if s.player.is_flying() { 1 } else { 0 });
            }
            c if (b'1' as u32..=b'9' as u32).contains(&c) => {
                s.inventory.set_selected_slot((c - b'1' as u32) as usize);
                game::sync_selected_block(&s.inventory, &mut s.player);
            }
            _ => {
                // Check scancode for special keys (char_code=0 for non-printable)
                if ke.keycode == libanyui_client::KEY_ESCAPE {
                    release_mouse(s);
                }
            }
        }
    });

    window_ref.on_key_up(|ke| {
        let s = unsafe { STATE.as_mut().unwrap() };
        let ch = ke.char_code;
        match ch {
            c if c == b'w' as u32 || c == b'W' as u32 => s.player.forward = false,
            c if c == b's' as u32 || c == b'S' as u32 => s.player.backward = false,
            c if c == b'a' as u32 || c == b'A' as u32 => s.player.left = false,
            c if c == b'd' as u32 || c == b'D' as u32 => s.player.right = false,
            c if c == b' ' as u32 => s.player.jump = false,
            c if c == b'f' as u32 || c == b'F' as u32 => s.player.ascend = false,
            c if c == b'c' as u32 || c == b'C' as u32 => s.player.descend = false,
            _ => {}
        }
    });

    // Mouse handlers
    let canvas_ref = unsafe { &STATE.as_ref().unwrap().canvas };
    canvas_ref.on_mouse_down(|x, y, button| {
        let s = unsafe { STATE.as_mut().unwrap() };
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
                if let Some(block_id) = s.inventory.selected_block() {
                    if s.world.get_block(hit.prev_x, hit.prev_y, hit.prev_z) == block::AIR
                        && s.inventory.consume_selected()
                    {
                        s.world.set_block(hit.prev_x, hit.prev_y, hit.prev_z, block_id);
                        game::sync_selected_block(&s.inventory, &mut s.player);
                    }
                }
            }
        }
    });

    canvas_ref.on_mouse_up(|_x, _y, button| {
        let s = unsafe { STATE.as_mut().unwrap() };
        if button == 0 {
            s.mining_active = false;
            game::reset_mining(s);
        }
    });

    canvas_ref.on_mouse_move(|x, y| {
        let s = unsafe { STATE.as_mut().unwrap() };
        if !s.mouse_captured {
            return;
        }
        let dx = x - s.last_mouse_x;
        let dy = y - s.last_mouse_y;
        s.last_mouse_x = x;
        s.last_mouse_y = y;
        s.player.mouse_move(dx as f32, dy as f32);
    });

    // ── Fullscreen support ────────────────────────────────────────────────
    window_ref.set_fullscreen_capable(false);

    window_ref.on_fullscreen_enter(|_| {
        let s = unsafe { STATE.as_mut().unwrap() };
        s.fullscreen = true;
        if let Some(info) = libanyui_client::get_fullscreen_info() {
            s.canvas_w = info.width;
            s.canvas_h = info.height;
            s.upscale_buffer.resize((info.width * info.height) as usize, 0);
            s.fb_w = (info.width / s.render_divisor).max(1);
            s.fb_h = (info.height / s.render_divisor).max(1);
            gl::gl_resize(s.fb_w, s.fb_h);
            gl::viewport(0, 0, s.fb_w as i32, s.fb_h as i32);
        }
        // Hide cursor and capture mouse in fullscreen
        capture_mouse(s, s.canvas_w as i32 / 2, s.canvas_h as i32 / 2);
        anyos_std::println!("forger: fullscreen ENTER canvas={}x{} render={}x{}", s.canvas_w, s.canvas_h, s.fb_w, s.fb_h);
    });

    window_ref.on_fullscreen_exit(|_| {
        let s = unsafe { STATE.as_mut().unwrap() };
        s.fullscreen = false;
        gl::gl_exit_fullscreen();
        // Show cursor and release capture when leaving fullscreen
        release_mouse(s);
        // Restore canvas size from actual widget
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
        }
        anyos_std::println!("forger: fullscreen EXIT {}x{}", s.canvas_w, s.canvas_h);
    });

    // Reset FPS timer just before event loop so init time is not counted
    unsafe {
        STATE.as_mut().unwrap().fps_last_ms = anyos_std::sys::uptime_ms();
    }

    anyos_std::println!("forger: entering event loop");

    // Game loop timer
    libanyui_client::set_timer(33, || {
        game::game_tick();
    });

    libanyui_client::run();
}
