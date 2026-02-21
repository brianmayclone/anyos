use alloc::vec;
use alloc::vec::Vec;
use crate::{Control, Widget, lib, KIND_IMAGE_VIEW};

/// Scale mode constants (must match server-side values).
pub const SCALE_NONE: u32 = 0;
pub const SCALE_FIT: u32 = 1;
pub const SCALE_FILL: u32 = 2;
pub const SCALE_STRETCH: u32 = 3;

leaf_control!(ImageView, KIND_IMAGE_VIEW);

impl ImageView {
    /// Create an empty ImageView with the given display size.
    pub fn new(w: u32, h: u32) -> Self {
        let id = (lib().create_control)(KIND_IMAGE_VIEW, core::ptr::null(), 0);
        (lib().set_size)(id, w, h);
        Self { ctrl: Control { id } }
    }

    /// Create an ImageView and load an image from a file path.
    /// Supports BMP, PNG, JPEG, GIF, ICO formats via libimage.
    pub fn from_file(path: &str, w: u32, h: u32) -> Self {
        let iv = Self::new(w, h);
        if let Ok(data) = anyos_std::fs::read_to_vec(path) {
            iv.load_from_bytes(&data);
        }
        iv
    }

    /// Create an ImageView and decode image data from raw bytes.
    pub fn from_bytes(data: &[u8], w: u32, h: u32) -> Self {
        let iv = Self::new(w, h);
        iv.load_from_bytes(data);
        iv
    }

    /// Load image data from bytes into this ImageView.
    pub fn load_from_bytes(&self, data: &[u8]) {
        if let Some(info) = libimage_client::probe(data) {
            let pixel_count = (info.width as usize) * (info.height as usize);
            let mut pixels = vec![0u32; pixel_count];
            let mut scratch = vec![0u8; info.scratch_needed as usize];
            if libimage_client::decode(data, &mut pixels, &mut scratch).is_ok() {
                (lib().imageview_set_pixels)(self.ctrl.id, pixels.as_ptr(), info.width, info.height);
            }
        }
    }

    /// Load image data from a file path into this ImageView.
    pub fn load_from_file(&self, path: &str) {
        if let Ok(data) = anyos_std::fs::read_to_vec(path) {
            self.load_from_bytes(&data);
        }
    }

    /// Load an ICO file at a specific icon size.
    pub fn load_ico(&self, path: &str, preferred_size: u32) {
        if let Ok(data) = anyos_std::fs::read_to_vec(path) {
            if let Some(info) = libimage_client::probe_ico_size(&data, preferred_size) {
                let pixel_count = (info.width as usize) * (info.height as usize);
                let mut pixels = vec![0u32; pixel_count];
                let mut scratch = vec![0u8; info.scratch_needed as usize];
                if libimage_client::decode_ico_size(&data, preferred_size, &mut pixels, &mut scratch).is_ok() {
                    (lib().imageview_set_pixels)(self.ctrl.id, pixels.as_ptr(), info.width, info.height);
                }
            }
        }
    }

    /// Set raw ARGB pixel data directly.
    pub fn set_pixels(&self, pixels: &[u32], w: u32, h: u32) {
        (lib().imageview_set_pixels)(self.ctrl.id, pixels.as_ptr(), w, h);
    }

    /// Set scale mode: SCALE_NONE, SCALE_FIT, SCALE_FILL, SCALE_STRETCH.
    pub fn set_scale_mode(&self, mode: u32) {
        (lib().imageview_set_scale_mode)(self.ctrl.id, mode);
    }

    /// Get the original image dimensions.
    pub fn image_size(&self) -> (u32, u32) {
        let mut w = 0u32;
        let mut h = 0u32;
        (lib().imageview_get_image_size)(self.ctrl.id, &mut w, &mut h);
        (w, h)
    }

    /// Clear the image.
    pub fn clear(&self) {
        (lib().imageview_clear)(self.ctrl.id);
    }
}
