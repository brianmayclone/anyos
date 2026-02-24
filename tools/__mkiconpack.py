#!/usr/bin/env python3
"""
mkiconpack.py — Pack SVG icons into a binary ico.pak file for anyOS.

Reads SVG files from assets/icons/svg/{filled,outline}/, extracts raw SVG
path d="" strings, and packs them into a single binary file with an index.

The SVG path strings are stored as-is (not pre-parsed) so the runtime
SVG path parser/rasterizer can scale to any size.

Usage:
    python3 tools/__mkiconpack.py

Binary format (ico.pak):

    Header (18 bytes):
        [0..4]   magic "IPAK"
        [4..6]   version u16 LE = 1
        [6..8]   filled_count u16
        [8..10]  outline_count u16
        [10..14] names_offset u32 (byte offset to names section)
        [14..18] data_offset u32 (byte offset to path data section)

    Index Table (N * 12 bytes, sorted by name within each group):
        First: filled_count entries
        Then:  outline_count entries
        Each entry (12 bytes):
            [0..4]   name_off: u32 (offset into names section)
            [4..6]   name_len: u16
            [6..8]   data_off_hi_and_path_count: u16
                     - bits 0..11: path_count (max 4095)
                     - bits 12..15: data_off bits 16..19 (overflow)
            [8..12]  data_off_lo_and_len: u32
                     - bits 0..15: data_off low 16 bits  ... no this is getting complex

    Actually simpler — 14 bytes per entry:
        [0..4]   name_off: u32 (offset into names section)
        [4..6]   name_len: u16
        [6..8]   path_count: u16
        [8..12]  data_off: u32 (offset into data section)
        [12..16] data_len: u32

    Names Section:
        Concatenated UTF-8 icon names (sorted, no separators — use name_len)

    Data Section:
        For each icon: raw SVG path d="" strings.
        Multiple paths per icon separated by \\0 (null byte).
"""

import os
import re
import struct
import sys
import xml.etree.ElementTree as ET

# ── SVG File Parser ──────────────────────────────────────────────────

def extract_svg_paths(filepath):
    """
    Extract raw SVG path d="" strings from an SVG file.
    Handles <g transform="translate(x,y)"> — prepends a synthetic
    M offset to the path (translate becomes part of the path string).
    Returns list of path d-strings.
    """
    try:
        tree = ET.parse(filepath)
    except ET.ParseError:
        return []

    root = tree.getroot()
    paths = []

    def process_element(elem, tx=0.0, ty=0.0):
        tag = elem.tag
        if '}' in tag:
            tag = tag.split('}', 1)[1]

        if tag == 'g':
            transform = elem.get('transform', '')
            gtx, gty = tx, ty
            m = re.match(r'translate\(\s*([^,\s]+)\s*,?\s*([^)]*)\)', transform)
            if m:
                gtx += float(m.group(1))
                gty += float(m.group(2)) if m.group(2).strip() else 0.0
            for child in elem:
                process_element(child, gtx, gty)
        elif tag == 'path':
            d = elem.get('d', '').strip()
            if d:
                # If there's a translate offset, we need to inform the runtime.
                # We encode it by wrapping the path: prepend a comment-like marker.
                # Actually: just store the translate as metadata prefix "T tx ty\n" + path
                if tx != 0.0 or ty != 0.0:
                    d = f"T{tx} {ty}\n{d}"
                paths.append(d)
        else:
            for child in elem:
                process_element(child, tx, ty)

    for child in root:
        process_element(child)

    return paths

# ── Collect icons from directory ─────────────────────────────────────

def collect_icons(svg_dir):
    """Collect SVG icons from a directory.
    Returns sorted list of (name, path_count, data_bytes)."""
    icons = []
    if not os.path.isdir(svg_dir):
        return icons

    for fname in sorted(os.listdir(svg_dir)):
        if not fname.endswith('.svg'):
            continue
        name = fname[:-4]
        fpath = os.path.join(svg_dir, fname)
        paths = extract_svg_paths(fpath)
        if not paths:
            continue

        # Join multiple paths with null separator
        data = b'\x00'.join(p.encode('utf-8') for p in paths)
        icons.append((name, len(paths), data))

    return icons

# ── Write .pak file ──────────────────────────────────────────────────

def write_pak(filled_icons, outline_icons, output_path):
    """Write the binary ico.pak file."""
    filled_count = len(filled_icons)
    outline_count = len(outline_icons)
    total = filled_count + outline_count

    names_blob = bytearray()
    data_blob = bytearray()
    entries = []

    for icons_list in [filled_icons, outline_icons]:
        for name, path_count, data in icons_list:
            name_bytes = name.encode('utf-8')
            name_off = len(names_blob)
            names_blob += name_bytes
            data_off = len(data_blob)
            data_blob += data
            entries.append((name_off, len(name_bytes), path_count, data_off, len(data)))

    # Layout
    header_size = 18
    index_entry_size = 16  # 4 + 2 + 2 + 4 + 4
    index_size = total * index_entry_size
    names_offset = header_size + index_size
    data_offset = names_offset + len(names_blob)

    with open(output_path, 'wb') as f:
        # Header (18 bytes)
        f.write(b'IPAK')                            # magic
        f.write(struct.pack('<H', 1))                # version
        f.write(struct.pack('<H', filled_count))
        f.write(struct.pack('<H', outline_count))
        f.write(struct.pack('<I', names_offset))
        f.write(struct.pack('<I', data_offset))

        # Index table (16 bytes per entry)
        for name_off, name_len, path_count, data_off, data_len in entries:
            f.write(struct.pack('<I', name_off))     # 4: name offset
            f.write(struct.pack('<H', name_len))     # 2: name length
            f.write(struct.pack('<H', path_count))   # 2: number of paths
            f.write(struct.pack('<I', data_off))     # 4: data offset
            f.write(struct.pack('<I', data_len))     # 4: data length

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

    print(f"Scanning filled icons: {filled_dir}")
    filled = collect_icons(filled_dir)
    print(f"  Found {len(filled)} filled icons")

    print(f"Scanning outline icons: {outline_dir}")
    outline = collect_icons(outline_dir)
    print(f"  Found {len(outline)} outline icons")

    if not filled and not outline:
        print("ERROR: No icons found!")
        sys.exit(1)

    print(f"\nWriting {output_path}")
    total_size = write_pak(filled, outline, output_path)
    print(f"  Total: {len(filled) + len(outline)} icons, {total_size:,} bytes ({total_size / 1024:.1f} KB)")

    filled_data = sum(len(d) for _, _, d in filled)
    outline_data = sum(len(d) for _, _, d in outline)
    print(f"  Filled path data:  {filled_data:,} bytes ({filled_data / 1024:.1f} KB)")
    print(f"  Outline path data: {outline_data:,} bytes ({outline_data / 1024:.1f} KB)")

    # Compare with original SVG sizes
    svg_total = 0
    for d in [filled_dir, outline_dir]:
        if os.path.isdir(d):
            for f in os.listdir(d):
                if f.endswith('.svg'):
                    svg_total += os.path.getsize(os.path.join(d, f))
    if svg_total > 0:
        ratio = total_size / svg_total * 100
        print(f"\n  Original SVGs: {svg_total:,} bytes ({svg_total / 1024:.1f} KB)")
        print(f"  Compression:   {ratio:.1f}% of original ({svg_total - total_size:,} bytes saved)")
        print(f"  File count:    {len(filled) + len(outline)} icons in 1 file (was {len(filled) + len(outline)} files)")

if __name__ == '__main__':
    main()
