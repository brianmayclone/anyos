#!/usr/bin/env python3
# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT

"""
Convert boot logo image (JPG/PNG) to raw RGBA binary for kernel inclusion.

Output format:
  [width: u32 LE] [height: u32 LE] [RGBA pixel data: width*height*4 bytes]

The logo is scaled to fit within a reasonable size for 1024x768 display
(roughly 200x200 max) while maintaining aspect ratio.
"""

import struct
import sys
import os

try:
    from PIL import Image
except ImportError:
    print("ERROR: Pillow is required. Install with: pip3 install Pillow", file=sys.stderr)
    sys.exit(1)

MAX_SIZE = 200  # Max dimension in pixels for the boot logo


def main():
    if len(sys.argv) < 3:
        print(f"Usage: {sys.argv[0]} <input_image> <output_bin>", file=sys.stderr)
        sys.exit(1)

    input_path = sys.argv[1]
    output_path = sys.argv[2]

    if not os.path.exists(input_path):
        print(f"ERROR: Input file not found: {input_path}", file=sys.stderr)
        sys.exit(1)

    img = Image.open(input_path).convert("RGBA")
    orig_w, orig_h = img.size

    # Scale to fit within MAX_SIZE while maintaining aspect ratio
    scale = min(MAX_SIZE / orig_w, MAX_SIZE / orig_h)
    if scale < 1.0:
        new_w = int(orig_w * scale)
        new_h = int(orig_h * scale)
        img = img.resize((new_w, new_h), Image.LANCZOS)

    width, height = img.size
    pixels = img.tobytes()  # RGBA order, row-major

    os.makedirs(os.path.dirname(output_path), exist_ok=True)

    with open(output_path, "wb") as f:
        f.write(struct.pack("<II", width, height))
        f.write(pixels)

    total_size = 8 + len(pixels)
    print(f"Boot logo: {input_path} ({orig_w}x{orig_h}) -> {output_path} ({width}x{height}, {total_size} bytes)")


if __name__ == "__main__":
    main()
