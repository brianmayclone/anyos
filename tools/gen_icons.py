#!/usr/bin/env python3
"""Generate macOS/iOS-style dock icons as raw RGBA files for anyOS.

Format: [width:u32 LE][height:u32 LE][RGBA pixel data (width*height*4 bytes)]
Icons are 48x48 with smooth rounded-rect backgrounds, gradients, and symbols.
"""

import struct
import os
import sys
import math

SIZE = 48
RADIUS = 11  # iOS-style corner radius (~23% of size)


def lerp(a, b, t):
    """Linear interpolation between a and b."""
    return a + (b - a) * t


def clamp(v, lo=0, hi=255):
    return max(lo, min(hi, int(v)))


def rounded_rect_mask(x, y, w, h, r):
    """Returns alpha (0.0-1.0) for anti-aliased rounded rectangle."""
    # Distance from edges
    if x < r and y < r:
        dx = r - x - 0.5
        dy = r - y - 0.5
        dist = math.sqrt(dx * dx + dy * dy)
        return max(0.0, min(1.0, r - dist + 0.5))
    elif x >= w - r and y < r:
        dx = x - (w - r) + 0.5
        dy = r - y - 0.5
        dist = math.sqrt(dx * dx + dy * dy)
        return max(0.0, min(1.0, r - dist + 0.5))
    elif x < r and y >= h - r:
        dx = r - x - 0.5
        dy = y - (h - r) + 0.5
        dist = math.sqrt(dx * dx + dy * dy)
        return max(0.0, min(1.0, r - dist + 0.5))
    elif x >= w - r and y >= h - r:
        dx = x - (w - r) + 0.5
        dy = y - (h - r) + 0.5
        dist = math.sqrt(dx * dx + dy * dy)
        return max(0.0, min(1.0, r - dist + 0.5))
    elif x < 0 or x >= w or y < 0 or y >= h:
        return 0.0
    else:
        return 1.0


def make_icon(gradient_top, gradient_bottom, symbol_func):
    """Create a 48x48 RGBA icon with gradient background and symbol."""
    pixels = bytearray(SIZE * SIZE * 4)

    for y in range(SIZE):
        t = y / (SIZE - 1)  # 0.0 at top, 1.0 at bottom
        # Gradient background
        r = clamp(lerp(gradient_top[0], gradient_bottom[0], t))
        g = clamp(lerp(gradient_top[1], gradient_bottom[1], t))
        b = clamp(lerp(gradient_top[2], gradient_bottom[2], t))

        for x in range(SIZE):
            alpha = rounded_rect_mask(x, y, SIZE, SIZE, RADIUS)
            if alpha <= 0:
                continue
            off = (y * SIZE + x) * 4

            # Subtle highlight at top (iOS glass effect)
            highlight = max(0.0, 1.0 - y / 8.0) * 0.15
            pr = clamp(r + highlight * 255)
            pg = clamp(g + highlight * 255)
            pb = clamp(b + highlight * 255)

            pixels[off] = pr
            pixels[off + 1] = pg
            pixels[off + 2] = pb
            pixels[off + 3] = clamp(alpha * 255)

    # Draw symbol
    symbol_func(pixels)
    return pixels


def set_pixel(pixels, x, y, r, g, b, a=255):
    """Set a pixel with alpha blending over existing content."""
    if x < 0 or x >= SIZE or y < 0 or y >= SIZE:
        return
    off = (y * SIZE + x) * 4
    if a >= 255:
        pixels[off] = r
        pixels[off + 1] = g
        pixels[off + 2] = b
        pixels[off + 3] = 255
    elif a > 0:
        da = pixels[off + 3]
        if da == 0:
            pixels[off] = r
            pixels[off + 1] = g
            pixels[off + 2] = b
            pixels[off + 3] = a
        else:
            fa = a / 255.0
            inv = 1.0 - fa
            pixels[off] = clamp(r * fa + pixels[off] * inv)
            pixels[off + 1] = clamp(g * fa + pixels[off + 1] * inv)
            pixels[off + 2] = clamp(b * fa + pixels[off + 2] * inv)
            pixels[off + 3] = clamp(a + da * inv)


