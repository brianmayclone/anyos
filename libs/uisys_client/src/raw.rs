//! Raw export struct definition — must exactly match the DLL's UisysExports layout.

/// Base virtual address where uisys.dlib is mapped.
pub const UISYS_BASE: usize = 0x0400_0000;

#[repr(C)]
pub struct UisysExports {
    pub magic: [u8; 4],
    pub version: u32,
    pub num_exports: u32,
    pub theme: u32,
    pub _reserved: [u32; 4],

    // Label (4)
    pub label_render: extern "C" fn(u32, i32, i32, *const u8, u32, u32, u16, u8),
    pub label_measure: extern "C" fn(*const u8, u32, u16, *mut u32, *mut u32),
    pub label_render_ellipsis: extern "C" fn(u32, i32, i32, *const u8, u32, u32, u16, u32),
    pub label_render_multiline: extern "C" fn(u32, i32, i32, *const u8, u32, u32, u16, u32, u32),

    // Button (3)
    pub button_render: extern "C" fn(u32, i32, i32, u32, u32, *const u8, u32, u8, u8),
    pub button_hit_test: extern "C" fn(i32, i32, u32, u32, i32, i32) -> u32,
    pub button_measure: extern "C" fn(*const u8, u32, *mut u32, *mut u32),

    // Toggle (2)
    pub toggle_render: extern "C" fn(u32, i32, i32, u32),
    pub toggle_hit_test: extern "C" fn(i32, i32, i32, i32) -> u32,

    // Checkbox (2)
    pub checkbox_render: extern "C" fn(u32, i32, i32, u8, *const u8, u32),
    pub checkbox_hit_test: extern "C" fn(i32, i32, i32, i32) -> u32,

    // TextField (3)
    pub textfield_render: extern "C" fn(u32, i32, i32, u32, u32, *const u8, u32, *const u8, u32, u32, u32),
    pub textfield_hit_test: extern "C" fn(i32, i32, u32, u32, i32, i32) -> u32,
    pub textfield_cursor_from_click: extern "C" fn(i32, u32, i32) -> u32,

    // Badge (1)
    pub badge_render: extern "C" fn(u32, i32, i32, u32, u32),

    // Tag (3)
    pub tag_render: extern "C" fn(u32, i32, i32, *const u8, u32, u32, u32, u32),
    pub tag_hit_test: extern "C" fn(i32, i32, u32, u32, i32, i32) -> u32,
    pub tag_close_hit_test: extern "C" fn(i32, i32, u32, i32, i32) -> u32,

    // RadioButton (2)
    pub radio_render: extern "C" fn(u32, i32, i32, u32, *const u8, u32),
    pub radio_hit_test: extern "C" fn(i32, i32, i32, i32) -> u32,

    // Slider (3)
    pub slider_render: extern "C" fn(u32, i32, i32, u32, u32, u32, u32, u32),
    pub slider_hit_test: extern "C" fn(i32, i32, u32, u32, i32, i32) -> u32,
    pub slider_value_from_x: extern "C" fn(i32, u32, u32, u32, i32) -> u32,

    // ProgressBar (1)
    pub progress_render: extern "C" fn(u32, i32, i32, u32, u32, u32, u32),

    // Stepper (3)
    pub stepper_render: extern "C" fn(u32, i32, i32, i32, i32, i32),
    pub stepper_hit_test_plus: extern "C" fn(i32, i32, i32, i32) -> u32,
    pub stepper_hit_test_minus: extern "C" fn(i32, i32, i32, i32) -> u32,

    // SegmentedControl (2)
    pub segmented_render: extern "C" fn(u32, i32, i32, u32, u32, *const u8, u32, *const u32, u32),
    pub segmented_hit_test: extern "C" fn(i32, i32, u32, u32, u32, i32, i32) -> u32,

    // TableView (4)
    pub tableview_render: extern "C" fn(u32, i32, i32, u32, u32, u32, u32, u32, u32),
    pub tableview_render_row: extern "C" fn(u32, i32, i32, u32, u32, *const u8, u32, u32, u32),
    pub tableview_hit_test_row: extern "C" fn(i32, u32, u32, i32) -> u32,
    pub tableview_render_header: extern "C" fn(u32, i32, i32, u32, u32, *const u8, u32),

    // ScrollView (3)
    pub scrollview_render_scrollbar: extern "C" fn(u32, i32, i32, u32, u32, u32, u32),
    pub scrollview_hit_test_scrollbar: extern "C" fn(i32, i32, u32, u32, i32, i32) -> u32,
    pub scrollview_thumb_pos: extern "C" fn(u32, u32, u32) -> u64,

