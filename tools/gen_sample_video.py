#!/usr/bin/env python3
# Copyright (c) 2024-2026 Christian Moeller
# SPDX-License-Identifier: MIT

"""
gen_sample_video.py - Generate a sample .mjv video for anyOS testing.

Creates a simple animation (bouncing colored ball) as JPEG frames
packed into MJV format. Requires Pillow for JPEG encoding.

Usage:
    python3 tools/gen_sample_video.py output.mjv [--width 160] [--height 120] [--fps 10] [--frames 30]
"""

import argparse
import io
import math
import struct
import sys

try:
    from PIL import Image
except ImportError:
    print("Error: Pillow required. Install with: pip3 install Pillow", file=sys.stderr)
    sys.exit(1)

MJV_MAGIC = b"MJV1"
MJV_HEADER_SIZE = 32


def hsv_to_rgb(h, s, v):
    """Convert HSV (h: 0-360, s/v: 0-1) to RGB (0-255 each)."""
    c = v * s
    x = c * (1 - abs((h / 60) % 2 - 1))
    m = v - c
    if h < 60:
        r, g, b = c, x, 0
    elif h < 120:
        r, g, b = x, c, 0
    elif h < 180:
        r, g, b = 0, c, x
    elif h < 240:
        r, g, b = 0, x, c
    elif h < 300:
        r, g, b = x, 0, c
    else:
        r, g, b = c, 0, x
    return int((r + m) * 255), int((g + m) * 255), int((b + m) * 255)


def generate_frame(width, height, frame_idx, total_frames):
    """Generate a single frame: dark background with bouncing colored ball."""
    img = Image.new("RGB", (width, height), (30, 30, 30))
    pixels = img.load()

    # Bouncing ball trajectory (Lissajous pattern)
    t = frame_idx / max(total_frames, 1)
    ball_r = min(width, height) // 6
    margin = ball_r + 2
    cx = int(margin + (width - 2 * margin) * (0.5 + 0.5 * math.sin(2 * math.pi * t)))
    cy = int(margin + (height - 2 * margin) * (0.5 + 0.5 * math.sin(3 * math.pi * t)))

    # Ball color cycles through hue
    hue = (frame_idx * 360 // max(total_frames, 1)) % 360
    br, bg, bb = hsv_to_rgb(hue, 0.85, 1.0)

    # Draw ball with soft edge
    for y in range(max(0, cy - ball_r), min(height, cy + ball_r + 1)):
        for x in range(max(0, cx - ball_r), min(width, cx + ball_r + 1)):
            dx = x - cx
            dy = y - cy
            dist_sq = dx * dx + dy * dy
            r_sq = ball_r * ball_r
            if dist_sq <= r_sq:
                # Smooth edge: alpha decreases near boundary
                dist = math.sqrt(dist_sq) / ball_r
                alpha = max(0.0, min(1.0, (1.0 - dist) * 3.0))
                px = pixels[x, y]
                pixels[x, y] = (
                    int(px[0] * (1 - alpha) + br * alpha),
                    int(px[1] * (1 - alpha) + bg * alpha),
                    int(px[2] * (1 - alpha) + bb * alpha),
                )

    # Add a simple frame counter text (small white dots for digits)
    # Just draw a small white bar at the bottom as a progress indicator
    progress_w = int((width - 4) * (frame_idx + 1) / total_frames)
    for x in range(2, 2 + progress_w):
        for y in range(height - 4, height - 2):
            pixels[x, y] = (100, 180, 255)

    # Encode to JPEG
    buf = io.BytesIO()
    img.save(buf, format="JPEG", quality=80)
    return buf.getvalue()


def main():
    parser = argparse.ArgumentParser(description="Generate sample .mjv video")
    parser.add_argument("output", help="Output .mjv file")
    parser.add_argument("--width", type=int, default=160, help="Frame width (default: 160)")
    parser.add_argument("--height", type=int, default=120, help="Frame height (default: 120)")
    parser.add_argument("--fps", type=int, default=10, help="Frames per second (default: 10)")
    parser.add_argument("--frames", type=int, default=30, help="Total frames (default: 30)")
    args = parser.parse_args()

    frame_data = []
    for i in range(args.frames):
        jpeg = generate_frame(args.width, args.height, i, args.frames)
        frame_data.append(jpeg)
        print(f"\rGenerating frame {i + 1}/{args.frames}...", end="", flush=True)
    print()

    # Build MJV
    table_size = args.frames * 8
    data_start = MJV_HEADER_SIZE + table_size

    header = struct.pack(
        "<4sIIIIIII",
        MJV_MAGIC, 1,
        args.width, args.height,
        args.fps, args.frames,
        0, 0,  # reserved
    )

    table = b""
    offset = data_start
    for fd in frame_data:
        table += struct.pack("<II", offset, len(fd))
        offset += len(fd)

    with open(args.output, "wb") as f:
        f.write(header)
        f.write(table)
        for fd in frame_data:
            f.write(fd)

    duration = args.frames / args.fps
    print(f"Written {args.output}: {offset:,} bytes ({offset / 1024:.1f} KiB), "
          f"{args.width}x{args.height} @ {args.fps} fps, {duration:.1f}s")


if __name__ == "__main__":
    main()
