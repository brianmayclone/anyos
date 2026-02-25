# anyOS Image Library (libimage) API Reference

The **libimage** DLL is a shared library for decoding/encoding images and video frames into ARGB8888 pixel buffers. It supports BMP, PNG, JPEG, GIF, and ICO image formats plus MJV video, and includes scaling, BMP encoding, and icon pack rendering APIs.

**DLL Address:** `0x04100000`
**Version:** 1
**Exports:** 10
**Client crate:** `libimage_client`

---

## Table of Contents

- [Getting Started](#getting-started)
- [Types](#types)
- [Image Functions](#image-functions)
  - [probe](#probe)
  - [decode](#decode)
  - [format_name](#format_name)
- [ICO Functions](#ico-functions)
  - [probe_ico_size](#probe_ico_size)
  - [decode_ico_size](#decode_ico_size)
- [Video Functions](#video-functions)
  - [video_probe](#video_probe)
  - [video_decode_frame](#video_decode_frame)
- [Scale Functions](#scale-functions)
  - [scale_image](#scale_image)
- [Encode Functions](#encode-functions)
  - [encode_bmp](#encode_bmp)
- [Iconpack Functions](#iconpack-functions)
  - [iconpack_render](#iconpack_render)
  - [iconpack_render_cached](#iconpack_render_cached)
- [Format Support](#format-support)
  - [BMP](#bmp)
  - [PNG](#png)
  - [JPEG](#jpeg)
  - [GIF](#gif)
  - [ICO](#ico)
  - [MJV (Video)](#mjv-video)
- [Error Handling](#error-handling)
- [Examples](#examples)

---

## Getting Started

### Dependencies

Add to your program's `Cargo.toml`:

```toml
[dependencies]
anyos_std = { path = "../../libs/stdlib" }
libimage_client = { path = "../../libs/libimage_client" }
```

### Minimal Decode Example

```rust
#![no_std]
#![no_main]

use anyos_std::*;
use libimage_client;

anyos_std::entry!(main);

fn main() {
    // Read image file
    let fd = fs::open("/images/photo.png", 0);
    if fd == u32::MAX { return; }

    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = fs::read(fd, &mut buf);
        if n == 0 || n == u32::MAX { break; }
        data.extend_from_slice(&buf[..n as usize]);
    }
    fs::close(fd);

    // Probe format and dimensions
    let info = match libimage_client::probe(&data) {
        Some(i) => i,
        None => {
            println!("Unsupported image format");
            return;
        }
    };

    println!("{} image: {}x{}",
        libimage_client::format_name(info.format),
        info.width, info.height);

    // Allocate output and scratch buffers
    let pixel_count = (info.width * info.height) as usize;
    let mut pixels = vec![0u32; pixel_count];
    let mut scratch = vec![0u8; info.scratch_needed as usize];

    // Decode
    match libimage_client::decode(&data, &mut pixels, &mut scratch) {
        Ok(()) => println!("Decoded {} pixels", pixel_count),
        Err(e) => println!("Decode error: {:?}", e),
    }
}
```

### Memory Model

The DLL is mostly **stateless** with caller-provided buffers. The only internal state is the `iconpack_render_cached` function which lazy-loads and caches `/System/media/ico.pak` on first call:

1. **Image data** (`&[u8]`): The raw file bytes (read from disk)
2. **Pixel buffer** (`&mut [u32]`): Output ARGB8888 pixels, `width * height` elements
3. **Scratch buffer** (`&mut [u8]`): Working memory for the decoder, size from `probe()`

This design avoids heap allocation inside the DLL, making it safe for use across process address spaces.

---

## Types

### ImageInfo

Returned by `probe()` and `probe_ico_size()` with image metadata.

```rust
#[repr(C)]
pub struct ImageInfo {
    pub width: u32,          // Image width in pixels
    pub height: u32,         // Image height in pixels
    pub format: u32,         // Format identifier (see constants below)
    pub scratch_needed: u32, // Bytes of scratch buffer needed for decode()
}
```

### VideoInfo

Returned by `video_probe()` with video metadata.

```rust
#[repr(C)]
pub struct VideoInfo {
    pub width: u32,          // Frame width in pixels
    pub height: u32,         // Frame height in pixels
    pub fps: u32,            // Frames per second
    pub num_frames: u32,     // Total number of frames
    pub scratch_needed: u32, // Bytes of scratch buffer needed for video_decode_frame()
}
```

### Format Constants

| Constant | Value | Magic Bytes |
|----------|-------|-------------|
| `FMT_UNKNOWN` | 0 | -- |
| `FMT_BMP` | 1 | `BM` |
| `FMT_PNG` | 2 | `\x89PNG\r\n\x1a\n` |
| `FMT_JPEG` | 3 | `\xFF\xD8` |
| `FMT_GIF` | 4 | `GIF87a` or `GIF89a` |
| `FMT_ICO` | 5 | `\x00\x00\x01\x00` (reserved=0, type=1) |
| `FMT_MJV` | 10 | `MJV1` |

### Scale Mode Constants

| Constant | Value | Description |
|----------|-------|-------------|
| `MODE_SCALE` | 0 | Stretch to fill, ignoring aspect ratio |
| `MODE_CONTAIN` | 1 | Fit within destination, maintaining aspect ratio (letterboxed with transparent black) |
| `MODE_COVER` | 2 | Fill destination, maintaining aspect ratio (excess cropped) |

### ImageError

Error type returned by decode functions.

| Variant | Raw Code | Description |
|---------|----------|-------------|
| `InvalidData` | -1 | Null pointer, truncated file, or corrupt data |
| `Unsupported` | -2 | Unrecognized format or unsupported feature |
| `BufferTooSmall` | -3 | Pixel output buffer smaller than width*height |
| `ScratchTooSmall` | -4 | Scratch buffer smaller than `scratch_needed` |
| `Unknown(i32)` | other | Unexpected error code |

---

## Image Functions

### probe

```rust
pub fn probe(data: &[u8]) -> Option<ImageInfo>
```

Detect the image format from magic bytes and parse the header for dimensions. Supports BMP, PNG, JPEG, GIF, and ICO formats.

**Parameters:**
- `data` -- Raw image file bytes (at least 8 bytes required)

**Returns:**
- `Some(ImageInfo)` with format, dimensions, and scratch buffer size
- `None` if the format is not recognized

**Notes:**
- Only reads the file header, does not decode pixel data
- The `scratch_needed` field tells you how large a scratch buffer to allocate for `decode()`
- BMP images need no scratch buffer (`scratch_needed = 0`)
- For ICO files, selects the best entry closest to 16x16 (default behavior)

### decode

```rust
pub fn decode(data: &[u8], pixels: &mut [u32], scratch: &mut [u8]) -> Result<(), ImageError>
```

Decode an image into ARGB8888 pixels. The format is auto-detected from magic bytes.

**Parameters:**
- `data` -- Raw image file bytes
- `pixels` -- Output buffer, must have at least `width * height` elements
- `scratch` -- Working memory, must have at least `scratch_needed` bytes (from `probe()`)

**Returns:**
- `Ok(())` on success, pixels filled with ARGB8888 values
- `Err(ImageError)` on failure

**Pixel format:** Each `u32` is `0xAARRGGBB` (alpha in high byte, blue in low byte). Opaque pixels have alpha = `0xFF`.

### format_name

```rust
pub fn format_name(format: u32) -> &'static str
```

Convert a format constant to a human-readable string.

| Input | Output |
|-------|--------|
| `FMT_BMP` | `"BMP"` |
| `FMT_PNG` | `"PNG"` |
| `FMT_JPEG` | `"JPEG"` |
| `FMT_GIF` | `"GIF"` |
| `FMT_MJV` | `"MJV"` |
| other | `"Unknown"` |

---

## ICO Functions

ICO files contain multiple image entries at different sizes. These functions let you select the best entry for a specific display size.

### probe_ico_size

```rust
pub fn probe_ico_size(data: &[u8], preferred_size: u32) -> Option<ImageInfo>
```

Probe an ICO file, selecting the best entry for a preferred display size.

**Parameters:**
- `data` -- Raw ICO file bytes
- `preferred_size` -- Desired icon dimension (e.g. 16, 32, 48, 64, 128, 256)

**Returns:**
- `Some(ImageInfo)` with the selected entry's dimensions and scratch buffer size
- `None` if the file is not a valid ICO

**Selection logic:** Prefers exact size match, then next-larger entry (downscaling is preferred over upscaling), then closest available.

### decode_ico_size

```rust
pub fn decode_ico_size(
    data: &[u8],
    preferred_size: u32,
    pixels: &mut [u32],
    scratch: &mut [u8],
) -> Result<(), ImageError>
```

Decode an ICO file, selecting the best entry for a preferred display size.

**Parameters:**
- `data` -- Raw ICO file bytes
- `preferred_size` -- Desired icon dimension
- `pixels` -- Output buffer, must have at least `width * height` elements (from `probe_ico_size()`)
- `scratch` -- Working memory, must have at least `scratch_needed` bytes (from `probe_ico_size()`)

**Returns:**
- `Ok(())` on success
- `Err(ImageError)` on failure

---

## Video Functions

### video_probe

```rust
pub fn video_probe(data: &[u8]) -> Option<VideoInfo>
```

Probe a video file to determine format, dimensions, frame rate, and frame count.

**Parameters:**
- `data` -- Raw video file bytes (at least 32 bytes required)

**Returns:**
- `Some(VideoInfo)` with width, height, fps, num_frames, and scratch_needed
- `None` if the format is not recognized

**Notes:**
- Currently only supports the MJV (Motion JPEG Video) format
- Probes the first JPEG frame internally to determine `scratch_needed`

### video_decode_frame

```rust
pub fn video_decode_frame(
    data: &[u8],
    num_frames: u32,
    frame_idx: u32,
    pixels: &mut [u32],
    scratch: &mut [u8],
) -> Result<(), ImageError>
```

Decode a single video frame into ARGB8888 pixels.

**Parameters:**
- `data` -- Raw video file bytes (entire .mjv file)
- `num_frames` -- Total frame count (from `video_probe()`)
- `frame_idx` -- Zero-based frame index to decode
- `pixels` -- Output buffer, must have at least `width * height` elements
- `scratch` -- Working memory, must have at least `scratch_needed` bytes (from `video_probe()`)

**Returns:**
- `Ok(())` on success
- `Err(ImageError)` on failure

**Notes:**
- Each frame is an independent JPEG, so frames can be decoded in any order
- The scratch buffer is reused across frame decodes (no need to reallocate)

---

## Scale Functions

### scale_image

```rust
pub fn scale_image(
    src: &[u32], src_w: u32, src_h: u32,
    dst: &mut [u32], dst_w: u32, dst_h: u32,
    mode: u32,
) -> bool
```

Scale an ARGB8888 image to a new size.

**Parameters:**
- `src` -- Source pixel buffer (`src_w * src_h` elements)
- `src_w`, `src_h` -- Source dimensions
- `dst` -- Destination pixel buffer (`dst_w * dst_h` elements)
- `dst_w`, `dst_h` -- Destination dimensions
- `mode` -- Scale mode: `MODE_SCALE` (0), `MODE_CONTAIN` (1), or `MODE_COVER` (2)

**Returns:**
- `true` on success
- `false` on error (null pointer, zero dimension, or invalid mode)

**Algorithm:**
- **Downscaling**: Area averaging (box filter) -- averages all source pixels that map to each destination pixel, preventing aliasing and detail loss
- **Upscaling**: Bilinear interpolation -- blends four neighboring source pixels using 16.16 fixed-point arithmetic (no floating point)

**Scale modes:**
- `MODE_SCALE` (0): Stretches source to fill destination exactly. Aspect ratio is not preserved.
- `MODE_CONTAIN` (1): Fits source within destination while preserving aspect ratio. Unused areas are filled with transparent black (`0x00000000`). The scaled image is centered in the destination.
- `MODE_COVER` (2): Fills destination while preserving aspect ratio. Excess source area is cropped (centered). No transparent pixels are produced.

---

## Encode Functions

### encode_bmp

```rust
pub fn encode_bmp(pixels: &[u32], width: u32, height: u32, out: &mut [u8]) -> Result<usize, ImageError>
```

Encode ARGB8888 pixels to BMP format.

**Parameters:**
- `pixels` -- Source ARGB8888 pixel buffer (`width * height` elements)
- `width`, `height` -- Image dimensions
- `out` -- Output buffer for BMP file data

**Returns:**
- `Ok(bytes_written)` on success
- `Err(ImageError)` on failure (buffer too small, invalid dimensions)

**Buffer size:** The output buffer should be at least `54 + width * height * 4` bytes (BMP header + 32-bit pixel data).

---

## Iconpack Functions

The icon pack system renders SVG icons from the binary `ico.pak` file containing 6000+ Tabler Icons in both filled and outline variants.

**Icon pack format (IPAK v2):**

| Offset | Size | Field |
|--------|------|-------|
| 0 | 4 | Magic: `IPAK` |
| 4 | 2 | Version (2) |
| 6 | 2 | Filled icon count |
| 8 | 2 | Outline icon count |
| 10 | 2 | Pre-rasterized icon size (pixels) |
| 12 | 4 | Names section offset |
| 16 | 4 | Data section offset |

Icons are looked up by name (e.g. `"device-floppy"`, `"folder-open"`) and rendered with a caller-specified color and size.

### iconpack_render

```rust
pub fn iconpack_render(
    pak: &[u8], name: &str, filled: bool, size: u32, color: u32, out: &mut [u32],
) -> Result<(), ImageError>
```

Render an icon from a caller-provided pak buffer.

**Parameters:**
- `pak` -- Raw ico.pak file data
- `name` -- Icon name (e.g. `"heart"`, `"folder-open"`)
- `filled` -- `true` for filled variant, `false` for outline
- `size` -- Desired output size in pixels (1-512)
- `color` -- ARGB color to apply (e.g. `0xFFCCCCCC`)
- `out` -- Output pixel buffer (`size * size` elements)

### iconpack_render_cached

```rust
pub fn iconpack_render_cached(
    name: &str, filled: bool, size: u32, color: u32, out: &mut [u32],
) -> Result<(), ImageError>
```

Render an icon using the DLL's internal ico.pak cache. The DLL lazy-loads `/System/media/ico.pak` on first call -- no client-side file reads needed. Preferred over `iconpack_render` for normal use.

**Parameters:**
- `name` -- Icon name (e.g. `"device-floppy"`, `"arrow-back-up"`)
- `filled` -- `true` for filled variant, `false` for outline
- `size` -- Desired output size in pixels (1-512)
- `color` -- ARGB color to apply
- `out` -- Output pixel buffer (`size * size` elements)

**Notes:**
- The pak file is loaded once and cached for the lifetime of the process
- If the requested size doesn't match the pre-rasterized size, the icon is scaled automatically
- Returns `Err(Unsupported)` if the icon name is not found or the pak file cannot be loaded
- Maximum pak file size: 2 MiB

---

## Format Support

### BMP

Windows Bitmap format.

| Feature | Supported |
|---------|-----------|
| 24-bit uncompressed (RGB) | Yes |
| 32-bit uncompressed (ARGB) | Yes |
| Bottom-up row order | Yes (standard) |
| Top-down row order | Yes |
| RLE compression | No |
| 1/4/8-bit palette | No |

**Scratch needed:** 0 bytes (decoded directly from file data)

### PNG

Portable Network Graphics.

| Feature | Supported |
|---------|-----------|
| 8-bit RGB | Yes |
| 8-bit RGBA | Yes |
| 8-bit Grayscale | Yes |
| Filter types 0-4 (None, Sub, Up, Average, Paeth) | Yes |
| DEFLATE decompression | Yes (fixed + dynamic Huffman) |
| Interlaced (Adam7) | No |
| 16-bit channels | No |
| Palette (indexed color) | No |

**Scratch needed:** ~33 KiB + `width * 4` (32 KiB sliding window + row buffer + Huffman tables)

### JPEG

JPEG/JFIF baseline format.

| Feature | Supported |
|---------|-----------|
| Baseline DCT (SOF0) | Yes |
| 4:4:4 chroma subsampling | Yes |
| 4:2:2 chroma subsampling | Yes |
| 4:2:0 chroma subsampling | Yes |
| YCbCr to RGB conversion | Yes (fixed-point integer math) |
| Huffman coding | Yes |
| Quantization tables | Yes |
| Progressive JPEG (SOF2) | No |
| Arithmetic coding | No |
| CMYK color space | No |

**IDCT:** Uses the LLM (Loeffler, Ligtenberg, Moschytz) fast integer IDCT algorithm.

**Scratch needed:** `width * height * 3 + 4096` bytes (decoded component buffers + tables)

### GIF

Graphics Interchange Format.

| Feature | Supported |
|---------|-----------|
| GIF87a | Yes |
| GIF89a | Yes |
| LZW decompression | Yes |
| Global color table | Yes |
| Local color table | Yes |
| Transparency (GCE) | Yes |
| Interlacing | Yes |
| Animation (multiple frames) | First frame only |

**Scratch needed:** `4096 * 4 + width * height` bytes (LZW string table + index buffer)

### ICO

Windows Icon format.

| Feature | Supported |
|---------|-----------|
| ICO (type 1) | Yes |
| CUR (type 2, cursor) | Yes |
| BMP-in-ICO: 32-bit BGRA | Yes |
| BMP-in-ICO: 24-bit BGR | Yes |
| BMP-in-ICO: 8-bit palette | Yes |
| BMP-in-ICO: 4-bit palette | Yes |
| BMP-in-ICO: 1-bit monochrome | Yes |
| PNG-in-ICO | Yes (delegates to PNG decoder) |
| AND mask transparency | Yes |
| Multi-size selection | Yes (probe_ico_size / decode_ico_size) |
| 256x256 entries (width byte = 0) | Yes |

**Scratch needed:** 0 bytes for BMP-in-ICO, PNG scratch for PNG-in-ICO entries

**Size selection:** `probe()` / `decode()` default to preferring a 16x16 entry. Use `probe_ico_size()` / `decode_ico_size()` to select by a preferred size.

### MJV (Video)

Motion JPEG Video -- a simple container for a sequence of JPEG frames.

**File format:**

| Offset | Size | Field |
|--------|------|-------|
| 0 | 4 | Magic: `MJV1` |
| 4 | 4 | Reserved |
| 8 | 4 | Width (pixels) |
| 12 | 4 | Height (pixels) |
| 16 | 4 | FPS (frames per second) |
| 20 | 4 | Number of frames |
| 24 | 8 | Reserved |
| 32 | 8 * N | Frame table: N entries of (offset: u32, size: u32) |
| variable | variable | Concatenated JPEG frame data |

Each frame is an independent baseline JPEG that can be decoded in any order.

**Scratch needed:** Same as JPEG: `width * height * 3 + 4096` bytes

---

## Error Handling

### Common Error Scenarios

| Error | Cause | Fix |
|-------|-------|-----|
| `probe()` returns `None` | File is not BMP/PNG/JPEG/GIF/ICO or too short | Check file format, ensure >= 8 bytes |
| `video_probe()` returns `None` | File is not MJV or too short | Check file format, ensure >= 32 bytes |
| `InvalidData` | Corrupt header, truncated file | Verify file integrity |
| `Unsupported` | Progressive JPEG, palette PNG, RLE BMP | Convert to supported format |
| `BufferTooSmall` | `pixels.len() < width * height` | Allocate `width * height` u32s |
| `ScratchTooSmall` | `scratch.len() < scratch_needed` | Use `scratch_needed` from `probe()` |

---

## Examples

### Display Image in a Window

```rust
use anyos_std::*;
use libimage_client;

fn show_image(win: u32, data: &[u8]) {
    let info = match libimage_client::probe(data) {
        Some(i) => i,
        None => return,
    };

    let count = (info.width * info.height) as usize;
    let mut pixels = vec![0u32; count];
    let mut scratch = vec![0u8; info.scratch_needed as usize];

    if libimage_client::decode(data, &mut pixels, &mut scratch).is_ok() {
        // Blit rows to window (64 rows at a time for efficiency)
        let w = info.width as u16;
        let h = info.height;
        let mut y = 0u32;
        while y < h {
            let rows = (h - y).min(64);
            let start = (y * info.width) as usize;
            let end = start + (rows * info.width) as usize;
            ui::window::blit(win, 0, y as i16, w, rows as u16, &pixels[start..end]);
            y += rows;
        }
        ui::window::present(win);
    }
}
```

### Load a Specific Icon Size

```rust
use libimage_client;

fn load_icon_48(data: &[u8]) -> Option<Vec<u32>> {
    let info = libimage_client::probe_ico_size(data, 48)?;
    let count = (info.width * info.height) as usize;
    let mut pixels = vec![0u32; count];
    let mut scratch = vec![0u8; info.scratch_needed as usize];

    libimage_client::decode_ico_size(data, 48, &mut pixels, &mut scratch).ok()?;
    Some(pixels)
}
```

### Play Video Frames

```rust
use anyos_std::*;
use libimage_client;

fn play_video(win: u32, data: &[u8]) {
    let info = match libimage_client::video_probe(data) {
        Some(i) => i,
        None => return,
    };

    let count = (info.width * info.height) as usize;
    let mut pixels = vec![0u32; count];
    let mut scratch = vec![0u8; info.scratch_needed as usize];
    let frame_ms = 1000 / info.fps;

    for frame in 0..info.num_frames {
        if libimage_client::video_decode_frame(
            data, info.num_frames, frame, &mut pixels, &mut scratch
        ).is_ok() {
            // Blit frame to window
            ui::window::blit(win, 0, 0, info.width as u16, info.height as u16, &pixels);
            ui::window::present(win);
        }
        process::sleep(frame_ms);
    }
}
```

### Scale Image to Fit Window

```rust
use libimage_client::{self, MODE_CONTAIN};

fn scale_to_window(src: &[u32], src_w: u32, src_h: u32, win_w: u32, win_h: u32) -> Vec<u32> {
    let mut dst = vec![0u32; (win_w * win_h) as usize];
    libimage_client::scale_image(src, src_w, src_h, &mut dst, win_w, win_h, MODE_CONTAIN);
    dst
}
```

### Detect Format Without Decoding

```rust
use libimage_client;

fn identify(data: &[u8]) -> &'static str {
    match libimage_client::probe(data) {
        Some(info) => libimage_client::format_name(info.format),
        None => "Unknown",
    }
}
```

### Raw DLL Access

For advanced use, you can access the export table directly:

```rust
use libimage_client::raw;

let exports = raw::exports();
let mut info = raw::ImageInfo {
    width: 0, height: 0, format: 0, scratch_needed: 0,
};

let ret = (exports.image_probe)(data.as_ptr(), data.len() as u32, &mut info);
if ret == 0 {
    // info is populated
}
```