def draw_line_aa(pixels, x0, y0, x1, y1, r, g, b, thickness=2.0):
    """Draw an anti-aliased line."""
    dx = x1 - x0
    dy = y1 - y0
    length = math.sqrt(dx * dx + dy * dy)
    if length < 0.01:
        return
    steps = int(length * 2) + 1
    half_t = thickness / 2.0

    for s in range(steps + 1):
        t = s / steps
        cx = x0 + dx * t
        cy = y0 + dy * t

        # Fill pixels around the center point
        for py in range(int(cy - half_t - 1), int(cy + half_t + 2)):
            for px in range(int(cx - half_t - 1), int(cx + half_t + 2)):
                dist = math.sqrt((px + 0.5 - cx) ** 2 + (py + 0.5 - cy) ** 2)
                alpha = max(0.0, min(1.0, half_t - dist + 0.5))
                if alpha > 0:
                    set_pixel(pixels, px, py, r, g, b, clamp(alpha * 255))


def draw_filled_circle(pixels, cx, cy, radius, r, g, b, a=255):
    """Draw a filled anti-aliased circle."""
    for py in range(int(cy - radius - 1), int(cy + radius + 2)):
        for px in range(int(cx - radius - 1), int(cx + radius + 2)):
            dist = math.sqrt((px + 0.5 - cx) ** 2 + (py + 0.5 - cy) ** 2)
            alpha = max(0.0, min(1.0, radius - dist + 0.5))
            if alpha > 0:
                set_pixel(pixels, px, py, r, g, b, clamp(alpha * a / 255))


def draw_terminal_symbol(pixels):
    """Draw '>_' prompt symbol — clean terminal look."""
    fg = (255, 255, 255)

    # Draw '>' chevron with anti-aliased lines
    draw_line_aa(pixels, 13, 14, 24, 23, *fg, thickness=2.5)
    draw_line_aa(pixels, 24, 23, 13, 32, *fg, thickness=2.5)

    # Draw '_' cursor blink underscore
    draw_line_aa(pixels, 26, 33, 36, 33, *fg, thickness=2.5)


def draw_monitor_symbol(pixels):
    """Draw activity monitor bars — colorful bar chart."""
    colors = [
        (46, 204, 113),   # green
        (52, 152, 219),   # blue
        (241, 196, 15),   # yellow
        (231, 76, 60),    # red
    ]
    bar_w = 6
    gap = 3
    start_x = 9
    base_y = 37
    heights = [20, 14, 26, 10]

    for i, (h, (cr, cg, cb)) in enumerate(zip(heights, colors)):
        bx = start_x + i * (bar_w + gap)
        # Draw each bar with rounded top
        for x in range(bx, bx + bar_w):
            for y in range(base_y - h, base_y):
                # Rounded top corners (radius=2)
                if y < base_y - h + 2:
                    if x < bx + 2 or x >= bx + bar_w - 2:
                        dx = min(x - bx, bx + bar_w - 1 - x)
                        dy = base_y - h + 2 - y
                        if dx < 2 and dy > 0:
                            dist = math.sqrt((2 - dx - 0.5) ** 2 + (dy - 0.5) ** 2)
                            if dist > 2:
                                continue
                set_pixel(pixels, x, y, cr, cg, cb, 255)


