# anyOS Font Library (libfont) API Reference

The **libfont** shared library provides TrueType font loading and text rendering into pixel buffers. It supports greyscale and LCD subpixel anti-aliasing with **size-adaptive gamma correction** for optimal readability on dark backgrounds.

**Format:** ELF64 shared object (.so), loaded on demand via `SYS_DLL_LOAD`
**Load Address:** `0x05000000`
**Exports:** 9
**Client crate:** `libfont_client` (uses `dynlink::dl_open` / `dl_sym`)

## fontd — Font Server Architecture

System fonts are managed by the **fontd** daemon (`system/fontd/`). fontd loads TTF files from `/System/fonts/` into shared memory (SHM) on demand. All processes share the same physical font data pages — fonts are loaded exactly **once** from disk regardless of how many processes use them.

### Boot sequence

1. **Compositor spawns fontd** before calling `libfont_client::init()`
2. **Compositor subscribes** to the `"fontd"` event channel before spawning fontd (avoids race)
3. **fontd creates** the `"fontd"` channel and emits `EVT_FONTD_READY` (0x6000)
4. **Compositor receives** the ready signal, then calls `font_init()`
5. **`font_init()`** requests sfpro.ttf and andale-mono.ttf from fontd (immediate load)
6. **Bold, thin, italic, emoji** are registered as lazy — loaded on first use

### Font loading flow

```
Client process                    fontd
     |                              |
     |--- CMD_LOAD_BY_NAME -------->|   (filename in SHM, e.g. "sfpro.ttf")
     |                              |
     |                         [check cache]
     |                              |--- cache hit: return existing SHM ID
     |                              |--- cache miss: read from /System/fonts/,
     |                              |    create SHM, copy data, cache it
     |                              |
     |<-- EVT_FONT_READY ----------|   (shm_id, data_size)
     |                              |
     |--- shm_map(shm_id) -------> kernel
     |<-- virtual address ---------|
     |                              |
     | TtfFont::parse_static(data)  |   (zero-copy, data lives in SHM)
```

### fontd IPC Protocol (event channel: `"fontd"`)

#### CMD_LOAD_BY_NAME (0x6001)
Load a system font by filename from `/System/fonts/`.

| Field | Description |
|-------|-------------|
| evt[0] | `0x6001` |
| evt[1] | requester sub_id (for directed response) |
| evt[2] | shm_id containing filename (null-terminated, e.g. `"sfpro.ttf\0"`) |

#### CMD_LOAD_BY_PATH (0x6003)
Load a font by absolute path (for user-installed fonts).

| Field | Description |
|-------|-------------|
| evt[0] | `0x6003` |
| evt[1] | requester sub_id |
| evt[2] | shm_id containing full path (null-terminated) |

#### EVT_FONT_READY (0x6002) — Response
Sent back to the requester after loading.

| Field | Description |
|-------|-------------|
| evt[0] | `0x6002` |
| evt[1] | shm_id with font data (0 = failed) |
| evt[2] | data size in bytes |

#### CMD_LIST_FONTS (0x6005)
List all available system fonts.

| Field | Description |
|-------|-------------|
| evt[0] | `0x6005` |
| evt[1] | requester sub_id |

Response: `EVT_FONT_LIST` (0x6006) with shm_id containing newline-separated filenames.

### fontd source structure

```
system/fontd/
├── src/
│   ├── main.rs       — Event loop, spawns cache, emits EVT_FONTD_READY
│   ├── protocol.rs   — IPC command dispatch, SHM string I/O
│   ├── cache.rs      — Path → SHM-ID cache (64 slots, lifetime of fontd)
│   └── loader.rs     — Read font file from disk into SHM region
├── build.rs
└── Cargo.toml
```

### Fallback behavior

If fontd is not running (e.g. early boot, host build), libfont falls back to loading fonts directly from disk via `read_file("/System/fonts/...")`. This means the system is always functional, just slower without fontd because each process loads its own copy.

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

// Initialize (connects to fontd, loads system fonts via SHM)
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

Initialize the font subsystem. Loads `libfont.so` via `dl_open`, resolves symbols, and calls `font_init()` which:
1. Connects to the `"fontd"` event channel
2. Requests sfpro.ttf (ID 0) and andale-mono.ttf (ID 4) immediately via SHM
3. Registers bold, thin, italic, and emoji as lazy (loaded on first use)
4. Falls back to direct disk loading if fontd is not available
5. Initializes gamma correction LUTs and auto-detects subpixel capability

