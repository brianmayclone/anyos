# anyOS Font Library (libfont) API Reference

The **libfont** DLL is a shared library for loading TrueType fonts and rendering text into pixel buffers. It supports greyscale and LCD subpixel anti-aliasing.

**DLL Address:** `0x04200000`
**Version:** 2
**Exports:** 7
**Client crate:** `libfont_client`

---

## Getting Started

### Dependencies

```toml
[dependencies]
anyos_std = { path = "../../libs/stdlib" }
libfont_client = { path = "../../libs/libfont_client" }
```

### Example

```rust
use libfont_client as font;

// Initialize (loads system fonts, detects subpixel capability)
font::init();

// Measure text
let (w, h) = font::measure(0, 13, "Hello, World!");

// Render into ARGB8888 buffer
let mut pixels = vec![0u32; 200 * 30];
font::draw_string_buf(&mut pixels, 200, 30, 0, 0, 0xFFFFFFFF, 0, 13, "Hello, World!");
```

---

## Functions

### `init()`

Initialize the font subsystem. Loads system fonts from `/System/fonts/`, auto-detects LCD subpixel rendering capability based on GPU driver (enabled for VMware SVGA II).

Must be called once before any other font operations.

---

### `load_font(path) -> u32`

Load a custom TTF font from a filesystem path.

| Parameter | Type | Description |
|-----------|------|-------------|
| path | `&str` | Filesystem path to `.ttf` file |
| **Returns** | `u32` | Font ID (>0) or `u32::MAX` on failure |

Font ID 0 is always the default system font (SF Pro).

---

### `unload_font(font_id)`

Unload a previously loaded font and free its memory.

| Parameter | Type | Description |
|-----------|------|-------------|
| font_id | `u32` | Font ID returned by `load_font` |

---

### `measure(font_id, size, text) -> (u32, u32)`

Measure the pixel dimensions of rendered text without drawing.

| Parameter | Type | Description |
|-----------|------|-------------|
| font_id | `u32` | Font ID (0 = system font) |
| size | `u16` | Font size in pixels |
| text | `&str` | Text string to measure |
| **Returns** | `(u32, u32)` | (width, height) in pixels |

---

### `draw_string_buf(pixels, buf_w, buf_h, x, y, color, font_id, size, text)`

Render text into an ARGB8888 pixel buffer with alpha-blended anti-aliasing.

| Parameter | Type | Description |
|-----------|------|-------------|
| pixels | `&mut [u32]` | Target pixel buffer (ARGB8888) |
| buf_w | `u32` | Buffer width in pixels |
| buf_h | `u32` | Buffer height in pixels |
| x, y | `i32` | Top-left position to start rendering |
| color | `u32` | Text color (ARGB8888, e.g. `0xFFFFFFFF` = white) |
| font_id | `u32` | Font ID (0 = system font) |
| size | `u16` | Font size in pixels |
| text | `&str` | Text string to render |

When subpixel rendering is enabled, each glyph pixel is rendered with separate R/G/B coverage values for LCD-quality anti-aliasing.

---

### `line_height(font_id, size) -> u32`

Get the line height for a font at a given size. Useful for multi-line text layout.

| Parameter | Type | Description |
|-----------|------|-------------|
| font_id | `u32` | Font ID (0 = system font) |
| size | `u16` | Font size in pixels |
| **Returns** | `u32` | Line height in pixels |

---

### `set_subpixel(enabled)`

Override the auto-detected subpixel rendering setting.

| Parameter | Type | Description |
|-----------|------|-------------|
| enabled | `u32` | 1 = enable LCD subpixel, 0 = greyscale only |

Auto-detection on `init()`: enabled when VMware SVGA II is present (LCD monitors assumed), greyscale for Bochs VGA.
