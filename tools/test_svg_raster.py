#!/usr/bin/env python3
"""
Test the SVG rasterizer algorithm with a known icon path.
Outputs a PGM (grayscale) image to visually verify correctness.

Usage: python3 tools/test_svg_raster.py
"""

import sys

FP = 8
FP_ONE = 1 << FP

def fp(v): return v << FP
def fp_floor(v): return v >> FP
def fp_ceil(v): return (v + FP_ONE - 1) >> FP
def fp_mul(a, b): return (a * b) >> FP

def viewbox_to_px(vx, vy, scale):
    px = (vx * scale) // (24 * 256)
    py = (vy * scale) // (24 * 256)
    return px, py

def isqrt_fp(v):
    if v <= 0: return 0
    v64 = v * 256
    # Start above the root: max(v, 256) >= sqrt(v*256) always
    x = max(v, 256)
    for _ in range(24):
        nx = (x + v64 // x) // 2
        if nx >= x: break
        x = nx
    return x

# ── Edge structure ───────────────────────────

class Edge:
    def __init__(self, x0, y0, x1, y1, winding):
        self.x0 = x0; self.y0 = y0
        self.x1 = x1; self.y1 = y1
        self.winding = winding

def add_fill_edge(x0, y0, x1, y1, edges):
    if y0 == y1: return
    if y0 < y1:
        edges.append(Edge(x0, y0, x1, y1, 1))
    else:
        edges.append(Edge(x1, y1, x0, y0, -1))

# ── Stroke segment (rectangle, no caps) ─────

def stroke_segment_no_caps(x0, y0, x1, y1, hw, edges):
    dx = x1 - x0
    dy = y1 - y0
    len_sq_div256 = (dx * dx + dy * dy) // 256
    length = isqrt_fp(len_sq_div256)
    if length == 0: return

    nx = (-dy * hw) // length
    ny = (dx * hw) // length

    ax = x0 + nx; ay = y0 + ny
    bx = x0 - nx; by_ = y0 - ny
    cx = x1 - nx; cy = y1 - ny
    dx_ = x1 + nx; dy_ = y1 + ny

    add_fill_edge(ax, ay, dx_, dy_, edges)
    add_fill_edge(dx_, dy_, cx, cy, edges)
    add_fill_edge(cx, cy, bx, by_, edges)
    add_fill_edge(bx, by_, ax, ay, edges)

# ── Round cap (octagon) ─────────────────────

def add_round_cap(cx, cy, r, edges):
    r7 = r * 181 // 256
    pts = [
        (cx + r, cy), (cx + r7, cy + r7), (cx, cy + r), (cx - r7, cy + r7),
        (cx - r, cy), (cx - r7, cy - r7), (cx, cy - r), (cx + r7, cy - r7),
    ]
    for j in range(8):
        ax, ay = pts[j]
        bx, by = pts[(j + 1) % 8]
        add_fill_edge(ax, ay, bx, by, edges)

# ── Rasterizer ──────────────────────────────

def rasterize_edges(edges, width, height):
    w, h = width, height
    accum = [0] * (w * h)

    for edge in edges:
        ey0 = edge.y0
        ey1 = edge.y1
        if ey0 >= ey1: continue

        row_start = max(fp_floor(ey0), 0)
        row_end = min(fp_ceil(ey1), height)
        row_end = max(row_end, 0)
        if row_start >= row_end: continue

        dy = ey1 - ey0
        dx_total = edge.x1 - edge.x0

        for row in range(row_start, row_end):
            y_top = max(row * FP_ONE, ey0)
            y_bot = min((row + 1) * FP_ONE, ey1)
            if y_top >= y_bot: continue

            row_coverage = y_bot - y_top
            x_at_top = edge.x0 + dx_total * (y_top - ey0) // dy
            x_at_bot = edge.x0 + dx_total * (y_bot - ey0) // dy
            x_avg = (x_at_top + x_at_bot) // 2
            col = fp_floor(x_avg)
            frac_x = x_avg - (col << FP)
            full = edge.winding * row_coverage

            if 0 <= col < w:
                idx = row * w + col
                c0 = fp_mul(full, FP_ONE - frac_x)
                accum[idx] += c0
                if col + 1 < w:
                    accum[idx + 1] += full - c0
            elif col < 0 and w > 0:
                accum[row * w] += full

    coverage = [0] * (w * h)
    for row in range(h):
        base = row * w
        s = 0
        for col in range(w):
            s += accum[base + col]
            v = abs(s)
            coverage[base + col] = min(v, 255)
    return coverage

# ── Test: player-play icon ──────────────────

def test_player_play(size=24):
    """Path: M7 4v16l13 -8l-13 -8 (triangle)"""
    scale = fp(size)

    # Parse the path manually
    # M7 4 → MoveTo(7, 4) in viewbox
    # v16 → LineTo(7, 20)
    # l13 -8 → LineTo(20, 12)
    # l-13 -8 → LineTo(7, 4)

    points_vb = [(7, 4), (7, 20), (20, 12), (7, 4)]
    points_px = [viewbox_to_px(fp(x), fp(y), scale) for x, y in points_vb]

    print(f"Player-play triangle (size={size}):")
    for i, (px, py) in enumerate(points_px):
        vx, vy = points_vb[i]
        print(f"  Point {i}: viewbox=({vx},{vy}) -> pixel_fp=({px},{py}) = ({px/256:.1f}, {py/256:.1f})")

    # Stroke half-width
    hw = fp_mul(fp(1), scale) // 24
    print(f"  Stroke half-width (fp): {hw} = {hw/256:.2f} pixels")

    # Collect stroke edges
    edges = []
    # Start cap
    add_round_cap(points_px[0][0], points_px[0][1], hw, edges)
    # Segments
    for i in range(len(points_px) - 1):
        x0, y0 = points_px[i]
        x1, y1 = points_px[i + 1]
        if x0 != x1 or y0 != y1:
            stroke_segment_no_caps(x0, y0, x1, y1, hw, edges)
    # End cap (same point as start since path closes)
    add_round_cap(points_px[-1][0], points_px[-1][1], hw, edges)

    print(f"  Total edges: {len(edges)}")

    # Rasterize
    coverage = rasterize_edges(edges, size, size)

    # Print as ASCII art
    print(f"\n  Coverage map ({size}x{size}):")
    for row in range(size):
        line = "  "
        for col in range(size):
            v = coverage[row * size + col]
            if v == 0:
                line += " ."
            elif v < 64:
                line += " ░"
            elif v < 192:
                line += " ▒"
            else:
                line += " █"
        print(line)

    # Write PGM file
    with open("/tmp/player_play.pgm", "w") as f:
        f.write(f"P2\n{size} {size}\n255\n")
        for row in range(size):
            vals = [str(coverage[row * size + col]) for col in range(size)]
            f.write(" ".join(vals) + "\n")
    print(f"\n  Written to /tmp/player_play.pgm")

def test_fill_player_play(size=24):
    """Test FILL rendering of the same triangle"""
    scale = fp(size)
    points_vb = [(7, 4), (7, 20), (20, 12), (7, 4)]
    points_px = [viewbox_to_px(fp(x), fp(y), scale) for x, y in points_vb]

    edges = []
    for i in range(len(points_px) - 1):
        x0, y0 = points_px[i]
        x1, y1 = points_px[i + 1]
        if y0 != y1:  # skip horizontal
            if y0 < y1:
                edges.append(Edge(x0, y0, x1, y1, 1))
            else:
                edges.append(Edge(x1, y1, x0, y0, -1))

    print(f"\nFill test (same triangle):")
    print(f"  Total edges: {len(edges)}")

    coverage = rasterize_edges(edges, size, size)

    print(f"  Coverage map ({size}x{size}):")
    for row in range(size):
        line = "  "
        for col in range(size):
            v = coverage[row * size + col]
            if v == 0:
                line += " ."
            elif v < 64:
                line += " ░"
            elif v < 192:
                line += " ▒"
            else:
                line += " █"
        print(line)

def test_simple_line(size=24):
    """Just a simple horizontal line: M4 12 H20"""
    scale = fp(size)
    hw = fp_mul(fp(1), scale) // 24

    p0 = viewbox_to_px(fp(4), fp(12), scale)
    p1 = viewbox_to_px(fp(20), fp(12), scale)

    print(f"\nSimple horizontal line (size={size}):")
    print(f"  From pixel ({p0[0]/256:.1f}, {p0[1]/256:.1f}) to ({p1[0]/256:.1f}, {p1[1]/256:.1f})")
    print(f"  Stroke half-width: {hw/256:.2f} pixels")

    edges = []
    add_round_cap(p0[0], p0[1], hw, edges)
    stroke_segment_no_caps(p0[0], p0[1], p1[0], p1[1], hw, edges)
    add_round_cap(p1[0], p1[1], hw, edges)

    print(f"  Total edges: {len(edges)}")

    coverage = rasterize_edges(edges, size, size)

    print(f"  Coverage map ({size}x{size}):")
    for row in range(size):
        line = "  "
        for col in range(size):
            v = coverage[row * size + col]
            if v == 0:
                line += " ."
            elif v < 64:
                line += " ░"
            elif v < 192:
                line += " ▒"
            else:
                line += " █"
        print(line)

def debug_horizontal_line():
    """Debug why horizontal line fills entire canvas"""
    size = 24
    scale = fp(size)
    hw = fp_mul(fp(1), scale) // 24

    p0 = viewbox_to_px(fp(4), fp(12), scale)
    p1 = viewbox_to_px(fp(20), fp(12), scale)

    print(f"\n=== DEBUG horizontal line ===")
    print(f"p0={p0}, p1={p1}, hw={hw}")

    edges = []
    add_round_cap(p0[0], p0[1], hw, edges)
    stroke_segment_no_caps(p0[0], p0[1], p1[0], p1[1], hw, edges)
    add_round_cap(p1[0], p1[1], hw, edges)

    print(f"Total edges: {len(edges)}")
    for i, e in enumerate(edges):
        y_start_row = fp_floor(e.y0)
        y_end_row = fp_ceil(e.y1)
        print(f"  Edge {i}: ({e.x0},{e.y0})->({e.x1},{e.y1}) w={e.winding} rows={y_start_row}-{y_end_row}")

    # Check accumulator for row 0
    w, h = size, size
    accum = [0] * (w * h)

    for edge in edges:
        ey0 = edge.y0
        ey1 = edge.y1
        if ey0 >= ey1: continue

        row_start = max(fp_floor(ey0), 0)
        row_end = min(fp_ceil(ey1), h)
        row_end = max(row_end, 0)
        if row_start >= row_end: continue

        dy = ey1 - ey0
        dx_total = edge.x1 - edge.x0

        for row in range(row_start, row_end):
            y_top = max(row * FP_ONE, ey0)
            y_bot = min((row + 1) * FP_ONE, ey1)
            if y_top >= y_bot: continue

            row_coverage = y_bot - y_top
            x_at_top = edge.x0 + dx_total * (y_top - ey0) // dy
            x_at_bot = edge.x0 + dx_total * (y_bot - ey0) // dy
            x_avg = (x_at_top + x_at_bot) // 2
            col = fp_floor(x_avg)
            frac_x = x_avg - (col << FP)
            full = edge.winding * row_coverage

            if row <= 1:  # Debug first two rows
                print(f"  Row {row}: edge ({edge.x0},{edge.y0})->({edge.x1},{edge.y1}) w={edge.winding}")
                print(f"    y_top={y_top} y_bot={y_bot} coverage={row_coverage}")
                print(f"    x_avg={x_avg} col={col} frac_x={frac_x} full={full}")

            if 0 <= col < w:
                idx = row * w + col
                c0 = fp_mul(full, FP_ONE - frac_x)
                accum[idx] += c0
                if col + 1 < w:
                    accum[idx + 1] += full - c0
            elif col < 0 and w > 0:
                accum[row * w] += full

    print(f"\nAccum row 0: {accum[0:24]}")
    print(f"Accum row 11: {accum[11*24:12*24]}")
    print(f"Accum row 12: {accum[12*24:13*24]}")

    # Prefix sum
    coverage = [0] * (w * h)
    for row in range(h):
        base = row * w
        s = 0
        for col in range(w):
            s += accum[base + col]
            v = abs(s)
            coverage[base + col] = min(v, 255)

    print(f"\nCoverage row 0: {coverage[0:24]}")
    print(f"Coverage row 5: {coverage[5*24:6*24]}")
    print(f"Coverage row 11: {coverage[11*24:12*24]}")
    print(f"Coverage row 12: {coverage[12*24:13*24]}")

if __name__ == "__main__":
    test_simple_line()
    test_fill_player_play()
    test_player_play()
