//! UI helpers — card creation, detail view population, category icons.

use alloc::string::String;
use alloc::vec::Vec;
use libanyui_client as ui;
use ui::Widget;
use crate::apkg::{PackageInfo, PkgStatus, format_size};

// ─── Category → Tabler Icon Mapping ────────────────────────────────

/// Return a Tabler icon name for a package category.
pub fn category_icon(category: &str) -> &'static str {
    match category {
        "system"    => "settings",
        "network"   => "network",
        "dev"       => "code",
        "editor"    => "edit",
        "media"     => "player-play",
        "graphics"  => "palette",
        "games"     => "device-gamepad-2",
        "utils"     => "tool",
        "lib"       => "package",
        "security"  => "shield-lock",
        "shell"     => "terminal-2",
        "web"       => "world",
        "font"      => "typography",
        "driver"    => "cpu",
        _           => "package",
    }
}

/// Return a distinctive ARGB color for a package category icon.
pub fn category_color(category: &str) -> u32 {
    match category {
        "system"    => 0xFF64D2FF,
        "network"   => 0xFF5AC8FA,
        "dev"       => 0xFFFF9F0A,
        "editor"    => 0xFFBF5AF2,
        "media"     => 0xFFFF375F,
        "graphics"  => 0xFF30D158,
        "games"     => 0xFFFF6482,
        "utils"     => 0xFFFFD60A,
        "lib"       => 0xFF64D2FF,
        "security"  => 0xFF30D158,
        "shell"     => 0xFF98989D,
        "web"       => 0xFF5AC8FA,
        "font"      => 0xFFAC8E68,
        "driver"    => 0xFF98989D,
        _           => 0xFF8E8E93,
    }
}

// ─── Card Creation ─────────────────────────────────────────────────

/// Width and height of a package card in the grid.
pub const CARD_W: u32 = 280;
pub const CARD_H: u32 = 140;

/// Create a package card and add it to the given parent container.
/// Returns the action button so the caller can wire the on_click event.
pub fn create_card(
    parent: &ui::FlowPanel,
    pkg: &PackageInfo,
    status: PkgStatus,
    index: usize,
) -> ui::Button {
    let tc = ui::theme::colors();

    let card = ui::Card::new();
    card.set_size(CARD_W, CARD_H);
    card.set_margin(6, 6, 6, 6);
    parent.add(&card);

    // Category icon (top-left)
    let icon_name = category_icon(&pkg.category);
    let icon_color = category_color(&pkg.category);
    let icon_btn = ui::IconButton::new("");
    icon_btn.set_position(12, 12);
    icon_btn.set_size(36, 36);
    icon_btn.set_system_icon(icon_name, ui::IconType::Outline, icon_color, 28);
    icon_btn.set_enabled(false); // decorative only
    card.add(&icon_btn);

    // App name (bold)
    let name_label = ui::Label::new(&pkg.name);
    name_label.set_position(56, 12);
    name_label.set_size(CARD_W - 68, 20);
    name_label.set_font(1); // SF Pro Bold
    name_label.set_text_color(tc.text);
    card.add(&name_label);

    // Version
    let ver_label = ui::Label::new(&pkg.version);
    ver_label.set_position(56, 32);
    ver_label.set_size(100, 16);
    ver_label.set_font_size(11);
    ver_label.set_text_color(tc.text_secondary);
    card.add(&ver_label);

    // Description (2-line area)
    let desc = if pkg.description.len() > 80 {
        let mut s = String::from(&pkg.description[..77]);
        s.push_str("...");
        s
    } else {
        pkg.description.clone()
    };
    let desc_label = ui::Label::new(&desc);
    desc_label.set_position(12, 56);
    desc_label.set_size(CARD_W - 24, 32);
    desc_label.set_font_size(11);
    desc_label.set_text_color(tc.text_secondary);
    card.add(&desc_label);

    // Category tag (bottom-left)
    let tag = ui::Tag::new(&pkg.category);
    tag.set_position(12, CARD_H as i32 - 34);
    tag.set_size(80, 22);
    card.add(&tag);

    // Size label (bottom-center)
    let size_str = format_size(pkg.size);
    let size_label = ui::Label::new(&size_str);
    size_label.set_position(100, CARD_H as i32 - 32);
    size_label.set_size(80, 18);
    size_label.set_font_size(10);
    size_label.set_text_color(tc.text_disabled);
    card.add(&size_label);

    // Action button (bottom-right)
    let (btn_text, btn_color) = match status {
        PkgStatus::Available => ("Get", tc.accent),
        PkgStatus::Installed => ("Installed", tc.success),
        PkgStatus::Updatable => ("Update", tc.warning),
    };
    let action_btn = ui::Button::new(btn_text);
    action_btn.set_position(CARD_W as i32 - 88, CARD_H as i32 - 38);
    action_btn.set_size(76, 28);
    action_btn.set_color(btn_color);
    action_btn.set_text_color(0xFFFFFFFF);
    action_btn.set_font_size(12);
    if status == PkgStatus::Installed {
        action_btn.set_enabled(false);
    }
    card.add(&action_btn);

    action_btn
}

