#!/usr/bin/env python3
"""
mkiconpack.py — Pack pre-rasterized SVG icons into a binary ico.pak (v2) for anyOS.

Reads SVG files from assets/icons/svg/{filled,outline}/, rasterizes each at
4× supersampling using cairosvg, downscales with Pillow LANCZOS, and stores
the alpha channel as raw 32×32 bitmaps.

Dependencies:
    pip install cairosvg Pillow

Usage:
    python3 tools/__mkiconpack.py

Binary format (ico.pak v2):

    Header (20 bytes):
        [0..4]   magic "IPAK"
        [4..6]   version u16 LE = 2
        [6..8]   filled_count u16
        [8..10]  outline_count u16
        [10..12] icon_size u16 (e.g. 32)
        [12..16] names_offset u32
        [16..20] data_offset u32

    Index Table (16 bytes per entry, sorted by name within each group):
        First: filled_count entries
        Then:  outline_count entries
        Each entry (16 bytes):
            [0..4]   name_off: u32 (offset into names section)
            [4..6]   name_len: u16
            [6..8]   reserved: u16 (0)
            [8..12]  data_off: u32 (offset into data section)
            [12..16] reserved: u32 (0)

    Names Section:
        Concatenated UTF-8 icon names (sorted, no separators — use name_len)

    Data Section:
        Per icon: icon_size × icon_size bytes of alpha data (u8).
        0 = fully transparent, 255 = fully opaque.
"""

import io
import os
import struct
import sys
import re

try:
    import cairosvg
except ImportError:
    print("ERROR: cairosvg not installed. Run: pip install cairosvg")
    sys.exit(1)

try:
    from PIL import Image, ImageFilter
except ImportError:
    print("ERROR: Pillow not installed. Run: pip install Pillow")
    sys.exit(1)

# Pre-rendered icon size (pixels). All runtime sizes are scaled from this.
ICON_SIZE = 32

# Supersampling factor for high-quality anti-aliasing.
SUPERSAMPLE = 4

# Optional Gaussian blur sigma for smoother edges (0 = disabled).
BLUR_SIGMA = 0.3

# ── Rasterize a single SVG file to an alpha map ─────────────────────

def patch_svg_linecap(svg_bytes):
    """Replace stroke-linecap="round" with "square" to avoid round bubbles
    at stroke endpoints / intersections."""
    return svg_bytes.replace(
        b'stroke-linecap="round"',
        b'stroke-linecap="square"',
    )

def rasterize_svg(filepath):
    """Rasterize an SVG file to a ICON_SIZE×ICON_SIZE alpha map.

    Returns bytes of length ICON_SIZE² (one u8 per pixel), or None on error.
    """
    render_size = ICON_SIZE * SUPERSAMPLE

    # Read SVG and patch stroke-linecap before rendering
    with open(filepath, 'rb') as f:
        svg_data = f.read()
    svg_data = patch_svg_linecap(svg_data)

    try:
        png_data = cairosvg.svg2png(
            bytestring=svg_data,
            output_width=render_size,
            output_height=render_size,
        )
    except Exception as e:
        print(f"  WARNING: cairosvg failed for {filepath}: {e}")
        return None

    img = Image.open(io.BytesIO(png_data)).convert('RGBA')

    # Extract alpha channel
    alpha = img.split()[3]

    # Downscale with LANCZOS (high-quality anti-aliased resampling)
    alpha = alpha.resize((ICON_SIZE, ICON_SIZE), Image.LANCZOS)

    # Optional Gaussian blur for softer edges
    if BLUR_SIGMA > 0:
        alpha = alpha.filter(ImageFilter.GaussianBlur(radius=BLUR_SIGMA))

    return alpha.tobytes()

# ── Collect icons from directory ─────────────────────────────────────

def collect_icons(svg_dir):
    """Collect and rasterize SVG icons from a directory.
    Returns sorted list of (name, alpha_bytes)."""
    icons = []
    if not os.path.isdir(svg_dir):
        return icons

    filenames = sorted((f for f in os.listdir(svg_dir) if f.endswith('.svg')),
                       key=lambda f: f[:-4].encode('utf-8'))
    total = len(filenames)

    for idx, fname in enumerate(filenames):
        name = fname[:-4]
        fpath = os.path.join(svg_dir, fname)
        alpha = rasterize_svg(fpath)
        if alpha is None:
            continue
        icons.append((name, alpha))

        # Progress indicator every 100 icons
        if (idx + 1) % 100 == 0 or idx + 1 == total:
            print(f"  [{idx + 1}/{total}] rasterized")

    return icons

# ── Write .pak v2 file ───────────────────────────────────────────────

