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
    canvas_w: u32,
    canvas_h: u32,
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
    fullscreen: bool,
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
    let fb_w = canvas_w / 2;
    let fb_h = canvas_h / 2;

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
            canvas_w,
            canvas_h,
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
            fullscreen: false,
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
            c if c == b'g' as u32 || c == b'G' as u32 => s.player.toggle_fly(),
            c if c == b'1' as u32 => s.player.scroll_block(-1),
            c if c == b'2' as u32 => s.player.scroll_block(1),
            _ => {
                // Check scancode for special keys (char_code=0 for non-printable)
                if ke.keycode == libanyui_client::KEY_ESCAPE {
                    s.mouse_captured = false;
                    if s.fullscreen {
                        s.window.set_cursor_visible(true);
                        gl::set_cursor_captured(false);
                    }
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
            s.mouse_captured = true;
            s.last_mouse_x = x;
            s.last_mouse_y = y;
            if s.fullscreen {
                s.window.set_cursor_visible(false);
                gl::set_cursor_captured(true);
            }
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

    // ── Fullscreen support ────────────────────────────────────────────────
    window_ref.set_fullscreen_capable(true);

    window_ref.on_fullscreen_enter(|_| {
        let s = unsafe { STATE.as_mut().unwrap() };
        s.fullscreen = true;
        if let Some(info) = libanyui_client::get_fullscreen_info() {
            s.canvas_w = info.width;
            s.canvas_h = info.height;
            s.fb_w = info.width / 2;
            s.fb_h = info.height / 2;
            gl::gl_resize(s.fb_w, s.fb_h);
            gl::viewport(0, 0, s.fb_w as i32, s.fb_h as i32);
        }
        // Hide cursor and capture mouse in fullscreen
        s.window.set_cursor_visible(false);
        gl::set_cursor_captured(true);
        s.mouse_captured = true;
        anyos_std::println!("forger: fullscreen ENTER canvas={}x{} render={}x{}", s.canvas_w, s.canvas_h, s.fb_w, s.fb_h);
    });

    window_ref.on_fullscreen_exit(|_| {
        let s = unsafe { STATE.as_mut().unwrap() };
        s.fullscreen = false;
        gl::gl_exit_fullscreen();
        // Show cursor and release capture when leaving fullscreen
        s.window.set_cursor_visible(true);
        gl::set_cursor_captured(false);
        // Restore canvas size from actual widget
        let w = s.canvas.get_stride();
        let h = s.canvas.get_height();
        if w > 0 && h > 0 {
            s.canvas_w = w;
            s.canvas_h = h;
            s.fb_w = w / 2;
            s.fb_h = h / 2;
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
        game_tick();
    });

    libanyui_client::run();
}

fn game_tick() {
    let s = unsafe { STATE.as_mut().unwrap() };

    // Handle resize (render at half resolution).
    // In fullscreen mode, dimensions are managed by the fullscreen callback — skip widget query
    // to avoid stale logical sizes overriding the physical fullscreen dimensions.
    if !s.fullscreen {
        let cur_w = s.canvas.get_stride();
        let cur_h = s.canvas.get_height();
        if cur_w > 0 && cur_h > 0 && (cur_w != s.canvas_w || cur_h != s.canvas_h) {
            s.canvas_w = cur_w;
            s.canvas_h = cur_h;
            s.fb_w = cur_w / 2;
            s.fb_h = cur_h / 2;
            gl::gl_resize(s.fb_w, s.fb_h);
            gl::viewport(0, 0, s.fb_w as i32, s.fb_h as i32);
        }
    }

    // Physics/player update
    s.player.update(1.0 / 60.0);

    // Ensure chunks around player (limit to 3 to keep SW rasterizer manageable)
    let (px, _, pz) = s.player.position();
    let view_chunks = (s.renderer.fog_distance / 16.0) as i32 + 1;
    s.world.ensure_chunks_around(px as i32, pz as i32, view_chunks.min(3));

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

    // Sync camera
    let (ex, ey, ez) = s.player.eye_position();
    s.renderer.yaw = s.player.yaw;
    s.renderer.pitch = s.player.pitch;

    // Render
    s.renderer.render(ex, ey, ez, s.fb_w, s.fb_h);

    // Swap: upscale from half-res render buffer to display
    let fb_ptr = gl::swap_buffers();
    if !fb_ptr.is_null() {
        let src = unsafe { core::slice::from_raw_parts(fb_ptr, (s.fb_w * s.fb_h) as usize) };
        let cw = s.canvas_w as usize;
        let ch = s.canvas_h as usize;
        let rw = s.fb_w as usize;
        let rh = s.fb_h as usize;
        // Upscale from render buffer to canvas (works for both windowed and fullscreen)
        let mut upscaled = vec![0u32; cw * ch];
        for cy in 0..ch {
            let sy = (cy * rh / ch).min(rh - 1);
            let src_row = sy * rw;
            let dst_row = cy * cw;
            for cx in 0..cw {
                let sx = (cx * rw / cw).min(rw - 1);
                upscaled[dst_row + cx] = src[src_row + sx];
            }
        }
        s.canvas.copy_pixels_from(&upscaled);
    }

    // Debug: print camera and pixel samples once per second
    if s.fps_frame_count == 0 {
        anyos_std::println!("forger: cam=({},{},{}) vbos={} fb_null={}", ex as i32, ey as i32, ez as i32, s.renderer.chunk_vbos.len(), fb_ptr.is_null());
        if !fb_ptr.is_null() {
            let src = unsafe { core::slice::from_raw_parts(fb_ptr, (s.fb_w * s.fb_h) as usize) };
            let mid = (s.fb_w * s.fb_h / 2) as usize;
            let top = (s.fb_w * 10 + s.fb_w / 2) as usize; // sky area
            anyos_std::println!("forger: px[0]=0x{:08X} px[mid]=0x{:08X} px[sky]=0x{:08X} fb={}x{}",
                src[0], src[mid.min(src.len()-1)], src[top.min(src.len()-1)], s.fb_w, s.fb_h);
        }
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
    }

    // Debug overlay (update every frame for smooth coordinate display)
    let flying = libphysics_client::is_flying(s.player.body_id);
    let mode = if flying { "FLY" } else { "WALK" };
    let debug_text = alloc::format!(
        "FPS: {} | X: {:.1} Y: {:.1} Z: {:.1} | {}",
        s.fps_display, ex, ey, ez, mode
    );
    s.fps_label.set_text(&debug_text);
}
