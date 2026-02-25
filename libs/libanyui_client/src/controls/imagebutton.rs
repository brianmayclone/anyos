//! ImageButton — a clickable button that displays an image from file (PNG/ICO).
//!
//! Composed client-side from a View + ImageView.  The View handles click events
//! and provides the button background; the ImageView renders the icon.

use crate::{Control, Widget, lib, events, KIND_VIEW, KIND_IMAGE_VIEW};
use crate::events::ClickEvent;
use crate::controls::imageview::SCALE_FIT;

/// A clickable image button.  Behaves like a toolbar button but displays
/// a raster image (PNG, ICO, BMP, …) instead of a vector icon.
#[derive(Clone, Copy)]
pub struct ImageButton {
    /// The outer View that receives clicks.
    view_id: u32,
    /// The inner ImageView that displays the icon.
    iv_id: u32,
}

impl ImageButton {
    /// Create an ImageButton with the given outer size.
    pub fn new(w: u32, h: u32) -> Self {
        // Outer container — acts as the click target
        let view_id = (lib().create_control)(KIND_VIEW, core::ptr::null(), 0);
        (lib().set_size)(view_id, w, h);

        // Inner image — fills the view, centered via margin
        let iv_id = (lib().create_control)(KIND_IMAGE_VIEW, core::ptr::null(), 0);
        (lib().set_size)(iv_id, w, h);
        (lib().set_dock)(iv_id, crate::DOCK_FILL);
        (lib().imageview_set_scale_mode)(iv_id, SCALE_FIT);

        (lib().add_child)(view_id, iv_id);

        Self { view_id, iv_id }
    }

    /// Load an image from a file path (PNG, ICO, BMP, JPEG, GIF).
    pub fn load_file(&self, path: &str) {
        if let Ok(data) = anyos_std::fs::read_to_vec(path) {
            self.load_bytes(&data);
        }
    }

    /// Load an image from raw encoded bytes.
    pub fn load_bytes(&self, data: &[u8]) {
        if let Some(info) = libimage_client::probe(data) {
            let pixel_count = (info.width as usize) * (info.height as usize);
            let mut pixels = alloc::vec![0u32; pixel_count];
            let mut scratch = alloc::vec![0u8; info.scratch_needed as usize];
            if libimage_client::decode(data, &mut pixels, &mut scratch).is_ok() {
                (lib().imageview_set_pixels)(self.iv_id, pixels.as_ptr(), info.width, info.height);
            }
        }
    }

    /// Load an ICO file at a specific preferred size.
    pub fn load_ico(&self, path: &str, preferred_size: u32) {
        if let Ok(data) = anyos_std::fs::read_to_vec(path) {
            if let Some(info) = libimage_client::probe_ico_size(&data, preferred_size) {
                let pixel_count = (info.width as usize) * (info.height as usize);
                let mut pixels = alloc::vec![0u32; pixel_count];
                let mut scratch = alloc::vec![0u8; info.scratch_needed as usize];
                if libimage_client::decode_ico_size(&data, preferred_size, &mut pixels, &mut scratch).is_ok() {
                    (lib().imageview_set_pixels)(self.iv_id, pixels.as_ptr(), info.width, info.height);
                }
            }
        }
    }

    /// Set raw ARGB pixel data directly.
    pub fn set_pixels(&self, pixels: &[u32], w: u32, h: u32) {
        (lib().imageview_set_pixels)(self.iv_id, pixels.as_ptr(), w, h);
    }

    /// Register a click callback.
    pub fn on_click(&self, mut f: impl FnMut(&ClickEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| f(&ClickEvent { id }));
        (lib().on_click_fn)(self.view_id, thunk, ud);
    }

    /// Register a click callback with raw extern "C" function + userdata.
    pub fn on_click_raw(&self, cb: crate::Callback, userdata: u64) {
        (lib().on_click_fn)(self.view_id, cb, userdata);
    }
}

impl Widget for ImageButton {
    fn id(&self) -> u32 { self.view_id }
}

impl core::ops::Deref for ImageButton {
    type Target = Control;
    fn deref(&self) -> &Control {
        // SAFETY: Control is a #[repr(transparent)] wrapper around u32,
        // and view_id is the first field.
        unsafe { &*(self as *const Self as *const Control) }
    }
}