def draw_finder_symbol(pixels):
    """Draw a macOS Finder-style folder icon."""
    fg = (255, 255, 255)

    # Folder body
    body_x, body_y = 10, 18
    body_w, body_h = 28, 22

    # Folder tab on top-left
    tab_x, tab_y = 10, 14
    tab_w, tab_h = 14, 5

    # Draw tab
    for y in range(tab_y, tab_y + tab_h):
        for x in range(tab_x, tab_x + tab_w):
            r = min(2, min(x - tab_x, tab_x + tab_w - 1 - x))
            if y < tab_y + 2 and r < 2:
                dist = math.sqrt((2 - r) ** 2 + (tab_y + 2 - y) ** 2)
                if dist > 2.5:
                    continue
            set_pixel(pixels, x, y, *fg, 230)

    # Draw folder body with rounded bottom corners
    for y in range(body_y, body_y + body_h):
        for x in range(body_x, body_x + body_w):
            # Rounded bottom corners
            if y >= body_y + body_h - 3:
                r = min(x - body_x, body_x + body_w - 1 - x)
                dy = y - (body_y + body_h - 3)
                if r < 3 and dy > 0:
                    dist = math.sqrt((3 - r - 0.5) ** 2 + (dy - 0.5) ** 2)
                    if dist > 3:
                        continue
            set_pixel(pixels, x, y, *fg, 230)

    # Draw a small magnifying glass in the center (Finder = search)
    glass_cx, glass_cy = 22, 28
    glass_r = 5
    draw_filled_circle(pixels, glass_cx, glass_cy, glass_r, 50, 130, 220, 200)
    draw_filled_circle(pixels, glass_cx, glass_cy, glass_r - 1.5, 80, 160, 240, 200)
    # Handle
    draw_line_aa(pixels, glass_cx + 3.5, glass_cy + 3.5, glass_cx + 7, glass_cy + 7, 50, 130, 220, thickness=2.0)


def draw_settings_symbol(pixels):
    """Draw a macOS-style gear icon."""
    cx, cy = SIZE / 2, SIZE / 2
    outer_r = 17
    inner_r = 13
    hole_r = 7
    teeth = 8

    for y in range(SIZE):
        for x in range(SIZE):
            dx = x + 0.5 - cx
            dy = y + 0.5 - cy
            dist = math.sqrt(dx * dx + dy * dy)
            angle = math.atan2(dy, dx)

            # Teeth pattern
            tooth_angle = angle * teeth / (2 * math.pi)
            tooth_frac = tooth_angle - math.floor(tooth_angle)
            # Smooth teeth transitions
            if tooth_frac < 0.3:
                tooth_blend = min(1.0, tooth_frac / 0.05)
            elif tooth_frac < 0.35:
                tooth_blend = 1.0
            else:
                tooth_blend = max(0.0, 1.0 - (tooth_frac - 0.35) / 0.05)

            effective_r = inner_r + (outer_r - inner_r) * tooth_blend

            # Anti-aliased edges
            outer_alpha = max(0.0, min(1.0, effective_r - dist + 0.5))
            inner_alpha = max(0.0, min(1.0, dist - hole_r + 0.5))
            alpha = outer_alpha * inner_alpha

            if alpha > 0:
                # Subtle gradient on the gear
                t = (y - (cy - outer_r)) / (2 * outer_r)
                r = clamp(lerp(230, 200, t))
                g = clamp(lerp(230, 200, t))
                b = clamp(lerp(235, 205, t))
                set_pixel(pixels, x, y, r, g, b, clamp(alpha * 255))


def save_icon(pixels, path):
    """Save icon as raw RGBA with width/height header."""
    header = struct.pack('<II', SIZE, SIZE)
    with open(path, 'wb') as f:
        f.write(header)
        f.write(pixels)


def main():
    if len(sys.argv) < 2:
        print(f"Usage: {sys.argv[0]} <output_dir>")
        sys.exit(1)

    output_dir = sys.argv[1]
    os.makedirs(output_dir, exist_ok=True)

    # macOS-style gradient backgrounds (top color, bottom color)
    icons = {
        # Terminal: dark charcoal with slight blue tint
        'terminal': ((55, 55, 65), (25, 25, 35), draw_terminal_symbol),
        # Activity Monitor: dark navy blue
        'taskmanager': ((45, 55, 80), (20, 28, 50), draw_monitor_symbol),
        # Settings: medium-dark grey (like macOS System Preferences)
        'settings': ((130, 130, 135), (85, 85, 90), draw_settings_symbol),
        # Finder: blue (like macOS Finder)
        'finder': ((40, 120, 220), (20, 70, 160), draw_finder_symbol),
    }

    for name, (top, bot, sym_func) in icons.items():
        pixels = make_icon(top, bot, sym_func)
        path = os.path.join(output_dir, f'{name}.icon')
        save_icon(pixels, path)
        print(f'  Icon: {name}.icon ({SIZE}x{SIZE}, {len(pixels) + 8} bytes)')


if __name__ == '__main__':
    main()
