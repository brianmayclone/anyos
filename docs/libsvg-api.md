# anyOS SVG Library (libsvg) API Reference

The **libsvg** DLL is a shared library for parsing and rasterizing SVG 1.1 static images into ARGB8888 pixel buffers. It supports a practical subset of SVG elements including paths, basic shapes, gradients, transforms, and fill/stroke styling.

**Loaded via:** `dl_open("/Libraries/libsvg.so")`
**Exports:** 3
**Client crate:** `libsvg_client`

---

## Table of Contents

- [Getting Started](#getting-started)
- [Exported Functions](#exported-functions)
  - [svg_probe](#svg_probe)
  - [svg_render](#svg_render)
  - [svg_render_to_size](#svg_render_to_size)
- [Client Wrapper](#client-wrapper)
  - [init](#init)
  - [probe](#probe)
  - [render](#render)
  - [render_to_size](#render_to_size)
- [Supported SVG Subset](#supported-svg-subset)
  - [Elements](#elements)
  - [Path Commands](#path-commands)
  - [Gradients](#gradients)
  - [Transforms](#transforms)
  - [Fill and Stroke Styles](#fill-and-stroke-styles)
- [Error Handling](#error-handling)
- [Examples](#examples)

---

## Getting Started

### Dependencies

Add to your program's `Cargo.toml`:

```toml
[dependencies]
anyos_std = { path = "../../libs/stdlib" }
dynlink = { path = "../../libs/dynlink" }
libsvg_client = { path = "../../libs/libsvg_client" }
```

### Minimal Example

```rust
#![no_std]
#![no_main]

use anyos_std::*;
use libsvg_client as svg;

anyos_std::entry!(main);

fn main() {
    // Initialize (loads libsvg.so via dl_open)
    if !svg::init() {
        println!("Failed to load libsvg");
        return;
    }

    // Read SVG file
    let fd = fs::open("/images/icon.svg", 0);
    if fd == u32::MAX { return; }

    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = fs::read(fd, &mut buf);
        if n == 0 || n == u32::MAX { break; }
        data.extend_from_slice(&buf[..n as usize]);
    }
    fs::close(fd);

    // Probe for native dimensions
    if let Some((w, h)) = svg::probe(&data) {
        println!("SVG dimensions: {}x{}", w, h);

        // Render at native size
        let iw = w as u32;
        let ih = h as u32;
        let mut pixels = vec![0u32; (iw * ih) as usize];
        if svg::render(&data, &mut pixels, iw, ih) {
            println!("Rendered {} pixels", iw * ih);
        }
    }
}
```

### Memory Model

The DLL is **stateless** with caller-provided buffers:

1. **SVG data** (`&[u8]`): The raw SVG file bytes (UTF-8 XML)
2. **Pixel buffer** (`&mut [u32]`): Output ARGB8888 pixels, `width * height` elements

No internal heap allocation or caching. The caller owns all memory, making the library safe for use across process address spaces.

---

## Exported Functions

These are the raw `#[no_mangle] pub extern "C"` symbols exported from `libsvg.so`. Most users should use the [client wrapper](#client-wrapper) instead.

### svg_probe

```c
i32 svg_probe(const u8 *data, u32 data_len, f32 *out_width, f32 *out_height);
```

Parse the SVG header to extract the native width and height from the root `<svg>` element's `width`, `height`, or `viewBox` attributes.

**Parameters:**
- `data` -- Pointer to raw SVG file bytes (UTF-8)
- `data_len` -- Length of the SVG data in bytes
- `out_width` -- Pointer to receive the native width (in SVG user units)
- `out_height` -- Pointer to receive the native height (in SVG user units)

**Returns:**
- `0` on success, `out_width` and `out_height` are populated
- `-1` on error (null pointer, invalid XML, missing dimensions)

**Notes:**
- Only parses the root `<svg>` element, does not process child elements
- If the root element has `viewBox` but no explicit `width`/`height`, the viewBox dimensions are returned
- Dimensions are returned as floating-point SVG user units

### svg_render

```c
i32 svg_render(const u8 *data, u32 data_len, u32 *pixels, u32 width, u32 height);
```

Parse and rasterize an SVG document into an ARGB8888 pixel buffer. The SVG content is scaled to fit the specified output dimensions while preserving aspect ratio. The pixel buffer is cleared to transparent (`0x00000000`) before rendering.

**Parameters:**
- `data` -- Pointer to raw SVG file bytes (UTF-8)
- `data_len` -- Length of the SVG data in bytes
- `pixels` -- Output pixel buffer, must have at least `width * height` elements
- `width` -- Output width in pixels
- `height` -- Output height in pixels

**Returns:**
- `0` on success, pixel buffer is filled with ARGB8888 values
- `-1` on error (null pointer, invalid SVG, zero dimensions)

**Pixel format:** Each `u32` is `0xAARRGGBB` (alpha in high byte, blue in low byte). Transparent areas have alpha = `0x00`. Fully opaque areas have alpha = `0xFF`.

### svg_render_to_size

```c
i32 svg_render_to_size(const u8 *data, u32 data_len, u32 *pixels, u32 width, u32 height, u32 bg_color);
```

Parse and rasterize an SVG document into an ARGB8888 pixel buffer with a specified background color. Same as `svg_render` but composites onto a solid background instead of transparent.

**Parameters:**
- `data` -- Pointer to raw SVG file bytes (UTF-8)
- `data_len` -- Length of the SVG data in bytes
- `pixels` -- Output pixel buffer, must have at least `width * height` elements
- `width` -- Output width in pixels
- `height` -- Output height in pixels
- `bg_color` -- Background color as ARGB8888 (e.g. `0xFFFFFFFF` for opaque white)

**Returns:**
- `0` on success
- `-1` on error

**Notes:**
- The pixel buffer is first filled with `bg_color`, then SVG content is composited on top using src-over alpha blending
- Useful when the SVG will be displayed on a known background color and you want pre-multiplied output with no transparency

---

## Client Wrapper

The `libsvg_client` crate provides safe Rust wrappers around the raw DLL exports. It loads `libsvg.so` via `dynlink::dl_open` and resolves function pointers on initialization.

### init

```rust
pub fn init() -> bool
```

Load `/Libraries/libsvg.so` via `dl_open` and resolve all 3 exported symbols. Must be called once before any other `libsvg_client` functions.

**Returns:**
- `true` on success
- `false` if the library could not be loaded or symbols could not be resolved

### probe

```rust
pub fn probe(data: &[u8]) -> Option<(f32, f32)>
```

Parse the SVG header to extract the native dimensions.

**Parameters:**
- `data` -- Raw SVG file bytes

**Returns:**
- `Some((width, height))` with the native dimensions in SVG user units
- `None` if the SVG is invalid or dimensions could not be determined

### render

```rust
pub fn render(data: &[u8], pixels: &mut [u32], width: u32, height: u32) -> bool
```

Rasterize an SVG document into an ARGB8888 pixel buffer with a transparent background.

**Parameters:**
- `data` -- Raw SVG file bytes
- `pixels` -- Output buffer, must have at least `width * height` elements
- `width` -- Output width in pixels
- `height` -- Output height in pixels

**Returns:**
- `true` on success
- `false` on error

### render_to_size

```rust
pub fn render_to_size(data: &[u8], pixels: &mut [u32], width: u32, height: u32, bg_color: u32) -> bool
```

Rasterize an SVG document into an ARGB8888 pixel buffer with a specified background color.

**Parameters:**
- `data` -- Raw SVG file bytes
- `pixels` -- Output buffer, must have at least `width * height` elements
- `width` -- Output width in pixels
- `height` -- Output height in pixels
- `bg_color` -- Background color as ARGB8888 (e.g. `0xFFFFFFFF` for white, `0xFF1E1E1E` for dark grey)

**Returns:**
- `true` on success
- `false` on error

---

## Supported SVG Subset

libsvg implements a practical subset of SVG 1.1 focused on static vector graphics. Animations, scripting, text, filters, and CSS stylesheets are not supported.

### Elements

| Element | Attributes | Notes |
|---------|-----------|-------|
| `<svg>` | `width`, `height`, `viewBox`, `xmlns` | Root element, defines coordinate space |
| `<g>` | `transform`, `fill`, `stroke`, `opacity` | Group element, inherits styles to children |
| `<path>` | `d`, `fill`, `stroke`, `stroke-width`, `fill-rule`, `transform` | General-purpose shape via path commands |
| `<rect>` | `x`, `y`, `width`, `height`, `rx`, `ry`, `fill`, `stroke`, `stroke-width`, `transform` | Rectangle with optional rounded corners |
| `<circle>` | `cx`, `cy`, `r`, `fill`, `stroke`, `stroke-width`, `transform` | Circle |
| `<ellipse>` | `cx`, `cy`, `rx`, `ry`, `fill`, `stroke`, `stroke-width`, `transform` | Ellipse |
| `<line>` | `x1`, `y1`, `x2`, `y2`, `stroke`, `stroke-width`, `transform` | Single line segment |
| `<polyline>` | `points`, `fill`, `stroke`, `stroke-width`, `transform` | Connected line segments (open) |
| `<polygon>` | `points`, `fill`, `stroke`, `stroke-width`, `transform` | Connected line segments (closed) |

### Path Commands

The `d` attribute of `<path>` elements supports the following commands. Each command is available in absolute (uppercase) and relative (lowercase) form.

| Command | Parameters | Description |
|---------|-----------|-------------|
| `M` / `m` | `x y` | Move to (start new subpath) |
| `L` / `l` | `x y` | Line to |
| `H` / `h` | `x` | Horizontal line to |
| `V` / `v` | `y` | Vertical line to |
| `C` / `c` | `x1 y1 x2 y2 x y` | Cubic Bezier curve |
| `S` / `s` | `x2 y2 x y` | Smooth cubic Bezier (reflected control point) |
| `Q` / `q` | `x1 y1 x y` | Quadratic Bezier curve |
| `T` / `t` | `x y` | Smooth quadratic Bezier (reflected control point) |
| `A` / `a` | `rx ry x-rotation large-arc-flag sweep-flag x y` | Elliptical arc |
| `Z` / `z` | *(none)* | Close path (line back to subpath start) |

### Gradients

| Element | Attributes | Notes |
|---------|-----------|-------|
| `<linearGradient>` | `id`, `x1`, `y1`, `x2`, `y2`, `gradientUnits`, `gradientTransform`, `spreadMethod` | Linear gradient definition |
| `<radialGradient>` | `id`, `cx`, `cy`, `r`, `fx`, `fy`, `gradientUnits`, `gradientTransform`, `spreadMethod` | Radial gradient definition |
| `<stop>` | `offset`, `stop-color`, `stop-opacity` | Gradient color stop |

**gradientUnits:**
- `objectBoundingBox` (default) -- gradient coordinates relative to the bounding box of the referencing element (0.0 to 1.0)
- `userSpaceOnUse` -- gradient coordinates in the current user coordinate system

**spreadMethod:**
- `pad` (default) -- extend the last color beyond the gradient bounds
- `reflect` -- mirror the gradient pattern beyond its bounds
- `repeat` -- tile the gradient pattern beyond its bounds

Gradients are referenced via `fill="url(#gradient-id)"` or `stroke="url(#gradient-id)"`.

### Transforms

The `transform` attribute supports the following transform functions. Multiple transforms can be chained in a single attribute (applied right-to-left).

| Function | Parameters | Description |
|----------|-----------|-------------|
| `matrix` | `a b c d e f` | General 2D affine transform matrix |
| `translate` | `tx [ty]` | Translation (`ty` defaults to 0) |
| `rotate` | `angle [cx cy]` | Rotation in degrees around optional center point |
| `scale` | `sx [sy]` | Scale (`sy` defaults to `sx` for uniform scaling) |
| `skewX` | `angle` | Horizontal skew in degrees |
| `skewY` | `angle` | Vertical skew in degrees |

### Fill and Stroke Styles

**Fill:**
- `fill` -- Fill color (`none`, named color, `#RGB`, `#RRGGBB`, `rgb(r,g,b)`, or `url(#id)` for gradients)
- `fill-opacity` -- Fill opacity (0.0 to 1.0)
- `fill-rule` -- Winding rule for determining interior: `nonzero` (default) or `evenodd`
- `opacity` -- Element-level opacity (affects both fill and stroke)

**Stroke:**
- `stroke` -- Stroke color (same format as fill)
- `stroke-width` -- Stroke width in user units (default: 1)
- `stroke-opacity` -- Stroke opacity (0.0 to 1.0)
- `stroke-linecap` -- Line cap style: `butt` (default), `round`, `square`
- `stroke-linejoin` -- Line join style: `miter` (default), `round`, `bevel`
- `stroke-miterlimit` -- Miter limit ratio (default: 4)

**Color formats:**
- Named colors: `black`, `white`, `red`, `green`, `blue`, `yellow`, `cyan`, `magenta`, `orange`, `purple`, `gray`/`grey`, `transparent`, `none`
- Hex: `#RGB` (e.g. `#F00`), `#RRGGBB` (e.g. `#FF0000`)
- RGB function: `rgb(255, 0, 0)` or `rgb(100%, 0%, 0%)`

---

## Error Handling

### Common Error Scenarios

| Scenario | Cause | Fix |
|----------|-------|-----|
| `init()` returns `false` | `/Libraries/libsvg.so` not found or symbols missing | Ensure the library is installed in the system image |
| `probe()` returns `None` | Invalid XML, missing `<svg>` root, or no dimensions | Verify the SVG file is well-formed |
| `render()` returns `false` | Null pointer, zero dimensions, or parse error | Check that `width > 0`, `height > 0`, and buffer is large enough |
| Unexpected rendering | Unsupported SVG feature used | Simplify SVG to use only supported elements and attributes |

---

## Examples

### Render SVG to a Fixed Size

```rust
use libsvg_client as svg;

fn render_svg_64x64(data: &[u8]) -> Option<Vec<u32>> {
    if !svg::init() { return None; }

    let mut pixels = vec![0u32; 64 * 64];
    if svg::render(data, &mut pixels, 64, 64) {
        Some(pixels)
    } else {
        None
    }
}
```

### Render SVG with Background Color

```rust
use libsvg_client as svg;

fn render_svg_on_dark(data: &[u8], w: u32, h: u32) -> Option<Vec<u32>> {
    let mut pixels = vec![0u32; (w * h) as usize];
    if svg::render_to_size(data, &mut pixels, w, h, 0xFF1E1E1E) {
        Some(pixels)
    } else {
        None
    }
}
```

### Probe and Render at Native Size

```rust
use libsvg_client as svg;

fn render_native(data: &[u8]) -> Option<(Vec<u32>, u32, u32)> {
    let (w, h) = svg::probe(data)?;
    let iw = w as u32;
    let ih = h as u32;
    if iw == 0 || ih == 0 { return None; }

    let mut pixels = vec![0u32; (iw * ih) as usize];
    if svg::render(data, &mut pixels, iw, ih) {
        Some((pixels, iw, ih))
    } else {
        None
    }
}
```

### Render SVG for Display in a Window

```rust
use anyos_std::*;
use libsvg_client as svg;

fn show_svg(win: u32, data: &[u8], win_w: u32, win_h: u32) {
    let mut pixels = vec![0u32; (win_w * win_h) as usize];

    if svg::render_to_size(data, &mut pixels, win_w, win_h, 0xFF252526) {
        // Blit rows to window (64 rows at a time for efficiency)
        let w = win_w as u16;
        let mut y = 0u32;
        while y < win_h {
            let rows = (win_h - y).min(64);
            let start = (y * win_w) as usize;
            let end = start + (rows * win_w) as usize;
            ui::window::blit(win, 0, y as i16, w, rows as u16, &pixels[start..end]);
            y += rows;
        }
        ui::window::present(win);
    }
}
```

### Detect SVG Dimensions Without Rendering

```rust
use libsvg_client as svg;

fn svg_info(data: &[u8]) {
    match svg::probe(data) {
        Some((w, h)) => anyos_std::println!("SVG: {:.1} x {:.1} user units", w, h),
        None => anyos_std::println!("Not a valid SVG"),
    }
}
```
