//! Display-list renderer with compositor-driven smooth scrolling.
//!
//! After layout, the tree is flattened into a sorted `Vec<DrawCmd>` (the
//! display list).  Each tile is rasterized by binary-searching for the
//! first command that overlaps the tile Y range, then linearly executing
//! commands until they fall below the tile.  This is O(k) per tile where
//! k = commands visible in the tile, compared to O(n) for a full tree walk.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use libanyui_client::{self as ui, Widget};

use crate::layout::{FormFieldKind, LayoutBox};
use crate::style::{
    BackgroundClipVal, BackgroundImageVal, BackgroundRepeatVal, BackgroundSizeVal, TextDeco,
};

mod cache;
mod display_list;
mod forms;
mod raster;
mod raster_utils;
mod tile;
mod types;

pub use cache::{
    ImageCache, ImageEntry, PROGRESSIVE_BAND_VIEWPORTS_AFTER, PROGRESSIVE_BAND_VIEWPORTS_BEFORE,
};
pub use raster::parse_color_value;
pub use types::{FormControl, HitKind};

use raster::{parse_date_value, parse_time_value, rasterize_draw_cmd, rasterize_masked_cmd};
use raster_utils::{
    alpha_blend, cos_approx, darken_color, interpolate_gradient_color, lighten_color,
    resolve_axis_origin, sin_approx,
};
use tile::{
    TileCache, TileCanvas, BUFFER_ZONE, INITIAL_VISIBLE_EXTRA_ROWS, MAX_TILES_PER_IDLE_TICK,
    MAX_TILES_PER_SCROLL_TICK, MAX_TILE_CANVASES, MAX_TILE_CANVAS_CREATES_PER_IDLE_TICK,
    MAX_TILE_CANVAS_CREATES_PER_SCROLL_TICK, TILE_HEIGHT,
};
use types::{
    DisplayList, DrawCmd, DrawKind, DrawRotation, HitRegion, MaskLayer, RoundedClip, StickyContext,
};

pub(crate) struct Renderer {
    tile_canvases: Vec<TileCanvas>,
    tile_cache: TileCache,
    doc_w: u32,
    doc_h: u32,
    pub hit_regions: Vec<HitRegion>,
    pub form_controls: Vec<FormControl>,
    pub link_map: Vec<(u32, String)>,
    link_cb: Option<ui::Callback>,
    link_cb_ud: u64,
    submit_cb: Option<ui::Callback>,
    submit_cb_ud: u64,
    last_scroll_y: i32,
    /// The display list — built once after layout, used for all tile rasterization.
    display_list: DisplayList,
    display_list_complete: bool,
    display_list_y_range: Option<(i32, i32)>,
}

include!("runtime/core.rs");
include!("runtime/hit_test.rs");
include!("runtime/rendering.rs");
include!("runtime/tiles.rs");
