# anyOS Render Library (librender) API Reference

The **librender** DLL provides software 2D rendering primitives for drawing shapes, gradients, and performing pixel operations on ARGB8888 buffers. All operations are CPU-based (no GPU involvement).

**DLL Address:** `0x04300000`
**Version:** 1
**Exports:** 18
**Client crate:** `librender_client`

---

## Getting Started

### Dependencies

```toml
[dependencies]
anyos_std = { path = "../../libs/stdlib" }
librender_client = { path = "../../libs/librender_client" }
```

### Example

```rust
use librender_client as render;

let mut pixels = vec![0u32; 400 * 300];
let (w, h) = (400u32, 300u32);

// Fill background
render::fill_surface(&mut pixels, w, h, 0xFF1E1E1E);

// Draw anti-aliased rounded rect
render::fill_rounded_rect_aa(&mut pixels, w, h, 20, 20, 200, 100, 12, 0xFF007AFF);

// Draw circle
render::fill_circle_aa(&mut pixels, w, h, 300, 80, 40, 0xFFFF3B30);

// Horizontal gradient
render::fill_gradient_h(&mut pixels, w, h, 20, 150, 360, 40, 0xFF34C759, 0xFF007AFF);
```

---

## Surface Operations

### `fill_rect(pixels, w, h, x, y, rw, rh, color)`

Fill a rectangle with a solid color (alpha-blended).

### `fill_surface(pixels, w, h, color)`

Fill the entire pixel buffer with a solid color.

### `put_pixel(pixels, w, h, x, y, color)`

Set a single pixel with alpha blending (src-over compositing).

### `get_pixel(pixels, w, h, x, y) -> u32`

Read the ARGB value of a single pixel.

### `blit_rect(dst, dw, dh, dx, dy, src, sw, sh, sx, sy, cw, ch, src_opaque)`

Copy a rectangular region from source buffer to destination. When `src_opaque` is true, copies without blending (faster). Otherwise performs per-pixel alpha blending.

### `put_pixel_subpixel(pixels, w, h, x, y, r_cov, g_cov, b_cov, color)`

Set a pixel with LCD subpixel coverage values (one byte per RGB channel). Used by the font renderer for subpixel anti-aliasing.

---

## Shape Primitives

All shape functions take a pixel buffer (`*mut u32`), buffer dimensions (w, h), and shape-specific parameters.

### Filled Shapes

| Function | Description |
|----------|-------------|
| `fill_rounded_rect(pixels, w, h, x, y, rw, rh, radius, color)` | Solid rounded rectangle |
| `fill_rounded_rect_aa(pixels, w, h, x, y, rw, rh, radius, color)` | Anti-aliased rounded rectangle |
| `fill_circle(pixels, w, h, cx, cy, radius, color)` | Solid filled circle |
| `fill_circle_aa(pixels, w, h, cx, cy, radius, color)` | Anti-aliased filled circle |

### Outlines

| Function | Description |
|----------|-------------|
| `draw_rect(pixels, w, h, x, y, rw, rh, color, thickness)` | Rectangle outline with configurable thickness |
| `draw_circle(pixels, w, h, cx, cy, radius, color)` | 1px circle outline |
| `draw_circle_aa(pixels, w, h, cx, cy, radius, color)` | Anti-aliased circle outline |
| `draw_rounded_rect_aa(pixels, w, h, x, y, rw, rh, radius, color)` | 1px anti-aliased rounded rect outline |
| `draw_line(pixels, w, h, x0, y0, x1, y1, color)` | Line between two points (Bresenham's algorithm) |

### Gradients

| Function | Description |
|----------|-------------|
| `fill_gradient_h(pixels, w, h, x, y, rw, rh, color_left, color_right)` | Horizontal linear gradient |
| `fill_gradient_v(pixels, w, h, x, y, rw, rh, color_top, color_bottom)` | Vertical linear gradient |

---

## Color Utility

### `blend_color(src, dst) -> u32`

Perform alpha-blend of `src` over `dst` using standard src-over compositing. Both colors are ARGB8888.

```rust
let result = render::blend_color(0x80FF0000, 0xFF0000FF); // semi-transparent red over blue
```

---

## Client Wrapper: `Surface`

The `librender_client` crate provides a typed `Surface` struct that wraps a raw pixel buffer. All 17 shape functions have method wrappers on `Surface`.

### Construction

```rust
// Unsafe: buffer must be valid for w * h u32 values
let mut surface = unsafe { render::Surface::from_raw(pixels.as_mut_ptr(), w, h) };
```

### Methods

All methods mirror the free functions above:

```rust
surface.fill(color);                                    // fill_surface
surface.fill_rect(x, y, rw, rh, color);               // fill_rect
surface.put_pixel(x, y, color);                        // put_pixel (alpha-blended)
surface.get_pixel(x, y) -> u32;                        // get_pixel
surface.blit_rect(dx, dy, src, sw, sh, sx, sy, cw, ch, opaque);
surface.put_pixel_subpixel(x, y, r, g, b, color);     // LCD subpixel
surface.fill_rounded_rect(x, y, rw, rh, radius, color);
surface.fill_rounded_rect_aa(x, y, rw, rh, radius, color);
surface.fill_circle(cx, cy, radius, color);
surface.fill_circle_aa(cx, cy, radius, color);
surface.draw_line(x0, y0, x1, y1, color);
surface.draw_rect(x, y, rw, rh, color, thickness);    // thickness in pixels
surface.draw_circle(cx, cy, radius, color);
surface.draw_circle_aa(cx, cy, radius, color);
surface.draw_rounded_rect_aa(x, y, rw, rh, radius, color);
surface.fill_gradient_h(x, y, rw, rh, color_left, color_right);
surface.fill_gradient_v(x, y, rw, rh, color_top, color_bottom);
```

### Blending Behavior

- `fill()` — Direct write, no alpha blending
- `put_pixel()` — Alpha-blended (src-over compositing)
- All shape functions — Alpha-blended
- `blit_rect()` with `opaque=true` — Direct copy (faster), `opaque=false` — blended

### Bounds Checking

All coordinates are clipped to surface bounds. Out-of-bounds operations are silently ignored — no panics or undefined behavior.