Must be called once before any other font operations. Returns `true` on success.

#### Gamma Correction

During init, two 256-byte lookup tables are computed for size-adaptive gamma correction:

| Font Size | LUT | Effect |
|-----------|-----|--------|
| ≤ 14 px | Strong (`GAMMA_LUT_S`) | ~50% coverage boost for thin strokes |
| 15–24 px | Moderate (`GAMMA_LUT_M`) | ~33% boost — balanced readability |
| > 24 px | Identity (no LUT) | Large text has sufficient stroke width |

---

### `load(path) -> Option<u32>`

Load a custom TTF font from a filesystem path (reads from disk).

| Parameter | Type | Description |
|-----------|------|-------------|
| path | `&str` | Filesystem path to `.ttf` file |
| **Returns** | `Option<u32>` | Font ID on success, `None` on failure |

Font IDs 0–5 are the system fonts (see table below).

---

### `load_data(data) -> Option<u32>`

Load a custom TTF font from raw byte data in memory (no disk I/O).

| Parameter | Type | Description |
|-----------|------|-------------|
| data | `&[u8]` | Raw TTF font file data |
| **Returns** | `Option<u32>` | Font ID on success, `None` on failure |

Useful for loading fonts from archives, network responses, or embedded resources.

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

---

### `draw_string_buf_clipped(...)`

Same as `draw_string_buf` but with clip rectangle (clip_x, clip_y, clip_r, clip_b).

---

### `line_height(font_id, size) -> u32`

Get the line height for a font at a given size.

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
| enabled | `bool` | `true` = LCD subpixel, `false` = greyscale only |

---

## System Fonts

| ID | Font | File | Size | Loading |
|----|------|------|------|---------|
| 0 | SF Pro | sfpro.ttf | 5.9 MB | Immediate (via fontd SHM) |
| 1 | SF Pro Bold | sfpro-bold.ttf | 3.4 MB | Lazy (on first use) |
| 2 | SF Pro Thin | sfpro-thin.ttf | 3.4 MB | Lazy |
| 3 | SF Pro Italic | sfpro-italic.ttf | 3.3 MB | Lazy |
| 4 | Andale Mono | andale-mono.ttf | 108 KB | Immediate (via fontd SHM) |
| 5 | Noto Color Emoji | NotoColorEmoji.ttf | 11 MB | Lazy |

**Immediate fonts** (sfpro + mono) are loaded at startup so text rendering works without delay.
**Lazy fonts** are loaded on first access — bold when bold text is first rendered, emoji when an emoji glyph is first encountered.

All fonts are served from fontd via shared memory. Each font file is read from disk exactly once, regardless of how many processes use it.

## Architecture

```
                  ┌──────────┐
                  │  fontd   │  (system daemon, started by compositor)
                  │          │
                  │ SHM pool │  sfpro.ttf → SHM #2
                  │          │  andale-mono.ttf → SHM #4
                  │          │  sfpro-bold.ttf → SHM #27 (lazy)
                  └────┬─────┘
                       │ IPC (event channel "fontd")
          ┌────────────┼────────────┐
          │            │            │
    ┌─────┴─────┐ ┌───┴───┐ ┌─────┴─────┐
    │Compositor │ │ Dock  │ │  Finder   │  ...
    │           │ │       │ │           │
    │ libfont.so│ │libfont│ │ libfont   │
    │ (per-proc)│ │(.so)  │ │ (.so)     │
    │           │ │       │ │           │
    │ shm_map(2)│ │shm(2) │ │ shm(2)   │  ← same physical pages
    └───────────┘ └───────┘ └───────────┘
```

- **fontd**: Daemon that owns font SHM regions. Loads fonts from disk on first request, caches them for the lifetime of the process. Supports up to 64 cached fonts.
- **libfont.so**: Per-process DLL. Each process has its own FontManager, glyph cache, and gamma LUTs, but font **data** is shared via SHM.
- **libfont_client**: Thin wrapper crate that loads libfont.so and provides safe Rust types.
- **libanyui.so**: Loads libfont.so internally for text rendering in controls. Triggers `ensure_init()` → `font_init()` on first text draw.
