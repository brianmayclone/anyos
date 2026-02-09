# anyOS Image Library (libimage) API Reference

The **libimage** DLL is a shared library for decoding image files into ARGB8888 pixel buffers. It supports BMP, PNG, JPEG, and GIF formats. The DLL is loaded at virtual address `0x04100000` and exports 2 functions via a C ABI function pointer table.

**Client crate:** `libimage_client`

---

## Table of Contents

- [Getting Started](#getting-started)
- [Types](#types)
- [Functions](#functions)
  - [probe](#probe)
  - [decode](#decode)
- [Format Support](#format-support)
  - [BMP](#bmp)
  - [PNG](#png)
  - [JPEG](#jpeg)
  - [GIF](#gif)
- [Error Handling](#error-handling)
- [Examples](#examples)

---

## Getting Started

### Dependencies

Add to your program's `Cargo.toml`:

```toml
[dependencies]
anyos_std = { path = "../../stdlib" }
libimage_client = { path = "../../programs/dll/libimage_client" }
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

The DLL is **stateless** and uses **no heap**. All memory is provided by the caller:

1. **Image data** (`&[u8]`): The raw file bytes (read from disk)
2. **Pixel buffer** (`&mut [u32]`): Output ARGB8888 pixels, `width * height` elements
3. **Scratch buffer** (`&mut [u8]`): Working memory for the decoder, size from `probe()`

This design avoids heap allocation inside the DLL, making it safe for use across process address spaces.

---

## Types

### ImageInfo

Returned by `probe()` with image metadata.

```rust
#[repr(C)]
pub struct ImageInfo {
    pub width: u32,          // Image width in pixels
    pub height: u32,         // Image height in pixels
    pub format: u32,         // Format identifier (see constants below)
    pub scratch_needed: u32, // Bytes of scratch buffer needed for decode()
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

### ImageError

Error type returned by `decode()`.

| Variant | Raw Code | Description |
|---------|----------|-------------|
| `InvalidData` | -1 | Null pointer, truncated file, or corrupt data |
| `Unsupported` | -2 | Unrecognized format or unsupported feature |
| `BufferTooSmall` | -3 | Pixel output buffer smaller than width*height |
| `ScratchTooSmall` | -4 | Scratch buffer smaller than `scratch_needed` |
| `Unknown(i32)` | other | Unexpected error code |

---

## Functions

### probe

```rust
pub fn probe(data: &[u8]) -> Option<ImageInfo>
```

Detect the image format from magic bytes and parse the header for dimensions.

**Parameters:**
- `data` -- Raw image file bytes (at least 8 bytes required)

**Returns:**
- `Some(ImageInfo)` with format, dimensions, and scratch buffer size
- `None` if the format is not recognized

**Notes:**
- Only reads the file header, does not decode pixel data
- The `scratch_needed` field tells you how large a scratch buffer to allocate for `decode()`
- BMP images need no scratch buffer (`scratch_needed = 0`)

### decode

```rust
pub fn decode(data: &[u8], pixels: &mut [u32], scratch: &mut [u8]) -> Result<(), ImageError>
```

Decode an image into ARGB8888 pixels.

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
| other | `"Unknown"` |

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

---

## Error Handling

### Common Error Scenarios

| Error | Cause | Fix |
|-------|-------|-----|
| `probe()` returns `None` | File is not BMP/PNG/JPEG/GIF or too short | Check file format, ensure >= 8 bytes |
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
