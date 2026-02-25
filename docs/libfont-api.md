# anyOS Font Library (libfont) API Reference

The **libfont** shared library provides TrueType font loading and text rendering into pixel buffers. It supports greyscale and LCD subpixel anti-aliasing with **size-adaptive gamma correction** for optimal readability on dark backgrounds.

**Format:** ELF64 shared object (.so), loaded on demand via `SYS_DLL_LOAD`
**Load Address:** `0x05000000`
**Exports:** 8
**Client crate:** `libfont_client` (uses `dynlink::dl_open` / `dl_sym`)

System fonts (SF Pro family + Andale Mono, ~17 MiB) are embedded in `.rodata` via `include_bytes!()`. Since `.rodata` pages are shared read-only across all processes, the font data exists once in physical RAM — zero disk I/O at init, zero per-process memory duplication.

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

// Initialize (loads embedded system fonts, detects subpixel capability)
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

Initialize the font subsystem. Loads the `.so` via `dl_open("/Libraries/libfont.so")`, resolves all exported symbols, and calls `font_init()` which registers the embedded system fonts, initializes the gamma correction LUTs, and auto-detects LCD subpixel rendering capability based on GPU driver (enabled for VMware SVGA II).

Must be called once before any other font operations. Returns `true` on success.

#### Gamma Correction

During init, two 256-byte lookup tables are computed for size-adaptive gamma correction:

| Font Size | LUT | Effect |
|-----------|-----|--------|
| ≤ 14 px | Strong (`GAMMA_LUT_S`) | ~50% coverage boost for thin strokes — small text is clearly visible on dark backgrounds |
| 15–24 px | Moderate (`GAMMA_LUT_M`) | ~33% boost — balanced readability without over-thickening |
| > 24 px | Identity (no LUT) | Large text has sufficient stroke width, no correction needed |

The gamma curve blends linear and square-root components using integer math (no floating point). The 256-byte LUT lives permanently in L1 cache — zero measurable performance overhead (one byte lookup per coverage sample).

---

### `load(path) -> Option<u32>`

Load a custom TTF font from a filesystem path (reads from disk).

| Parameter | Type | Description |
|-----------|------|-------------|
| path | `&str` | Filesystem path to `.ttf` file |
| **Returns** | `Option<u32>` | Font ID on success, `None` on failure |

Font IDs 0–4 are the embedded system fonts (see table below).

---

### `unload(font_id)`

Unload a previously loaded font and free its memory.

| Parameter | Type | Description |
|-----------|------|-------------|
| font_id | `u32` | Font ID returned by `load()` |

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

### `draw_string_buf(buf, buf_w, buf_h, x, y, color, font_id, size, text)`

Render text into an ARGB8888 pixel buffer with alpha-blended anti-aliasing.

| Parameter | Type | Description |
|-----------|------|-------------|
| buf | `*mut u32` | Target pixel buffer (ARGB8888) |
| buf_w | `u32` | Buffer width in pixels |
| buf_h | `u32` | Buffer height in pixels |
| x, y | `i32` | Top-left position to start rendering |
| color | `u32` | Text color (ARGB8888, e.g. `0xFFFFFFFF` = white) |
| font_id | `u32` | Font ID (0 = system font) |
| size | `u16` | Font size in pixels |
| text | `&str` | Text string to render |

When subpixel rendering is enabled, each glyph pixel is rendered with separate R/G/B coverage values for LCD-quality anti-aliasing.

---

### `draw_string_buf_clipped(buf, buf_w, buf_h, x, y, color, font_id, size, text, clip_x, clip_y, clip_r, clip_b)`

Render text with clip rectangle. Same as `draw_string_buf` but only draws pixels within the specified clip region.

| Parameter | Type | Description |
|-----------|------|-------------|
| buf | `*mut u32` | Target pixel buffer (ARGB8888) |
| buf_w | `u32` | Buffer width in pixels |
| buf_h | `u32` | Buffer height in pixels |
| x, y | `i32` | Top-left position to start rendering |
| color | `u32` | Text color (ARGB8888) |
| font_id | `u32` | Font ID (0 = system font) |
| size | `u16` | Font size in pixels |
| text | `&str` | Text string to render |
| clip_x, clip_y | `i32` | Clip rectangle left, top (pixels) |
| clip_r, clip_b | `i32` | Clip rectangle right, bottom (pixels) |

**Note:** This function is exported from `libfont.so` but has no wrapper in `libfont_client` — use the raw FFI export directly if clipped rendering is needed.

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
| enabled | `bool` | `true` = enable LCD subpixel, `false` = greyscale only |

Auto-detection on `init()`: enabled when VMware SVGA II is present (LCD monitors assumed), greyscale for Bochs VGA.

---

## System Fonts

| ID | Font | File (embedded) | Usage |
|----|------|-----------------|-------|
| 0 | SF Pro | sfpro.ttf | Default UI text |
| 1 | SF Pro Bold | sfpro-bold.ttf | Bold text, headers |
| 2 | SF Pro Thin | sfpro-thin.ttf | Thin/light text |
| 3 | SF Pro Italic | sfpro-italic.ttf | Italic text |
| 4 | Andale Mono | andale-mono.ttf | Monospace (terminal, code editor) |

These fonts are compiled into `libfont.so`'s `.rodata` section and shared across all processes. No disk files are needed at runtime for system fonts.

## Architecture

libfont uses two library formats:

- **libfont** (`libs/libfont/`) — the shared library itself, built as a `staticlib` and linked by `anyld` into an ELF64 `.so`. Exports 8 `#[no_mangle] pub extern "C"` symbols (the client wraps 7 of them; `draw_string_buf_clipped` is server-only).
- **libfont_client** (`libs/libfont_client/`) — client wrapper that resolves symbols via `dynlink::dl_open("/Libraries/libfont.so")` + `dl_sym()`. Caches function pointers in a static `FontLib` struct.

Other libraries (libanyui, uisys, stdlib) that need font rendering resolve libfont symbols directly via inline ELF parsing of the mapped `.so` at runtime.
