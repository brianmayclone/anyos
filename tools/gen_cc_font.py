#!/usr/bin/env python3
"""
Cape Coral Font Generator — rasterize TTF into binary .ccf font data.

Reads a TrueType font file and rasterizes it at multiple sizes using Pillow.
Outputs a binary .ccf file that can be embedded in the kernel.

Binary format (all values little-endian):

Header:
  magic: [u8; 4] = "CCF\0"
  num_sizes: u32

Per size block:
  font_size: u16
  line_height: u16
  ascent: i16
  descent: i16
  num_glyphs: u16
  _padding: u16

  Glyph table (num_glyphs entries, sorted by codepoint):
    codepoint: u32
    advance_width: u16
    bitmap_width: u16
    bitmap_height: u16
    x_offset: i16
    y_offset: i16 (from top of line, positive = down)
    bitmap_offset: u32 (byte offset from start of coverage data block)

  Coverage data: u8[] (one byte per pixel, 0=transparent, 255=opaque)
"""

import struct
import sys
import os

try:
    from PIL import Image, ImageDraw, ImageFont
except ImportError:
    print("ERROR: Pillow is required. Install with: pip3 install Pillow", file=sys.stderr)
    sys.exit(1)

# Sizes to rasterize (in pixels)
SIZES = [13, 16, 20, 24]

# Character range: ASCII 32-126
FIRST_CHAR = 32
LAST_CHAR = 126


def rasterize_font(ttf_path, sizes):
    """Rasterize a TTF at multiple sizes, return binary .ccf data."""
    result = bytearray()

    # Header
    result.extend(b"CCF\x00")  # magic
    result.extend(struct.pack("<I", len(sizes)))  # num_sizes

    for size in sizes:
        font = ImageFont.truetype(ttf_path, size)

        # Get font metrics
        # Use a reference string to measure ascent/descent
        ascent, descent = font.getmetrics()
        line_height = ascent + descent

        glyphs = []
        coverage_data = bytearray()

        for codepoint in range(FIRST_CHAR, LAST_CHAR + 1):
            ch = chr(codepoint)

            # Get glyph bounding box and advance
            bbox = font.getbbox(ch)
            if bbox is None:
                # No glyph for this character, use space metrics
                adv = size // 4
                glyphs.append({
                    'codepoint': codepoint,
                    'advance_width': adv,
                    'bitmap_width': 0,
                    'bitmap_height': 0,
                    'x_offset': 0,
                    'y_offset': 0,
                    'bitmap_offset': len(coverage_data),
                })
                continue

            x0, y0, x1, y1 = bbox
            bw = x1 - x0
            bh = y1 - y0

            # Get advance width using getlength
            adv = int(round(font.getlength(ch)))
            if adv < 1:
                adv = bw + 1

            if bw <= 0 or bh <= 0:
                # Whitespace character (space, etc.)
                glyphs.append({
                    'codepoint': codepoint,
                    'advance_width': adv,
                    'bitmap_width': 0,
                    'bitmap_height': 0,
                    'x_offset': 0,
                    'y_offset': 0,
                    'bitmap_offset': len(coverage_data),
                })
                continue

            # Render glyph to grayscale image
            img = Image.new("L", (bw, bh), 0)
            draw = ImageDraw.Draw(img)
            draw.text((-x0, -y0), ch, fill=255, font=font)

            # Extract coverage data (grayscale values)
            pixels = img.tobytes()

            glyphs.append({
                'codepoint': codepoint,
                'advance_width': adv,
                'bitmap_width': bw,
                'bitmap_height': bh,
                'x_offset': x0,
                'y_offset': y0,  # offset from top of line
                'bitmap_offset': len(coverage_data),
            })
            coverage_data.extend(pixels)

        # Write size block header
        result.extend(struct.pack("<HH", size, line_height))
        result.extend(struct.pack("<hh", ascent, descent))
        result.extend(struct.pack("<HH", len(glyphs), 0))  # num_glyphs, padding

        # Write glyph table
        for g in glyphs:
            result.extend(struct.pack("<I", g['codepoint']))
            result.extend(struct.pack("<HHH", g['advance_width'], g['bitmap_width'], g['bitmap_height']))
            result.extend(struct.pack("<hh", g['x_offset'], g['y_offset']))
            result.extend(struct.pack("<I", g['bitmap_offset']))

        # Write coverage data
        result.extend(coverage_data)

    return bytes(result)


def main():
    if len(sys.argv) < 3:
        print(f"Usage: {sys.argv[0]} <input.ttf> <output.ccf>", file=sys.stderr)
        sys.exit(1)

    ttf_path = sys.argv[1]
    output_path = sys.argv[2]

    if not os.path.exists(ttf_path):
        print(f"ERROR: Font file not found: {ttf_path}", file=sys.stderr)
        sys.exit(1)

    data = rasterize_font(ttf_path, SIZES)

    os.makedirs(os.path.dirname(output_path), exist_ok=True)
    with open(output_path, "wb") as f:
        f.write(data)

    print(f"Cape Coral font: {ttf_path} -> {output_path}")
    print(f"  Sizes: {SIZES}")
    print(f"  Characters: {FIRST_CHAR}-{LAST_CHAR} ({LAST_CHAR - FIRST_CHAR + 1} glyphs)")
    print(f"  Total size: {len(data)} bytes ({len(data) / 1024:.1f} KiB)")


if __name__ == "__main__":
    main()
