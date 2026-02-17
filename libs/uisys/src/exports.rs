//! UisysExports — the C ABI export struct placed at byte 0 of the DLL.
//!
//! This struct IS the public ABI. New functions are appended, never reordered.

use crate::components::*;
use crate::draw;

/// The export table. Must be `#[repr(C)]` and placed in `.exports` section.
#[repr(C)]
pub struct UisysExports {
    // Header
    pub magic: [u8; 4],
    pub version: u32,
    pub num_exports: u32,
    /// Current theme: 0 = dark (default), 1 = light.
    /// Written by compositor (DLL pages are shared physical frames).
    /// Read by theme.rs via volatile pointer (no syscall needed).
    pub theme: u32,
    pub _reserved: [u32; 4],

    // --- Label (4) ---
    pub label_render: extern "C" fn(u32, i32, i32, *const u8, u32, u32, u16, u8),
    pub label_measure: extern "C" fn(*const u8, u32, u16, *mut u32, *mut u32),
    pub label_render_ellipsis: extern "C" fn(u32, i32, i32, *const u8, u32, u32, u16, u32),
    pub label_render_multiline: extern "C" fn(u32, i32, i32, *const u8, u32, u32, u16, u32, u32),

    // --- Button (3) ---
    pub button_render: extern "C" fn(u32, i32, i32, u32, u32, *const u8, u32, u8, u8),
    pub button_hit_test: extern "C" fn(i32, i32, u32, u32, i32, i32) -> u32,
    pub button_measure: extern "C" fn(*const u8, u32, *mut u32, *mut u32),

    // --- Toggle (2) ---
    pub toggle_render: extern "C" fn(u32, i32, i32, u32),
    pub toggle_hit_test: extern "C" fn(i32, i32, i32, i32) -> u32,

    // --- Checkbox (2) ---
    pub checkbox_render: extern "C" fn(u32, i32, i32, u8, *const u8, u32),
    pub checkbox_hit_test: extern "C" fn(i32, i32, i32, i32) -> u32,

    // --- TextField (3) ---
    pub textfield_render: extern "C" fn(u32, i32, i32, u32, u32, *const u8, u32, *const u8, u32, u32, u32),
    pub textfield_hit_test: extern "C" fn(i32, i32, u32, u32, i32, i32) -> u32,
    pub textfield_cursor_from_click: extern "C" fn(i32, u32, i32) -> u32,

    // --- Badge (1) ---
    pub badge_render: extern "C" fn(u32, i32, i32, u32, u32),

    // --- Tag (3) ---
    pub tag_render: extern "C" fn(u32, i32, i32, *const u8, u32, u32, u32, u32),
    pub tag_hit_test: extern "C" fn(i32, i32, u32, u32, i32, i32) -> u32,
    pub tag_close_hit_test: extern "C" fn(i32, i32, u32, i32, i32) -> u32,

    // --- RadioButton (2) ---
    pub radio_render: extern "C" fn(u32, i32, i32, u32, *const u8, u32),
    pub radio_hit_test: extern "C" fn(i32, i32, i32, i32) -> u32,

    // --- Slider (3) ---
    pub slider_render: extern "C" fn(u32, i32, i32, u32, u32, u32, u32, u32),
    pub slider_hit_test: extern "C" fn(i32, i32, u32, u32, i32, i32) -> u32,
    pub slider_value_from_x: extern "C" fn(i32, u32, u32, u32, i32) -> u32,

    // --- ProgressBar (1) ---
    pub progress_render: extern "C" fn(u32, i32, i32, u32, u32, u32, u32),

    // --- Stepper (3) ---
    pub stepper_render: extern "C" fn(u32, i32, i32, i32, i32, i32),
    pub stepper_hit_test_plus: extern "C" fn(i32, i32, i32, i32) -> u32,
    pub stepper_hit_test_minus: extern "C" fn(i32, i32, i32, i32) -> u32,

    // --- SegmentedControl (2) ---
    pub segmented_render: extern "C" fn(u32, i32, i32, u32, u32, *const u8, u32, *const u32, u32),
    pub segmented_hit_test: extern "C" fn(i32, i32, u32, u32, u32, i32, i32) -> u32,

    // --- TableView (4) ---
    pub tableview_render: extern "C" fn(u32, i32, i32, u32, u32, u32, u32, u32, u32),
    pub tableview_render_row: extern "C" fn(u32, i32, i32, u32, u32, *const u8, u32, u32, u32),
    pub tableview_hit_test_row: extern "C" fn(i32, u32, u32, i32) -> u32,
    pub tableview_render_header: extern "C" fn(u32, i32, i32, u32, u32, *const u8, u32),

    // --- ScrollView (3) ---
    pub scrollview_render_scrollbar: extern "C" fn(u32, i32, i32, u32, u32, u32, u32),
    pub scrollview_hit_test_scrollbar: extern "C" fn(i32, i32, u32, u32, i32, i32) -> u32,
    pub scrollview_thumb_pos: extern "C" fn(u32, u32, u32) -> u64,

