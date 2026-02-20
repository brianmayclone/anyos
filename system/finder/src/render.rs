//! Rendering — toolbar, sidebar, file list, scrollbar.

use anyos_std::icons;
use anyos_std::ui::window;
use uisys_client::*;

use crate::format::{fmt_item_count, fmt_size};
use crate::icon_cache::{blit_icon_alpha, tint_icon};
use crate::navigation::build_full_path;
use crate::types::*;

pub fn render(win: u32, state: &mut AppState, win_w: u32, win_h: u32) {
    window::fill_rect(win, 0, 0, win_w as u16, win_h as u16, colors::WINDOW_BG());

    render_toolbar(win, state, win_w);

    let content_y = TOOLBAR_H as i32;
    let content_h = win_h - TOOLBAR_H;

    // Sidebar
    sidebar_bg(win, 0, content_y, SIDEBAR_W, content_h);
    sidebar_header(win, 0, content_y, SIDEBAR_W, "LOCATIONS");

    const SIDEBAR_SELECTION: u32 = 0xFF0A54C4;
    let y0 = content_y + SIDEBAR_HEADER_H as i32;
    for (i, &(name, _)) in LOCATIONS.iter().enumerate() {
        let iy = y0 + i as i32 * SIDEBAR_ITEM_H as i32;
        let selected = i == state.sidebar_sel;

        if selected {
            fill_rounded_rect_aa(win, 4, iy + 1, SIDEBAR_W - 8, SIDEBAR_ITEM_H - 2, 4, SIDEBAR_SELECTION);
        }

        let icon_y = iy + (SIDEBAR_ITEM_H as i32 - ICON_DISPLAY_SIZE as i32) / 2;
        let bg = if selected { SIDEBAR_SELECTION } else { colors::SIDEBAR_BG() };
        if let Some(pixels) = state.icon_cache.get_or_load(icons::FOLDER_ICON) {
            blit_icon_alpha(win, 10, icon_y as i16, pixels, bg);
        }

        let text_x = 10 + ICON_DISPLAY_SIZE as i32 + 6;
        let text_y = iy + (SIDEBAR_ITEM_H as i32 - 14) / 2;
        let fg = if selected { 0xFFFFFFFF } else { colors::TEXT() };
        label(win, text_x, text_y, name, fg, FontSize::Normal, TextAlign::Left);
    }

    render_file_list(win, state, win_w, win_h);
}

fn render_toolbar(win: u32, state: &AppState, win_w: u32) {
    toolbar(win, 0, 0, win_w, TOOLBAR_H);

    let back_disabled = state.history_pos == 0;
    let back_style = if !back_disabled { ButtonStyle::Plain } else { ButtonStyle::Default };
    let back_state = if !back_disabled { ButtonState::Normal } else { ButtonState::Disabled };
    button(win, 8, NAV_BTN_Y, NAV_BTN_W as u32, NAV_BTN_H as u32, "", back_style, back_state);
    render_nav_icon(win, 8, NAV_BTN_Y, NAV_BTN_W, NAV_BTN_H, &state.nav_icons.back, back_disabled);

    let fwd_disabled = state.history_pos + 1 >= state.history.len();
    let fwd_style = if !fwd_disabled { ButtonStyle::Plain } else { ButtonStyle::Default };
    let fwd_state = if !fwd_disabled { ButtonState::Normal } else { ButtonState::Disabled };
    button(win, 42, NAV_BTN_Y, NAV_BTN_W as u32, NAV_BTN_H as u32, "", fwd_style, fwd_state);
    render_nav_icon(win, 42, NAV_BTN_Y, NAV_BTN_W, NAV_BTN_H, &state.nav_icons.forward, fwd_disabled);

    button(win, 76, NAV_BTN_Y, NAV_BTN_W as u32, NAV_BTN_H as u32, "", ButtonStyle::Plain, ButtonState::Normal);
    render_nav_icon(win, 76, NAV_BTN_Y, NAV_BTN_W, NAV_BTN_H, &state.nav_icons.refresh, false);

    state.path_field.render(win, "Enter path...");

    let mut buf = [0u8; 16];
    let count_str = fmt_item_count(&mut buf, state.entries.len());
    let text_w = count_str.len() as i32 * 7;
    label(win, win_w as i32 - text_w - 12, 10, count_str, colors::TEXT_SECONDARY(), FontSize::Normal, TextAlign::Left);
}

fn render_nav_icon(win: u32, bx: i32, by: i32, bw: i32, bh: i32, icon: &Option<ControlIcon>, disabled: bool) {
    if let Some(ref ic) = icon {
        let ix = bx + (bw - ic.width as i32) / 2;
        let iy = by + (bh - ic.height as i32) / 2;
        let count = (ic.width * ic.height) as usize;
        let tinted = tint_icon(&ic.pixels, count, 0xFFFFFFFF, disabled);
        window::blit_alpha(win, ix as i16, iy as i16, ic.width as u16, ic.height as u16, &tinted[..count]);
    }
}

