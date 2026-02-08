/// ARGB8888 color representation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub a: u8,
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Color { a: 255, r, g, b }
    }

    pub const fn with_alpha(a: u8, r: u8, g: u8, b: u8) -> Self {
        Color { a, r, g, b }
    }

    pub const fn from_u32(argb: u32) -> Self {
        Color {
            a: ((argb >> 24) & 0xFF) as u8,
            r: ((argb >> 16) & 0xFF) as u8,
            g: ((argb >> 8) & 0xFF) as u8,
            b: (argb & 0xFF) as u8,
        }
    }

    pub const fn to_u32(self) -> u32 {
        (self.a as u32) << 24 | (self.r as u32) << 16 | (self.g as u32) << 8 | self.b as u32
    }

    /// Alpha-blend `self` over `dst`
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

    // Common colors
    pub const BLACK: Color = Color::new(0, 0, 0);
    pub const WHITE: Color = Color::new(255, 255, 255);
    pub const RED: Color = Color::new(255, 0, 0);
    pub const GREEN: Color = Color::new(0, 255, 0);
    pub const BLUE: Color = Color::new(0, 0, 255);
    pub const TRANSPARENT: Color = Color::with_alpha(0, 0, 0, 0);

    // macOS-inspired palette
    pub const MACOS_BG: Color = Color::new(30, 30, 30);           // Dark desktop
    pub const MACOS_MENUBAR: Color = Color::new(40, 40, 40);      // Menu bar
    pub const MACOS_DOCK: Color = Color::with_alpha(180, 50, 50, 50); // Dock
    pub const MACOS_WINDOW_BG: Color = Color::new(45, 45, 45);    // Window background
    pub const MACOS_TITLEBAR: Color = Color::new(55, 55, 55);     // Title bar
    pub const MACOS_TEXT: Color = Color::new(230, 230, 230);       // Primary text
    pub const MACOS_TEXT_DIM: Color = Color::new(150, 150, 150);   // Secondary text
    pub const MACOS_ACCENT: Color = Color::new(0, 122, 255);      // Accent blue
    pub const MACOS_CLOSE: Color = Color::new(255, 95, 86);       // Close button
    pub const MACOS_MINIMIZE: Color = Color::new(255, 189, 46);   // Minimize button
    pub const MACOS_MAXIMIZE: Color = Color::new(39, 201, 63);    // Maximize button
    pub const MACOS_BORDER: Color = Color::new(70, 70, 70);       // Window border
}
