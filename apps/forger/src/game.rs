use crate::block;
use crate::inventory::Inventory;
use crate::mesh;
use crate::player;
use crate::state::{MiningTarget, STATE};

const FIXED_DT: f32 = 1.0 / 60.0;

pub fn game_tick() {
    let s = unsafe { STATE.as_mut().unwrap() };

    if !s.fullscreen {
        let cur_w = s.canvas.get_stride();
        let cur_h = s.canvas.get_height();
        if cur_w > 0 && cur_h > 0 && (cur_w != s.canvas_w || cur_h != s.canvas_h) {
            s.canvas_w = cur_w;
            s.canvas_h = cur_h;
            s.upscale_buffer.resize((cur_w * cur_h) as usize, 0);
            s.fb_w = (cur_w / s.render_divisor).max(1);
            s.fb_h = (cur_h / s.render_divisor).max(1);
            libgl_client::gl_resize(s.fb_w, s.fb_h);
            libgl_client::viewport(0, 0, s.fb_w as i32, s.fb_h as i32);
        }
    }

    s.player.update(FIXED_DT);

    let (px, _, pz) = s.player.position();
    let view_chunks = (s.renderer.fog_distance / 16.0) as i32 + 1;
    s.world.ensure_chunks_around(px as i32, pz as i32, view_chunks.min(3));

    let mut rebuilt = 0;
    let keys: alloc::vec::Vec<(i32, i32)> = s.world.chunks.keys().copied().collect();
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

    let (ex, ey, ez) = s.player.eye_position();
    let ray_hit = player::raycast(&s.world, ex, ey, ez, s.player.yaw, s.player.pitch);
    update_mining(s, ray_hit.as_ref());
    s.renderer.yaw = s.player.yaw;
    s.renderer.pitch = s.player.pitch;
    s.renderer.render(ex, ey, ez, s.fb_w, s.fb_h, s.shadows_enabled);

    let fb_ptr = libgl_client::swap_buffers();
    if !fb_ptr.is_null() {
        let src = unsafe { core::slice::from_raw_parts(fb_ptr, (s.fb_w * s.fb_h) as usize) };
        let cw = s.canvas_w as usize;
        let ch = s.canvas_h as usize;
        let rw = s.fb_w as usize;
        let rh = s.fb_h as usize;
        if s.upscale_buffer.len() != cw * ch {
            s.upscale_buffer.resize(cw * ch, 0);
        }
        for cy in 0..ch {
            let sy = (cy * rh / ch).min(rh - 1);
            let src_row = sy * rw;
            let dst_row = cy * cw;
            for cx in 0..cw {
                let sx = (cx * rw / cw).min(rw - 1);
                s.upscale_buffer[dst_row + cx] = src[src_row + sx];
            }
        }
        s.canvas.copy_pixels_from(&s.upscale_buffer);
        let mining_block = s.mining_target.map(|target| target.block_id);
        crate::ui::draw_hud(
            &s.canvas,
            s.canvas_w,
            s.canvas_h,
            &s.inventory,
            s.mining_progress,
            mining_block,
        );
    }

    if s.fps_frame_count == 0 {
        anyos_std::println!(
            "forger: cam=({},{},{}) vbos={} fb_null={}",
            ex as i32,
            ey as i32,
            ez as i32,
            s.renderer.chunk_vbos.len(),
            fb_ptr.is_null()
        );
        if !fb_ptr.is_null() {
            let src = unsafe { core::slice::from_raw_parts(fb_ptr, (s.fb_w * s.fb_h) as usize) };
            let mid = (s.fb_w * s.fb_h / 2) as usize;
            let top = (s.fb_w * 10 + s.fb_w / 2) as usize;
            anyos_std::println!(
                "forger: px[0]=0x{:08X} px[mid]=0x{:08X} px[sky]=0x{:08X} fb={}x{}",
                src[0],
                src[mid.min(src.len() - 1)],
                src[top.min(src.len() - 1)],
                s.fb_w,
                s.fb_h
            );
        }
    }

    s.fps_frame_count += 1;
    let now = anyos_std::sys::uptime_ms();
    let elapsed = now.wrapping_sub(s.fps_last_ms);
    if elapsed >= 1000 {
        s.fps_display = s.fps_frame_count * 1000 / elapsed;
        s.fps_frame_count = 0;
        s.fps_last_ms = now;
        s.renderer.adapt_view_distance(s.fps_display as f32);
        let desired_divisor = if s.fps_display < 18 { 3 } else { 2 };
        if desired_divisor != s.render_divisor {
            s.render_divisor = desired_divisor;
            s.fb_w = (s.canvas_w / s.render_divisor).max(1);
            s.fb_h = (s.canvas_h / s.render_divisor).max(1);
            libgl_client::gl_resize(s.fb_w, s.fb_h);
            libgl_client::viewport(0, 0, s.fb_w as i32, s.fb_h as i32);
        }

        let title = crate::ui::format_title(
            s.fps_display,
            ex,
            ey,
            ez,
            selected_block_name(&s.inventory),
            s.renderer.fog_distance / 16.0,
        );
        s.window.set_title(&title);
    }

    let mode = if s.player.is_flying() { "FLY" } else { "WALK" };
    let debug_text = alloc::format!(
        "FPS: {} | X: {:.1} Y: {:.1} Z: {:.1} | {} | {}x",
        s.fps_display, ex, ey, ez, mode, s.render_divisor
    );
    s.fps_label.set_text(&debug_text);
}

pub fn sync_selected_block(inventory: &Inventory, player: &mut player::Player) {
    player.selected_block = inventory.selected_block().unwrap_or(block::AIR);
}

pub fn reset_mining(s: &mut crate::state::GameState) {
    s.mining_target = None;
    s.mining_progress = 0.0;
}

fn update_mining(s: &mut crate::state::GameState, ray_hit: Option<&player::RayHit>) {
    if !s.mining_active {
        reset_mining(s);
        return;
    }

    let Some(hit) = ray_hit else {
        reset_mining(s);
        return;
    };
    let block_id = s.world.get_block(hit.x, hit.y, hit.z);
    let Some(break_time) = block::break_time_seconds(block_id, block::ToolKind::Hand) else {
        reset_mining(s);
        return;
    };

    let same_target = matches!(
        s.mining_target,
        Some(target) if target.x == hit.x && target.y == hit.y && target.z == hit.z && target.block_id == block_id
    );

    if !same_target {
        s.mining_target = Some(MiningTarget {
            x: hit.x,
            y: hit.y,
            z: hit.z,
            block_id,
        });
        s.mining_progress = 0.0;
    }

    s.mining_progress = (s.mining_progress + FIXED_DT / break_time).min(1.0);
    if s.mining_progress >= 1.0 {
        s.inventory.add_block(block_id);
        sync_selected_block(&s.inventory, &mut s.player);
        s.world.set_block(hit.x, hit.y, hit.z, block::AIR);
        reset_mining(s);
    }
}

fn selected_block_name(inventory: &Inventory) -> &'static str {
    inventory
        .selected_block()
        .map(|block_id| block::BLOCK_NAMES[block_id as usize])
        .unwrap_or("Hands")
}