fn get_extension(name: &str) -> Option<&str> {
    match name.rfind('.') {
        Some(pos) if pos + 1 < name.len() => Some(&name[pos + 1..]),
        _ => None,
    }
}

fn render_file_list(win: u32, state: &mut AppState, win_w: u32, win_h: u32) {
    let list_x = SIDEBAR_W as i32;
    let list_y = TOOLBAR_H as i32;
    let list_w = win_w - SIDEBAR_W;

    // Column header
    window::fill_rect(win, list_x as i16, list_y as i16, list_w as u16, ROW_H as u16, 0xFF333333);
    label(win, list_x + 12 + ICON_SIZE as i32 + 8, list_y + 6, "Name", colors::TEXT(), FontSize::Normal, TextAlign::Left);

    let size_col_x = list_x + list_w as i32 - 100;
    label(win, size_col_x, list_y + 6, "Size", colors::TEXT(), FontSize::Normal, TextAlign::Left);

    divider_h(win, list_x, list_y + ROW_H - 1, list_w);

    // File rows
    let file_area_y = list_y + ROW_H;
    let max_visible = ((win_h as i32 - file_area_y) / ROW_H) as usize;

    for i in 0..state.entries.len().min(max_visible) {
        let entry_idx = i + state.scroll_offset as usize;
        if entry_idx >= state.entries.len() {
            break;
        }
        let entry = &state.entries[entry_idx];
        let ry = file_area_y + i as i32 * ROW_H;

        if state.selected == Some(entry_idx) {
            window::fill_rect(win, list_x as i16, ry as i16, list_w as u16, ROW_H as u16, colors::ACCENT());
        } else if i % 2 == 1 {
            window::fill_rect(win, list_x as i16, ry as i16, list_w as u16, ROW_H as u16, 0xFF252525);
        }

        let text_color = if state.selected == Some(entry_idx) { 0xFFFFFFFF } else { colors::TEXT() };
        let dim_color = if state.selected == Some(entry_idx) { 0xFFDDDDDD } else { colors::TEXT_SECONDARY() };

        let icon_x = list_x + 12;
        let icon_y = ry + (ROW_H - ICON_SIZE as i32) / 2;

        let name = entry.name_str();
        let entry_type = entry.entry_type;
        let app_icon_buf: alloc::string::String;
        let icon_path: &str = if entry_type == TYPE_DIR {
            if name.ends_with(".app") {
                let full = build_full_path(&state.cwd, name);
                app_icon_buf = icons::app_icon_path(&full);
                app_icon_buf.as_str()
            } else {
                icons::FOLDER_ICON
            }
        } else {
            app_icon_buf = icons::app_icon_path(&build_full_path(&state.cwd, name));
            if app_icon_buf.as_str() != icons::DEFAULT_APP_ICON {
                app_icon_buf.as_str()
            } else {
                match get_extension(name) {
                    Some(ext) => state.mimetypes.icon_for_ext(ext),
                    None => icons::DEFAULT_FILE_ICON,
                }
            }
        };

        let row_bg = if state.selected == Some(entry_idx) {
            colors::ACCENT()
        } else if i % 2 == 1 {
            0xFF252525
        } else {
            colors::WINDOW_BG()
        };

        let mut icon_drawn = false;
        if let Some(pixels) = state.icon_cache.get_or_load(icon_path) {
            blit_icon_alpha(win, icon_x as i16, icon_y as i16, pixels, row_bg);
            icon_drawn = true;
        }

        if !icon_drawn {
            let icon_color = if entry_type == TYPE_DIR { colors::ACCENT() } else { colors::SUCCESS() };
            window::fill_rect(win, icon_x as i16, icon_y as i16, ICON_SIZE as u16, ICON_SIZE as u16, icon_color);
        }

        let name_x = icon_x + ICON_SIZE as i32 + 8;
        let display_name = entry.name_str();
        let display_name = if entry_type == TYPE_DIR {
            display_name.strip_suffix(".app").unwrap_or(display_name)
        } else {
            display_name
        };
        label_ellipsis(win, name_x, ry + 6, display_name, text_color, FontSize::Normal,
            (size_col_x - name_x - 8) as u32);

        if entry.entry_type == TYPE_FILE {
            let mut buf = [0u8; 16];
            let size_str = fmt_size(&mut buf, entry.size);
            label(win, size_col_x, ry + 6, size_str, dim_color, FontSize::Normal, TextAlign::Left);
        } else {
            label(win, size_col_x, ry + 6, "--", dim_color, FontSize::Normal, TextAlign::Left);
        }
    }

    // Scrollbar
    let file_area_h = (win_h as i32 - file_area_y).max(0) as u32;
    let content_h = (state.entries.len() as u32) * (ROW_H as u32);
    let scroll_pixels = state.scroll_offset * (ROW_H as u32);
    scrollbar(win, list_x, file_area_y, list_w, file_area_h, content_h, scroll_pixels);
}
