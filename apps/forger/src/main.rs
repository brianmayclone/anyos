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

use world::World;
use render::Renderer;
use player::Player;

struct GameState {
    canvas: libanyui_client::Canvas,
    window: libanyui_client::Window,
    fb_w: u32,
    fb_h: u32,
    world: World,
    renderer: Renderer,
    player: Player,
    fps_frame_count: u32,
    fps_last_ms: u32,
    fps_display: u32,
    fps_label: libanyui_client::Label,
    last_mouse_x: i32,
    last_mouse_y: i32,
    mouse_captured: bool,
}

static mut STATE: Option<GameState> = None;

extern "C" fn world_query(x: i32, y: i32, z: i32) -> bool {
    unsafe { STATE.as_ref().map_or(false, |s| s.world.is_solid(x, y, z)) }
}

fn find_spawn_height(world: &World) -> f32 {
    for y in (1..200).rev() {
        if world.is_solid(0, y, 0) {
            return y as f32 + 2.0;
        }
    }
    80.0
}

fn main() {
    libanyui_client::init();
    anyos_std::i18n::init();

    let window = libanyui_client::Window::new("Forger", 50, 50, 800, 600);
    let canvas = libanyui_client::Canvas::new(800, 600);
    canvas.set_dock(libanyui_client::DOCK_FILL);
    canvas.set_interactive(true);
    window.add(&canvas);
    window.set_visible(true);

    let fb_w = canvas.get_stride();
    let fb_h = canvas.get_height();

    gl::init();
    gl::gl_init(fb_w, fb_h);
    gl::set_hw_backend(false);
    gl::enable(gl::GL_DEPTH_TEST);
    gl::depth_func(gl::GL_LESS);
    gl::enable(gl::GL_BLEND);
    gl::blend_func(gl::GL_SRC_ALPHA, gl::GL_ONE_MINUS_SRC_ALPHA);

    physics::init();

    let atlas_data = textures::generate_atlas();
    let renderer = Renderer::init(&atlas_data, textures::ATLAS_W as u32, textures::ATLAS_H as u32);

    let mut world = World::new(42);
    world.ensure_chunks_around(0, 0, 4);

    // Build and upload initial chunk meshes
    let mut renderer = renderer;
    let keys: Vec<(i32, i32)> = world.chunks.keys().copied().collect();
    for (cx, cz) in keys {
        let m = mesh::build_chunk_mesh(&world, cx, cz);
        renderer.upload_chunk(cx, cz, &m);
        if let Some(chunk) = world.chunks.get_mut(&(cx, cz)) {
            chunk.dirty = false;
        }
    }

    let spawn_y = find_spawn_height(&world);

    let fps_label = libanyui_client::Label::new("FPS: --");
    fps_label.set_position(4, 4);
    fps_label.set_text_color(0xFFFFFFFF);
    fps_label.set_font_size(14);
    window.add(&fps_label);

    // Store state with a dummy player first so world_query can access STATE.world
    unsafe {
        STATE = Some(GameState {
            canvas,
            window,
            fb_w,
            fb_h,
            world,
            renderer,
            player: Player::new_uninit(),
            fps_frame_count: 0,
            fps_last_ms: anyos_std::sys::uptime_ms(),
            fps_display: 0,
            fps_label,
            last_mouse_x: 400,
            last_mouse_y: 300,
            mouse_captured: false,
        });
    }

    // Now physics_init can use world_query which reads STATE.world
    physics::physics_init(world_query);

    // Create the actual player body (after physics is initialized)
    unsafe {
        let s = STATE.as_mut().unwrap();
        s.player = Player::new(0.0, spawn_y, 0.0);
        // Start in fly mode so player doesn't fall while chunks load
        physics::set_flying(s.player.body_id, true);
    }

    // Keyboard handler
    let window_ref = unsafe { &STATE.as_ref().unwrap().window };
    window_ref.on_key_down(|ke| {
        let s = unsafe { STATE.as_mut().unwrap() };
        let kc = ke.keycode;
        match kc {
            k if k == b'w' as u32 || k == b'W' as u32 => s.player.forward = true,
            k if k == b's' as u32 || k == b'S' as u32 => s.player.backward = true,
            k if k == b'a' as u32 || k == b'A' as u32 => s.player.left = true,
            k if k == b'd' as u32 || k == b'D' as u32 => s.player.right = true,
            k if k == b' ' as u32 => s.player.jump = true,
            k if k == b'c' as u32 || k == b'C' as u32 => s.player.descend = true,
            k if k == b'f' as u32 || k == b'F' as u32 => s.player.toggle_fly(),
            k if k == b'1' as u32 => s.player.scroll_block(-1),
            k if k == b'2' as u32 => s.player.scroll_block(1),
            k if k == libanyui_client::KEY_ESCAPE => s.mouse_captured = false,
            _ => {}
        }
    });

    // Mouse handlers
    let canvas_ref = unsafe { &STATE.as_ref().unwrap().canvas };
    canvas_ref.on_mouse_down(|x, y, button| {
        let s = unsafe { STATE.as_mut().unwrap() };
        if !s.mouse_captured {
            s.mouse_captured = true;
            s.last_mouse_x = x;
            s.last_mouse_y = y;
            return;
        }
        let (ex, ey, ez) = s.player.eye_position();
        if let Some(hit) = player::raycast(&s.world, ex, ey, ez, s.player.yaw, s.player.pitch) {
            if button == 0 {
                // Left click: break block
                s.world.set_block(hit.x, hit.y, hit.z, block::AIR);
            } else if button == 2 {
                // Right click: place block
                s.world.set_block(hit.prev_x, hit.prev_y, hit.prev_z, s.player.selected_block);
            }
        }
    });

    canvas_ref.on_mouse_up(|_x, _y, _button| {});

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

    // Game loop timer
    libanyui_client::set_timer(16, || {
        game_tick();
    });

    libanyui_client::run();
}

