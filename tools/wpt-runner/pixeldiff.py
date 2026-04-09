#!/usr/bin/env python3
"""
Pixel-diff comparison for WPT reftests.
Compares two PNG images and reports match percentage.

Usage: pixeldiff.py <test.png> <ref.png> [threshold]

Exit code 0 = match (within threshold), 1 = mismatch, 2 = error
Threshold: max percentage of differing pixels (default: 1.0%)
"""

import sys
import struct
import zlib
import os


def read_png(path):
    """Minimal PNG reader - returns (width, height, pixels) where pixels is list of (r,g,b,a)."""
    with open(path, 'rb') as f:
        sig = f.read(8)
        if sig != b'\x89PNG\r\n\x1a\n':
            raise ValueError(f"Not a PNG: {path}")

        chunks = []
        while True:
            hdr = f.read(8)
            if len(hdr) < 8:
                break
            length = struct.unpack('>I', hdr[:4])[0]
            ctype = hdr[4:8]
            data = f.read(length)
            f.read(4)  # CRC
            chunks.append((ctype, data))

        # Parse IHDR
        ihdr = [c for c in chunks if c[0] == b'IHDR'][0][1]
        width, height, bit_depth, color_type = struct.unpack('>IIBB', ihdr[:10])

        if bit_depth != 8:
            raise ValueError(f"Unsupported bit depth: {bit_depth}")

        # Decompress IDAT
        idat_data = b''.join(c[1] for c in chunks if c[0] == b'IDAT')
        raw = zlib.decompress(idat_data)

        # Determine bytes per pixel
        if color_type == 6:    # RGBA
            bpp = 4
        elif color_type == 2:  # RGB
            bpp = 3
        elif color_type == 0:  # Grayscale
            bpp = 1
        elif color_type == 4:  # Grayscale+Alpha
            bpp = 2
        else:
            raise ValueError(f"Unsupported color type: {color_type}")

        # Unfilter
        stride = width * bpp
        pixels = []
        prev_row = bytes(stride)
        pos = 0

        for y in range(height):
            filter_type = raw[pos]
            pos += 1
            row = bytearray(raw[pos:pos + stride])
            pos += stride

            if filter_type == 1:  # Sub
                for i in range(stride):
                    a = row[i - bpp] if i >= bpp else 0
                    row[i] = (row[i] + a) & 0xFF
            elif filter_type == 2:  # Up
                for i in range(stride):
                    row[i] = (row[i] + prev_row[i]) & 0xFF
            elif filter_type == 3:  # Average
                for i in range(stride):
                    a = row[i - bpp] if i >= bpp else 0
                    row[i] = (row[i] + (a + prev_row[i]) // 2) & 0xFF
            elif filter_type == 4:  # Paeth
                for i in range(stride):
                    a = row[i - bpp] if i >= bpp else 0
                    b = prev_row[i]
                    c = prev_row[i - bpp] if i >= bpp else 0
                    p = a + b - c
                    pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                    if pa <= pb and pa <= pc:
                        pr = a
                    elif pb <= pc:
                        pr = b
                    else:
                        pr = c
                    row[i] = (row[i] + pr) & 0xFF

            # Extract pixels
            for x in range(width):
                off = x * bpp
                if bpp == 4:
                    pixels.append((row[off], row[off+1], row[off+2], row[off+3]))
                elif bpp == 3:
                    pixels.append((row[off], row[off+1], row[off+2], 255))
                elif bpp == 1:
                    pixels.append((row[off], row[off], row[off], 255))
                elif bpp == 2:
                    pixels.append((row[off], row[off], row[off], row[off+1]))

            prev_row = bytes(row)

        return width, height, pixels


def compare(test_path, ref_path, threshold=1.0, tolerance=2):
    """Compare two PNGs. Returns (match, diff_pct, total_pixels, diff_pixels)."""
    try:
        tw, th, tp = read_png(test_path)
        rw, rh, rp = read_png(ref_path)
    except Exception as e:
        return False, 100.0, 0, 0, str(e)

    # Use minimum dimensions for comparison
    w = min(tw, rw)
    h = min(th, rh)

    total = w * h
    diff = 0

    for y in range(h):
        for x in range(w):
            ti = y * tw + x
            ri = y * rw + x
            tr, tg, tb, ta = tp[ti]
            rr, rg, rb, ra = rp[ri]
            # Allow small per-channel tolerance for anti-aliasing differences
            if (abs(tr - rr) > tolerance or abs(tg - rg) > tolerance or
                abs(tb - rb) > tolerance or abs(ta - ra) > tolerance):
                diff += 1

    # Size mismatch counts as diff
    if tw != rw or th != rh:
        size_diff = abs(tw * th - rw * rh)
        diff += size_diff
        total = max(tw * th, rw * rh)

    pct = (diff / total * 100) if total > 0 else 100.0
    match = pct <= threshold
    return match, pct, total, diff, None


def main():
    if len(sys.argv) < 3:
        print(f"Usage: {sys.argv[0]} <test.png> <ref.png> [threshold%]")
        sys.exit(2)

    test_path = sys.argv[1]
    ref_path = sys.argv[2]
    threshold = float(sys.argv[3]) if len(sys.argv) > 3 else 1.0

    if not os.path.exists(test_path):
        print(f"SKIP: test image not found: {test_path}")
        sys.exit(2)
    if not os.path.exists(ref_path):
        print(f"SKIP: reference image not found: {ref_path}")
        sys.exit(2)

    match, pct, total, diff, err = compare(test_path, ref_path, threshold)

    if err:
        print(f"ERROR: {err}")
        sys.exit(2)

    status = "PASS" if match else "FAIL"
    print(f"{status}: {pct:.2f}% diff ({diff}/{total} pixels)")
    sys.exit(0 if match else 1)


if __name__ == '__main__':
    main()