// ─── Detail View ───────────────────────────────────────────────────

/// Populate the detail view with info about the given package.
pub fn populate_detail(
    detail: &ui::View,
    pkg: &PackageInfo,
    status: PkgStatus,
) {
    let tc = ui::theme::colors();

    // Clear existing children (remove all, then re-add)
    // We build a fresh set of controls each time.

    // Category icon (large)
    let icon_name = category_icon(&pkg.category);
    let icon_color = category_color(&pkg.category);
    let icon_btn = ui::IconButton::new("");
    icon_btn.set_position(24, 16);
    icon_btn.set_size(56, 56);
    icon_btn.set_system_icon(icon_name, ui::IconType::Outline, icon_color, 48);
    icon_btn.set_enabled(false);
    detail.add(&icon_btn);

    // App name (large, bold)
    let name_label = ui::Label::new(&pkg.name);
    name_label.set_position(92, 16);
    name_label.set_size(400, 28);
    name_label.set_font(1);
    name_label.set_font_size(20);
    name_label.set_text_color(tc.text);
    detail.add(&name_label);

    // Category + version subtitle
    let subtitle = alloc::format!("{}  •  v{}", pkg.category, pkg.version);
    let sub_label = ui::Label::new(&subtitle);
    sub_label.set_position(92, 46);
    sub_label.set_size(400, 18);
    sub_label.set_font_size(12);
    sub_label.set_text_color(tc.text_secondary);
    detail.add(&sub_label);

    // Divider
    let div = ui::Divider::new();
    div.set_position(24, 84);
    div.set_size(600, 1);
    detail.add(&div);

    // Info grid (left column)
    let mut y = 100;
    let pairs: Vec<(&str, String)> = alloc::vec![
        ("Type:", String::from(&pkg.pkg_type)),
        ("Size:", format_size(pkg.size)),
        ("Installed Size:", format_size(pkg.size_installed)),
    ];
    for (label, value) in &pairs {
        let lbl = ui::Label::new(label);
        lbl.set_position(24, y);
        lbl.set_size(120, 20);
        lbl.set_text_color(tc.text_secondary);
        lbl.set_font_size(12);
        detail.add(&lbl);

        let val = ui::Label::new(value);
        val.set_position(150, y);
        val.set_size(200, 20);
        val.set_text_color(tc.text);
        val.set_font_size(12);
        detail.add(&val);
        y += 24;
    }

    // Dependencies
    if !pkg.depends.is_empty() {
        let dep_label = ui::Label::new("Dependencies:");
        dep_label.set_position(24, y);
        dep_label.set_size(120, 20);
        dep_label.set_text_color(tc.text_secondary);
        dep_label.set_font_size(12);
        detail.add(&dep_label);

        let deps_str = pkg.depends.join(", ");
        let deps_val = ui::Label::new(&deps_str);
        deps_val.set_position(150, y);
        deps_val.set_size(450, 20);
        deps_val.set_text_color(tc.text);
        deps_val.set_font_size(12);
        detail.add(&deps_val);
        y += 24;
    }

    // Description section
    y += 8;
    let desc_header = ui::Label::new("Description");
    desc_header.set_position(24, y);
    desc_header.set_size(200, 22);
    desc_header.set_font(1);
    desc_header.set_font_size(14);
    desc_header.set_text_color(tc.text);
    detail.add(&desc_header);
    y += 26;

    let desc_text = ui::Label::new(&pkg.description);
    desc_text.set_position(24, y);
    desc_text.set_size(600, 80);
    desc_text.set_font_size(12);
    desc_text.set_text_color(tc.text_secondary);
    detail.add(&desc_text);
}