    // Sidebar (4)
    pub sidebar_render_bg: extern "C" fn(u32, i32, i32, u32, u32),
    pub sidebar_render_item: extern "C" fn(u32, i32, i32, u32, *const u8, u32, u32),
    pub sidebar_render_header: extern "C" fn(u32, i32, i32, u32, *const u8, u32),
    pub sidebar_hit_test_item: extern "C" fn(i32, i32, u32, u32, i32, i32) -> u32,

    // NavigationBar (2)
    pub navbar_render: extern "C" fn(u32, i32, i32, u32, *const u8, u32, u32),
    pub navbar_hit_test_back: extern "C" fn(i32, i32, i32, i32) -> u32,

    // TabBar (2)
    pub tabbar_render: extern "C" fn(u32, i32, i32, u32, u32, *const u8, u32, *const u32, u32),
    pub tabbar_hit_test: extern "C" fn(i32, i32, u32, u32, u32, i32, i32) -> u32,

    // Toolbar (2)
    pub toolbar_render: extern "C" fn(u32, i32, i32, u32, u32),
    pub toolbar_render_button: extern "C" fn(u32, i32, i32, u32, u32, *const u8, u32, u8),

    // Card (1)
    pub card_render: extern "C" fn(u32, i32, i32, u32, u32),

    // GroupBox (1)
    pub groupbox_render: extern "C" fn(u32, i32, i32, u32, u32, *const u8, u32),

    // SplitView (3)
    pub splitview_render: extern "C" fn(u32, i32, i32, u32, u32, u32),
    pub splitview_hit_test_divider: extern "C" fn(i32, i32, u32, u32, u32, i32, i32) -> u32,
    pub splitview_clamp: extern "C" fn(u32, u32, u32, u32) -> u32,

    // Divider (2)
    pub divider_render_h: extern "C" fn(u32, i32, i32, u32),
    pub divider_render_v: extern "C" fn(u32, i32, i32, u32),

    // Alert (2)
    pub alert_render: extern "C" fn(u32, i32, i32, u32, u32, *const u8, u32, *const u8, u32),
    pub alert_render_button: extern "C" fn(u32, i32, i32, u32, u32, *const u8, u32, u8, u8),

    // ContextMenu (4)
    pub contextmenu_render_bg: extern "C" fn(u32, i32, i32, u32, u32),
    pub contextmenu_render_item: extern "C" fn(u32, i32, i32, u32, *const u8, u32, u32),
    pub contextmenu_render_separator: extern "C" fn(u32, i32, i32, u32),
    pub contextmenu_hit_test_item: extern "C" fn(i32, i32, u32, u32, i32, i32) -> u32,

    // Tooltip (1)
    pub tooltip_render: extern "C" fn(u32, i32, i32, *const u8, u32),

    // ImageView (1)
    pub imageview_render: extern "C" fn(u32, i32, i32, u32, u32, *const u32, u32, u32),

    // StatusIndicator (1)
    pub status_render: extern "C" fn(u32, i32, i32, u8, *const u8, u32, u16),

    // ColorWell (1)
    pub colorwell_render: extern "C" fn(u32, i32, i32, u32, u32),

    // SearchField (2)
    pub searchfield_render: extern "C" fn(u32, i32, i32, u32, u32, *const u8, u32, u32, u32),
    pub searchfield_hit_test: extern "C" fn(i32, i32, u32, u32, i32, i32) -> u32,

    // TextArea (2)
    pub textarea_render: extern "C" fn(u32, i32, i32, u32, u32, *const u8, u32, u32, u32, u32, u32),
    pub textarea_hit_test: extern "C" fn(i32, i32, u32, u32, i32, i32) -> u32,

    // IconButton (2)
    pub iconbutton_render: extern "C" fn(u32, i32, i32, u32, u8, u32),
    pub iconbutton_hit_test: extern "C" fn(i32, i32, u32, i32, i32) -> u32,

    // v2 exports (4)
    pub gpu_has_accel: extern "C" fn() -> u32,
    pub fill_rounded_rect_aa: extern "C" fn(u32, i32, i32, u32, u32, u32, u32),
    pub draw_text_with_font: extern "C" fn(u32, i32, i32, u32, u32, u16, *const u8, u32),
    pub font_measure: extern "C" fn(u32, u16, *const u8, u32, *mut u32, *mut u32) -> u32,
}

/// Get a reference to the export table at the fixed DLL base address.
#[inline(always)]
pub fn exports() -> &'static UisysExports {
    unsafe { &*(UISYS_BASE as *const UisysExports) }
}