fn game_tick() {
    let s = unsafe { STATE.as_mut().unwrap() };

    // Handle resize
    let cur_w = s.canvas.get_stride();
    let cur_h = s.canvas.get_height();
    if cur_w != s.fb_w || cur_h != s.fb_h {
        s.fb_w = cur_w;
        s.fb_h = cur_h;
        gl::gl_resize(cur_w, cur_h);
        gl::viewport(0, 0, cur_w as i32, cur_h as i32);
    }

    // Physics/player update
    s.player.update(1.0 / 60.0);

    // Clear movement flags (re-set by key repeat events)
    s.player.forward = false;
    s.player.backward = false;
    s.player.left = false;
    s.player.right = false;
    s.player.jump = false;
    s.player.descend = false;

    // Ensure chunks around player
    let (px, _, pz) = s.player.position();
    let view_chunks = (s.renderer.fog_distance / 16.0) as i32 + 1;
    s.world.ensure_chunks_around(px as i32, pz as i32, view_chunks.min(8));

    // Rebuild dirty chunk meshes (max 2 per frame)
    let mut rebuilt = 0;
    let keys: Vec<(i32, i32)> = s.world.chunks.keys().copied().collect();
    for (cx, cz) in keys {
        if rebuilt >= 2 {
            break;
        }
        let is_dirty = s.world.chunks.get(&(cx, cz)).map_or(false, |c| c.dirty);
        if is_dirty {
            let m = mesh::build_chunk_mesh(&s.world, cx, cz);
            s.renderer.upload_chunk(cx, cz, &m);
            if let Some(chunk) = s.world.chunks.get_mut(&(cx, cz)) {
                chunk.dirty = false;
            }
            rebuilt += 1;
        }
    }

    // Day/night cycle: 10 min = 1 day
    s.renderer.time_of_day += 1.0 / (60.0 * 600.0);
    if s.renderer.time_of_day >= 1.0 {
        s.renderer.time_of_day -= 1.0;
    }

    // Sync camera
    let (ex, ey, ez) = s.player.eye_position();
    s.renderer.yaw = s.player.yaw;
    s.renderer.pitch = s.player.pitch;

    // Render
    s.renderer.render(ex, ey, ez, s.fb_w, s.fb_h);

    // Swap to canvas
    let fb_ptr = gl::swap_buffers();
    if !fb_ptr.is_null() {
        let pixels = unsafe { core::slice::from_raw_parts(fb_ptr, (s.fb_w * s.fb_h) as usize) };
        s.canvas.copy_pixels_from(pixels);
    }

    // FPS counter
    s.fps_frame_count += 1;
    let now = anyos_std::sys::uptime_ms();
    let elapsed = now.wrapping_sub(s.fps_last_ms);
    if elapsed >= 1000 {
        s.fps_display = s.fps_frame_count * 1000 / elapsed;
        s.fps_frame_count = 0;
        s.fps_last_ms = now;
        s.renderer.adapt_view_distance(s.fps_display as f32);

        let title = ui::format_title(
            s.fps_display,
            ex,
            ey,
            ez,
            block::BLOCK_NAMES[s.player.selected_block as usize],
            s.renderer.fog_distance / 16.0,
        );
        s.window.set_title(&title);
        s.fps_label.set_text(&alloc::format!("FPS: {}", s.fps_display));
    }
}
