use alloc::format;
use alloc::string::String;

use crate::block;
use crate::inventory::{HOTBAR_SLOTS, Inventory};

/// Format stats into a window title string.
pub fn format_title(fps: u32, x: f32, y: f32, z: f32, block_name: &str, fog_chunks: f32) -> String {
    format!(
        "Forger | FPS: {} | Pos: {:.1}, {:.1}, {:.1} | Block: {} | View: {:.0} chunks",
        fps, x, y, z, block_name, fog_chunks
    )
}

pub fn draw_hud(
    canvas: &libanyui_client::Canvas,
    canvas_w: u32,
    canvas_h: u32,
    inventory: &Inventory,
    mining_progress: f32,
    target_block: Option<u8>,
) {
    draw_crosshair(canvas, canvas_w, canvas_h);
    draw_hotbar(canvas, canvas_w, canvas_h, inventory);
    draw_mining_overlay(canvas, canvas_w, canvas_h, mining_progress, target_block);
}

fn draw_crosshair(canvas: &libanyui_client::Canvas, canvas_w: u32, canvas_h: u32) {
    let cx = canvas_w as i32 / 2;
    let cy = canvas_h as i32 / 2;
    canvas.draw_line(cx - 8, cy, cx - 2, cy, 0xE6FFFFFF);
    canvas.draw_line(cx + 2, cy, cx + 8, cy, 0xE6FFFFFF);
    canvas.draw_line(cx, cy - 8, cx, cy - 2, 0xE6FFFFFF);
    canvas.draw_line(cx, cy + 2, cx, cy + 8, 0xE6FFFFFF);
    canvas.draw_rect(cx - 1, cy - 1, 3, 3, 0xAA000000, 1);
}

fn draw_hotbar(
    canvas: &libanyui_client::Canvas,
    canvas_w: u32,
    canvas_h: u32,
    inventory: &Inventory,
) {
    let slot_w = 56i32;
    let slot_h = 40i32;
    let gap = 6i32;
    let total_w = HOTBAR_SLOTS as i32 * slot_w + (HOTBAR_SLOTS as i32 - 1) * gap;
    let start_x = (canvas_w as i32 - total_w) / 2;
    let y = canvas_h as i32 - slot_h - 18;

    canvas.fill_rect(start_x - 10, y - 10, (total_w + 20) as u32, (slot_h + 20) as u32, 0x7A11151B);
    canvas.draw_rect(start_x - 10, y - 10, (total_w + 20) as u32, (slot_h + 20) as u32, 0xCC8CA0B3, 1);

    for slot in 0..HOTBAR_SLOTS {
        let x = start_x + slot as i32 * (slot_w + gap);
        let selected = slot == inventory.selected_slot();
        let fill = if selected { 0xDD2B333D } else { 0xB01B2027 };
        let border = if selected { 0xFFF2C94C } else { 0xCC657280 };
        canvas.fill_rect(x, y, slot_w as u32, slot_h as u32, fill);
        canvas.draw_rect(x, y, slot_w as u32, slot_h as u32, border, if selected { 2 } else { 1 });
        let label = alloc::format!("{}", slot + 1);
        canvas.draw_text(x + 4, y + 4, 0xA0FFFFFF, 4, 10, &label);

        if let Some(block_id) = inventory.slot_block(slot) {
            canvas.draw_text(x + 8, y + 14, 0xFFFFFFFF, 1, 12, block::BLOCK_SHORT_NAMES[block_id as usize]);
            let count = alloc::format!("{}", inventory.slot_count(slot));
            canvas.draw_text(x + slot_w - 16, y + 25, 0xFFD9E3EA, 1, 11, &count);
        }
    }
}

fn draw_mining_overlay(
    canvas: &libanyui_client::Canvas,
    canvas_w: u32,
    canvas_h: u32,
    mining_progress: f32,
    target_block: Option<u8>,
) {
    let Some(block_id) = target_block else {
        return;
    };
    if mining_progress <= 0.0 {
        return;
    }

    let cx = canvas_w as i32 / 2;
    let cy = canvas_h as i32 / 2;
    let crack_stage = ((mining_progress.clamp(0.0, 0.999) * 3.0) as u32).min(2) + 1;
    let box_size = 46i32;
    let x = cx - box_size / 2;
    let y = cy - box_size / 2;

    canvas.fill_rect(x, y, box_size as u32, box_size as u32, 0x28000000);
    canvas.draw_rect(x, y, box_size as u32, box_size as u32, 0xE6FFFFFF, 1);

    if crack_stage >= 1 {
        canvas.draw_line(x + 8, y + 8, x + 38, y + 36, 0xCCFFFFFF);
        canvas.draw_line(x + 18, y + 10, x + 12, y + 30, 0xCCFFFFFF);
    }
    if crack_stage >= 2 {
        canvas.draw_line(x + 30, y + 10, x + 16, y + 36, 0xCCFFFFFF);
        canvas.draw_line(x + 8, y + 24, x + 36, y + 20, 0xCCFFFFFF);
    }
    if crack_stage >= 3 {
        canvas.draw_line(x + 10, y + 38, x + 34, y + 8, 0xCCFFFFFF);
        canvas.draw_line(x + 22, y + 8, x + 24, y + 38, 0xCCFFFFFF);
    }

    let bar_w = 120i32;
    let bar_h = 8i32;
    let bar_x = cx - bar_w / 2;
    let bar_y = cy + 36;
    canvas.fill_rect(bar_x, bar_y, bar_w as u32, bar_h as u32, 0x90111418);
    canvas.fill_rect(
        bar_x,
        bar_y,
        (bar_w as f32 * mining_progress.clamp(0.0, 1.0)) as u32,
        bar_h as u32,
        0xFFD7A43C,
    );
    canvas.draw_rect(bar_x, bar_y, bar_w as u32, bar_h as u32, 0xD0FFFFFF, 1);
    canvas.draw_text(
        bar_x,
        bar_y - 16,
        0xFFFFFFFF,
        1,
        12,
        block::BLOCK_NAMES[block_id as usize],
    );
}
