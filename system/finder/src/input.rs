//! Click handling — toolbar, sidebar, file list.

use crate::navigation::*;
use crate::types::*;

/// Double-click timing: 400ms in PIT ticks (computed at runtime).
static mut DBLCLICK_TICKS: u32 = 40;

pub fn init_dblclick_timing() {
    let hz = anyos_std::sys::tick_hz();
    if hz > 0 {
        unsafe { DBLCLICK_TICKS = hz * 400 / 1000; }
    }
}

pub fn handle_click(state: &mut AppState, mx: i32, my: i32, _win_w: u32, _win_h: u32) -> bool {
    if my < TOOLBAR_H as i32 {
        return handle_toolbar_click(state, mx);
    }

    let content_y = TOOLBAR_H as i32;

    // Sidebar
    if mx < SIDEBAR_W as i32 {
        let y0 = content_y + SIDEBAR_HEADER_H as i32;
        for i in 0..LOCATIONS.len() {
            let iy = y0 + i as i32 * SIDEBAR_ITEM_H as i32;
            if my >= iy && my < iy + SIDEBAR_ITEM_H as i32 {
                let (_, path) = LOCATIONS[i];
                navigate(state, path);
                return true;
            }
        }
        return false;
    }

    // File list
    let list_y = content_y;
    let header_h = ROW_H;

    if my < list_y + header_h {
        return false;
    }

    let file_area_y = list_y + header_h;
    let row_idx = ((my - file_area_y) as u32 + state.scroll_offset * ROW_H as u32) / ROW_H as u32;
    let row = row_idx as usize;

    if row < state.entries.len() {
        let now = anyos_std::sys::uptime();

        if let Some(last_idx) = state.last_click_idx {
            if last_idx == row && now.wrapping_sub(state.last_click_tick) < unsafe { DBLCLICK_TICKS } {
                state.last_click_idx = None;
                open_entry(state, row);
                return true;
            }
        }

        state.selected = Some(row);
        state.last_click_idx = Some(row);
        state.last_click_tick = now;
        true
    } else {
        state.selected = None;
        state.last_click_idx = None;
        true
    }
}

fn handle_toolbar_click(state: &mut AppState, mx: i32) -> bool {
    if mx >= 8 && mx < 8 + NAV_BTN_W {
        navigate_back(state);
        return true;
    }
    if mx >= 42 && mx < 42 + NAV_BTN_W {
        navigate_forward(state);
        return true;
    }
    if mx >= 76 && mx < 76 + NAV_BTN_W {
        state.entries = read_directory(&state.cwd);
        state.selected = None;
        state.scroll_offset = 0;
        return true;
    }
    false
}
