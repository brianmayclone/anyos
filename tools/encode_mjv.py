#!/usr/bin/env python3
# Copyright (c) 2024-2026 Christian Moeller
# SPDX-License-Identifier: MIT

"""
encode_mjv.py - Convert video files to MJV (Motion JPEG Video) format.

Uses ffmpeg to extract JPEG frames and packs them into the MJV container.

Usage:
    python3 tools/encode_mjv.py input.mp4 output.mjv [--fps 15] [--width 320] [--height 240] [--quality 80]
"""

import argparse
import os
import struct
import subprocess
import sys
import tempfile

MJV_MAGIC = b"MJV1"
MJV_VERSION = 1
MJV_HEADER_SIZE = 32
MJV_FRAME_ENTRY_SIZE = 8


def main():
    parser = argparse.ArgumentParser(description="Convert video to MJV format")
    parser.add_argument("input", help="Input video file")
    parser.add_argument("output", help="Output .mjv file")
    parser.add_argument("--fps", type=int, default=15, help="Frames per second (default: 15)")
    parser.add_argument("--width", type=int, default=320, help="Output width (default: 320)")
    parser.add_argument("--height", type=int, default=240, help="Output height (default: 240)")
    parser.add_argument("--quality", type=int, default=80, help="JPEG quality 1-100 (default: 80)")
    args = parser.parse_args()

    if not os.path.exists(args.input):
        print(f"Error: input file not found: {args.input}", file=sys.stderr)
        sys.exit(1)

    # Map quality 1-100 to ffmpeg qscale 31-1 (lower = better)
    qscale = max(1, min(31, (100 - args.quality) * 31 // 100 + 1))

    with tempfile.TemporaryDirectory() as tmpdir:
        # Extract JPEG frames using ffmpeg
        pattern = os.path.join(tmpdir, "frame_%06d.jpg")
        cmd = [
            "ffmpeg", "-i", args.input,
            "-vf", f"scale={args.width}:{args.height}",
            "-r", str(args.fps),
            "-q:v", str(qscale),
            "-y",
            pattern,
        ]
        print(f"Extracting frames: {args.width}x{args.height} @ {args.fps} fps, quality={args.quality}...")
        result = subprocess.run(cmd, capture_output=True)
        if result.returncode != 0:
            print(f"ffmpeg failed:\n{result.stderr.decode()}", file=sys.stderr)
            sys.exit(1)

        # Collect frame files
        frames = sorted([
            os.path.join(tmpdir, f) for f in os.listdir(tmpdir)
            if f.startswith("frame_") and f.endswith(".jpg")
        ])

        if not frames:
            print("Error: no frames extracted", file=sys.stderr)
            sys.exit(1)

        num_frames = len(frames)
        print(f"Extracted {num_frames} frames")

        # Read all frame data
        frame_data = []
        for f in frames:
            with open(f, "rb") as fh:
                frame_data.append(fh.read())

        # Build MJV file
        table_size = num_frames * MJV_FRAME_ENTRY_SIZE
        data_offset = MJV_HEADER_SIZE + table_size

        # Header: magic(4) + version(4) + width(4) + height(4) + fps(4) + num_frames(4) + reserved(8)
        header = struct.pack(
            "<4sIIIIIII",
            MJV_MAGIC, MJV_VERSION,
            args.width, args.height,
            args.fps, num_frames,
            0, 0,  # reserved
        )

        # Build frame table
        table = b""
        current_offset = data_offset
        for fd in frame_data:
            table += struct.pack("<II", current_offset, len(fd))
            current_offset += len(fd)

        # Write output
        with open(args.output, "wb") as out:
            out.write(header)
            out.write(table)
            for fd in frame_data:
                out.write(fd)

        total_size = current_offset
        duration = num_frames / args.fps
        print(f"Written {args.output}: {total_size:,} bytes ({total_size / 1024:.1f} KiB), {duration:.1f}s")


if __name__ == "__main__":
    main()