    // --- Sidebar (4) ---
    pub sidebar_render_bg: extern "C" fn(u32, i32, i32, u32, u32),
    pub sidebar_render_item: extern "C" fn(u32, i32, i32, u32, *const u8, u32, u32),
    pub sidebar_render_header: extern "C" fn(u32, i32, i32, u32, *const u8, u32),
    pub sidebar_hit_test_item: extern "C" fn(i32, i32, u32, u32, i32, i32) -> u32,

    // --- NavigationBar (2) ---
    pub navbar_render: extern "C" fn(u32, i32, i32, u32, *const u8, u32, u32),
    pub navbar_hit_test_back: extern "C" fn(i32, i32, i32, i32) -> u32,

    // --- TabBar (2) ---
    pub tabbar_render: extern "C" fn(u32, i32, i32, u32, u32, *const u8, u32, *const u32, u32),
    pub tabbar_hit_test: extern "C" fn(i32, i32, u32, u32, u32, i32, i32) -> u32,

    // --- Toolbar (2) ---
    pub toolbar_render: extern "C" fn(u32, i32, i32, u32, u32),
    pub toolbar_render_button: extern "C" fn(u32, i32, i32, u32, u32, *const u8, u32, u8),

    // --- Card (1) ---
    pub card_render: extern "C" fn(u32, i32, i32, u32, u32),

    // --- GroupBox (1) ---
    pub groupbox_render: extern "C" fn(u32, i32, i32, u32, u32, *const u8, u32),

    // --- SplitView (3) ---
    pub splitview_render: extern "C" fn(u32, i32, i32, u32, u32, u32),
    pub splitview_hit_test_divider: extern "C" fn(i32, i32, u32, u32, u32, i32, i32) -> u32,
    pub splitview_clamp: extern "C" fn(u32, u32, u32, u32) -> u32,

    // --- Divider (2) ---
    pub divider_render_h: extern "C" fn(u32, i32, i32, u32),
    pub divider_render_v: extern "C" fn(u32, i32, i32, u32),

    // --- Alert (2) ---
    pub alert_render: extern "C" fn(u32, i32, i32, u32, u32, *const u8, u32, *const u8, u32),
    pub alert_render_button: extern "C" fn(u32, i32, i32, u32, u32, *const u8, u32, u8, u8),

    // --- ContextMenu (4) ---
    pub contextmenu_render_bg: extern "C" fn(u32, i32, i32, u32, u32),
    pub contextmenu_render_item: extern "C" fn(u32, i32, i32, u32, *const u8, u32, u32),
    pub contextmenu_render_separator: extern "C" fn(u32, i32, i32, u32),
    pub contextmenu_hit_test_item: extern "C" fn(i32, i32, u32, u32, i32, i32) -> u32,

    // --- Tooltip (1) ---
    pub tooltip_render: extern "C" fn(u32, i32, i32, *const u8, u32),

    // --- ImageView (1) ---
    pub imageview_render: extern "C" fn(u32, i32, i32, u32, u32, *const u32, u32, u32),

    // --- StatusIndicator (1) ---
    pub status_render: extern "C" fn(u32, i32, i32, u8, *const u8, u32, u16),

    // --- ColorWell (1) ---
    pub colorwell_render: extern "C" fn(u32, i32, i32, u32, u32),

    // --- SearchField (2) ---
    pub searchfield_render: extern "C" fn(u32, i32, i32, u32, u32, *const u8, u32, u32, u32),
    pub searchfield_hit_test: extern "C" fn(i32, i32, u32, u32, i32, i32) -> u32,

    // --- TextArea (2) ---
    pub textarea_render: extern "C" fn(u32, i32, i32, u32, u32, *const u8, u32, u32, u32, u32, u32),
    pub textarea_hit_test: extern "C" fn(i32, i32, u32, u32, i32, i32) -> u32,

    // --- IconButton (2) ---
    pub iconbutton_render: extern "C" fn(u32, i32, i32, u32, u8, u32),
    pub iconbutton_hit_test: extern "C" fn(i32, i32, u32, i32, i32) -> u32,

    // --- v2 exports (4) ---
    pub gpu_has_accel: extern "C" fn() -> u32,
    pub fill_rounded_rect_aa: extern "C" fn(u32, i32, i32, u32, u32, u32, u32),
    pub draw_text_with_font: extern "C" fn(u32, i32, i32, u32, u32, u16, *const u8, u32),
    pub font_measure: extern "C" fn(u32, u16, *const u8, u32, *mut u32, *mut u32) -> u32,

    // --- TextField v2 (2) ---
    pub textfield_render_ex: extern "C" fn(u32, i32, i32, u32, u32, *const u8, u32, *const u8, u32, u32, u32, u32, u32),
    pub textfield_cursor_from_click_ex: extern "C" fn(i32, *const u8, u32, i32, i32) -> u32,
}

// Total export function count
const NUM_EXPORTS: u32 = 79;