def write_pak_v2(filled_icons, outline_icons, output_path):
    """Write the binary ico.pak v2 file with alpha maps."""
    filled_count = len(filled_icons)
    outline_count = len(outline_icons)
    total = filled_count + outline_count

    names_blob = bytearray()
    data_blob = bytearray()
    entries = []

    alpha_size = ICON_SIZE * ICON_SIZE

    for icons_list in [filled_icons, outline_icons]:
        for name, alpha_data in icons_list:
            assert len(alpha_data) == alpha_size, \
                f"Icon '{name}' alpha size {len(alpha_data)} != {alpha_size}"

            name_bytes = name.encode('utf-8')
            name_off = len(names_blob)
            names_blob += name_bytes
            data_off = len(data_blob)
            data_blob += alpha_data
            entries.append((name_off, len(name_bytes), data_off))

    # Layout
    header_size = 20
    index_entry_size = 16
    index_size = total * index_entry_size
    names_offset = header_size + index_size
    data_offset = names_offset + len(names_blob)

    with open(output_path, 'wb') as f:
        # Header (20 bytes)
        f.write(b'IPAK')                            # magic
        f.write(struct.pack('<H', 2))                # version = 2
        f.write(struct.pack('<H', filled_count))
        f.write(struct.pack('<H', outline_count))
        f.write(struct.pack('<H', ICON_SIZE))        # icon_size
        f.write(struct.pack('<I', names_offset))
        f.write(struct.pack('<I', data_offset))

        # Index table (16 bytes per entry)
        for name_off, name_len, data_off in entries:
            f.write(struct.pack('<I', name_off))     # 4: name offset
            f.write(struct.pack('<H', name_len))     # 2: name length
            f.write(struct.pack('<H', 0))            # 2: reserved
            f.write(struct.pack('<I', data_off))     # 4: data offset
            f.write(struct.pack('<I', 0))            # 4: reserved

        # Names section
        f.write(names_blob)

        # Data section
        f.write(data_blob)

    return header_size + index_size + len(names_blob) + len(data_blob)

# ── Main ─────────────────────────────────────────────────────────────

def main():
    script_dir = os.path.dirname(os.path.abspath(__file__))
    root_dir = os.path.dirname(script_dir)

    filled_dir = os.path.join(root_dir, 'assets', 'icons', 'svg', 'filled')
    outline_dir = os.path.join(root_dir, 'assets', 'icons', 'svg', 'outline')
    output_path = os.path.join(root_dir, 'sysroot', 'System', 'media', 'ico.pak')

    os.makedirs(os.path.dirname(output_path), exist_ok=True)

    print(f"Icon size: {ICON_SIZE}px (rendered at {ICON_SIZE * SUPERSAMPLE}px, "
          f"downscaled with LANCZOS)")
    if BLUR_SIGMA > 0:
        print(f"Gaussian blur: sigma={BLUR_SIGMA}")
    print()

    print(f"Scanning filled icons: {filled_dir}")
    filled = collect_icons(filled_dir)
    print(f"  Found {len(filled)} filled icons\n")

    print(f"Scanning outline icons: {outline_dir}")
    outline = collect_icons(outline_dir)
    print(f"  Found {len(outline)} outline icons\n")

    if not filled and not outline:
        print("ERROR: No icons found!")
        print(f"  Expected SVGs in:")
        print(f"    {filled_dir}")
        print(f"    {outline_dir}")
        sys.exit(1)

    print(f"Writing {output_path}")
    total_size = write_pak_v2(filled, outline, output_path)
    icon_count = len(filled) + len(outline)
    alpha_data_size = icon_count * ICON_SIZE * ICON_SIZE

    print(f"  Format:     IPAK v2 ({ICON_SIZE}×{ICON_SIZE} alpha maps)")
    print(f"  Icons:      {icon_count} ({len(filled)} filled + {len(outline)} outline)")
    print(f"  Alpha data: {alpha_data_size:,} bytes ({alpha_data_size / 1024:.1f} KB)")
    print(f"  Total file: {total_size:,} bytes ({total_size / 1024:.1f} KB)")

    # Compare with original SVG sizes
    svg_total = 0
    for d in [filled_dir, outline_dir]:
        if os.path.isdir(d):
            for f in os.listdir(d):
                if f.endswith('.svg'):
                    svg_total += os.path.getsize(os.path.join(d, f))
    if svg_total > 0:
        print(f"\n  Original SVGs: {svg_total:,} bytes ({svg_total / 1024:.1f} KB)")
        print(f"  Ratio: {total_size / svg_total * 100:.1f}% of original")

if __name__ == '__main__':
    main()
