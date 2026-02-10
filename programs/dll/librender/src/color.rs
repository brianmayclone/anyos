//! ARGB8888 color type with alpha blending and a macOS-inspired dark palette.

/// ARGB8888 color representation with 8 bits per channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub a: u8,
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    /// Create a fully opaque color from RGB components.
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Color { a: 255, r, g, b }
    }

    /// Create a color with explicit alpha, red, green, and blue components.
    pub const fn with_alpha(a: u8, r: u8, g: u8, b: u8) -> Self {
        Color { a, r, g, b }
    }

    /// Decode a color from a packed 0xAARRGGBB u32.
    pub const fn from_u32(argb: u32) -> Self {
        Color {
            a: ((argb >> 24) & 0xFF) as u8,
            r: ((argb >> 16) & 0xFF) as u8,
            g: ((argb >> 8) & 0xFF) as u8,
            b: (argb & 0xFF) as u8,
        }
    }

    /// Encode this color as a packed 0xAARRGGBB u32.
    pub const fn to_u32(self) -> u32 {
        (self.a as u32) << 24 | (self.r as u32) << 16 | (self.g as u32) << 8 | self.b as u32
    }

    /// Alpha-blend `self` over `dst`.
    pub fn blend_over(self, dst: Color) -> Color {
        if self.a == 255 {
            return self;
        }
        if self.a == 0 {
            return dst;
        }

        let sa = self.a as u32;
        let da = 255 - sa;

        Color {
            a: 255,
            r: ((self.r as u32 * sa + dst.r as u32 * da) / 255) as u8,
            g: ((self.g as u32 * sa + dst.g as u32 * da) / 255) as u8,
            b: ((self.b as u32 * sa + dst.b as u32 * da) / 255) as u8,
        }
    }

    pub const TRANSPARENT: Color = Color::with_alpha(0, 0, 0, 0);
}