/// The global export instance, placed at the very start of the binary.
#[link_section = ".exports"]
#[used]
#[no_mangle]
pub static UISYS_EXPORTS: UisysExports = UisysExports {
    magic: *b"DLIB",
    version: 1,
    num_exports: NUM_EXPORTS,
    theme: 0,
    _reserved: [0; 4],

    // Label
    label_render: label::label_render,
    label_measure: label::label_measure,
    label_render_ellipsis: label::label_render_ellipsis,
    label_render_multiline: label::label_render_multiline,

    // Button
    button_render: button::button_render,
    button_hit_test: button::button_hit_test,
    button_measure: button::button_measure,

    // Toggle
    toggle_render: toggle::toggle_render,
    toggle_hit_test: toggle::toggle_hit_test,

    // Checkbox
    checkbox_render: checkbox::checkbox_render,
    checkbox_hit_test: checkbox::checkbox_hit_test,

    // TextField
    textfield_render: textfield::textfield_render,
    textfield_hit_test: textfield::textfield_hit_test,
    textfield_cursor_from_click: textfield::textfield_cursor_from_click,

    // Badge
    badge_render: badge::badge_render,

    // Tag
    tag_render: tag::tag_render,
    tag_hit_test: tag::tag_hit_test,
    tag_close_hit_test: tag::tag_close_hit_test,

    // RadioButton
    radio_render: radio::radio_render,
    radio_hit_test: radio::radio_hit_test,

    // Slider
    slider_render: slider::slider_render,
    slider_hit_test: slider::slider_hit_test,
    slider_value_from_x: slider::slider_value_from_x,

    // ProgressBar
    progress_render: progress::progress_render,

    // Stepper
    stepper_render: stepper::stepper_render,
    stepper_hit_test_plus: stepper::stepper_hit_test_plus,
    stepper_hit_test_minus: stepper::stepper_hit_test_minus,

    // SegmentedControl
    segmented_render: segmented::segmented_render,
    segmented_hit_test: segmented::segmented_hit_test,

    // TableView
    tableview_render: tableview::tableview_render,
    tableview_render_row: tableview::tableview_render_row,
    tableview_hit_test_row: tableview::tableview_hit_test_row,
    tableview_render_header: tableview::tableview_render_header,

    // ScrollView
    scrollview_render_scrollbar: scrollview::scrollview_render_scrollbar,
    scrollview_hit_test_scrollbar: scrollview::scrollview_hit_test_scrollbar,
    scrollview_thumb_pos: scrollview::scrollview_thumb_pos,

    // Sidebar
    sidebar_render_bg: sidebar::sidebar_render_bg,
    sidebar_render_item: sidebar::sidebar_render_item,
    sidebar_render_header: sidebar::sidebar_render_header,
    sidebar_hit_test_item: sidebar::sidebar_hit_test_item,

    // NavigationBar
    navbar_render: navbar::navbar_render,
    navbar_hit_test_back: navbar::navbar_hit_test_back,

    // TabBar
    tabbar_render: tabbar::tabbar_render,
    tabbar_hit_test: tabbar::tabbar_hit_test,

    // Toolbar
    toolbar_render: toolbar::toolbar_render,
    toolbar_render_button: toolbar::toolbar_render_button,

    // Card
    card_render: card::card_render,

    // GroupBox
    groupbox_render: groupbox::groupbox_render,

    // SplitView
    splitview_render: splitview::splitview_render,
    splitview_hit_test_divider: splitview::splitview_hit_test_divider,
    splitview_clamp: splitview::splitview_clamp,

    // Divider
    divider_render_h: divider::divider_render_h,
    divider_render_v: divider::divider_render_v,

    // Alert
    alert_render: alert::alert_render,
    alert_render_button: alert::alert_render_button,

    // ContextMenu
    contextmenu_render_bg: contextmenu::contextmenu_render_bg,
    contextmenu_render_item: contextmenu::contextmenu_render_item,
    contextmenu_render_separator: contextmenu::contextmenu_render_separator,
    contextmenu_hit_test_item: contextmenu::contextmenu_hit_test_item,

    // Tooltip
    tooltip_render: tooltip::tooltip_render,

    // ImageView
    imageview_render: imageview::imageview_render,

    // StatusIndicator
    status_render: status::status_render,

    // ColorWell
    colorwell_render: colorwell::colorwell_render,

    // SearchField
    searchfield_render: searchfield::searchfield_render,
    searchfield_hit_test: searchfield::searchfield_hit_test,

    // TextArea
    textarea_render: textarea::textarea_render,
    textarea_hit_test: textarea::textarea_hit_test,

    // IconButton
    iconbutton_render: iconbutton::iconbutton_render,
    iconbutton_hit_test: iconbutton::iconbutton_hit_test,

    // v2 exports
    gpu_has_accel: draw::gpu_has_accel_export,
    fill_rounded_rect_aa: draw::fill_rounded_rect_aa,
    draw_text_with_font: draw::draw_text_with_font,
    font_measure: draw::font_measure_export,

    // TextField v2
    textfield_render_ex: textfield::textfield_render_ex,
    textfield_cursor_from_click_ex: textfield::textfield_cursor_from_click_ex,
};
