#![no_std]
#![no_main]

anyos_std::entry!(main);

/// Default ASCII character ramp (dark to light), matching original jp2a.
const DEFAULT_CHARS: &[u8] = b"   ...',;:clodxkO0KXNWM";

fn read_file(path: &str) -> Option<anyos_std::Vec<u8>> {
    let fd = anyos_std::fs::open(path, 0);
    if fd == u32::MAX {
        return None;
    }
    let mut data = anyos_std::Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = anyos_std::fs::read(fd, &mut buf);
        if n == 0 || n == u32::MAX {
            break;
        }
        let n = n as usize;
        for i in 0..n {
            data.push(buf[i]);
        }
    }
    anyos_std::fs::close(fd);
    Some(data)
}

/// Parse "WxH" string into (width, height). Returns None on failure.
fn parse_size(s: &str) -> Option<(u32, u32)> {
    let bytes = s.as_bytes();
    let mut split = 0;
    for i in 0..bytes.len() {
        if bytes[i] == b'x' || bytes[i] == b'X' {
            split = i;
            break;
        }
    }
    if split == 0 || split >= bytes.len() - 1 {
        return None;
    }
    let w = parse_u32(&bytes[..split])?;
    let h = parse_u32(&bytes[split + 1..])?;
    Some((w, h))
}

fn parse_u32(bytes: &[u8]) -> Option<u32> {
    if bytes.is_empty() {
        return None;
    }
    let mut n: u32 = 0;
    for &b in bytes {
        if b < b'0' || b > b'9' {
            return None;
        }
        n = n.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(n)
}

/// Select ANSI color code (31-37) based on RGB channel dominance.
/// Matches the original jp2a threshold-based color selection.
fn ansi_color(r: u32, g: u32, b: u32) -> u8 {
    // Thresholds for channel dominance
    let threshold: u32 = 40;

    let r_gt_g = r > g + threshold;
    let r_gt_b = r > b + threshold;
    let g_gt_r = g > r + threshold;
    let g_gt_b = g > b + threshold;
    let b_gt_r = b > r + threshold;
    let b_gt_g = b > g + threshold;

    if r_gt_g && r_gt_b {
        31 // Red
    } else if g_gt_r && g_gt_b {
        32 // Green
    } else if b_gt_r && b_gt_g {
        34 // Blue
    } else if r_gt_b && g_gt_b {
        33 // Yellow (red + green)
    } else if r_gt_g && b_gt_g {
        35 // Magenta (red + blue)
    } else if g_gt_r && b_gt_r {
        36 // Cyan (green + blue)
    } else {
        37 // White (neutral)
    }
}

/// Write a small decimal number as ASCII digits.
fn write_u8_decimal(val: u8) {
    let mut buf = [0u8; 3];
    let mut n = val as u32;
    let mut len = 0;
    if n == 0 {
        anyos_std::fs::write(1, b"0");
        return;
    }
    while n > 0 {
        buf[len] = b'0' + (n % 10) as u8;
        n /= 10;
        len += 1;
    }
    // Reverse
    let mut out = [0u8; 3];
    for i in 0..len {
        out[i] = buf[len - 1 - i];
    }
    anyos_std::fs::write(1, &out[..len]);
}

fn process_image(
    path: &str,
    out_w: u32,
    out_h_opt: Option<u32>,
    chars: &[u8],
    invert: bool,
    flipx: bool,
    flipy: bool,
    border: bool,
    colors: bool,
    colorfill: bool,
    grayscale: bool,
) {
    // Read image file
    let data = match read_file(path) {
        Some(d) => d,
        None => {
            anyos_std::println!("jp2a: cannot open '{}'", path);
            return;
        }
    };

    // Probe image
    let info = match libimage_client::probe(&data) {
        Some(i) => i,
        None => {
            anyos_std::println!("jp2a: unsupported image format '{}'", path);
            return;
        }
    };

    if info.width == 0 || info.height == 0 {
        anyos_std::println!("jp2a: invalid image dimensions in '{}'", path);
        return;
    }

    // Decode image to ARGB8888
    let pixel_count = (info.width as usize) * (info.height as usize);
    let mut pixels = anyos_std::vec![0u32; pixel_count];
    let mut scratch = anyos_std::vec![0u8; info.scratch_needed as usize];

    if libimage_client::decode(&data, &mut pixels, &mut scratch).is_err() {
        anyos_std::println!("jp2a: failed to decode '{}'", path);
        return;
    }

    // Free file data and scratch (no longer needed)
    drop(data);
    drop(scratch);

    // Calculate output height if not explicitly set
    // Account for ~2:1 terminal character aspect ratio (chars are taller than wide)
    let out_h = match out_h_opt {
        Some(h) => h,
        None => {
            let h = (info.height as u64 * out_w as u64) / (info.width as u64 * 2);
            if h < 1 { 1 } else { h as u32 }
        }
    };

    // Scale image to output dimensions
    let scaled_count = (out_w as usize) * (out_h as usize);
    let mut scaled = anyos_std::vec![0u32; scaled_count];

    if !libimage_client::scale_image(
        &pixels, info.width, info.height,
        &mut scaled, out_w, out_h,
        libimage_client::MODE_SCALE,
    ) {
        anyos_std::println!("jp2a: failed to scale image");
        return;
    }

    // Free original pixels
    drop(pixels);

    let chars_len = chars.len();
    if chars_len == 0 {
        return;
    }
    let chars_max = (chars_len - 1) as u32;

    // Border top
    if border {
        anyos_std::fs::write(1, b"+");
        for _ in 0..out_w {
            anyos_std::fs::write(1, b"-");
        }
        anyos_std::fs::write(1, b"+\n");
    }

    // Render each row
    for row in 0..out_h {
        let src_y = if flipy { out_h - 1 - row } else { row };

        if border {
            anyos_std::fs::write(1, b"|");
        }

        // Build a line buffer to minimize write syscalls
        let w = out_w as usize;
        let mut line = anyos_std::vec![0u8; w];

        for col in 0..out_w {
            let src_x = if flipx { out_w - 1 - col } else { col };
            let pixel = scaled[(src_y * out_w + src_x) as usize];

            let r = (pixel >> 16) & 0xFF;
            let g = (pixel >> 8) & 0xFF;
            let b = pixel & 0xFF;

            // ITU-R BT.601 luminance (integer approximation)
            let mut luma = (r * 299 + g * 587 + b * 114) / 1000;
            if invert {
                luma = 255 - luma;
            }

            let idx = (luma * chars_max / 255) as usize;
            let ch = chars[idx];

            if colors {
                // ANSI color escape sequence
                let color_code = if grayscale {
                    37 // white
                } else {
                    ansi_color(r, g, b)
                };

                // Write escape: \x1b[
                anyos_std::fs::write(1, b"\x1b[");
                write_u8_decimal(color_code);
                if colorfill {
                    anyos_std::fs::write(1, b";");
                    write_u8_decimal(color_code + 10); // background = foreground + 10
                }
                anyos_std::fs::write(1, b"m");
                anyos_std::fs::write(1, &[ch]);
            } else {
                line[col as usize] = ch;
            }
        }

        if colors {
            // Reset color at end of line
            anyos_std::fs::write(1, b"\x1b[0m");
        } else {
            anyos_std::fs::write(1, &line);
        }

        if border {
            anyos_std::fs::write(1, b"|");
        }

        anyos_std::fs::write(1, b"\n");
    }

    // Border bottom
    if border {
        anyos_std::fs::write(1, b"+");
        for _ in 0..out_w {
            anyos_std::fs::write(1, b"-");
        }
        anyos_std::fs::write(1, b"+\n");
    }
}

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"whsc");

    // Parse dimensions
    let mut out_w: u32 = 78;
    let mut out_h_opt: Option<u32> = None;

    // -s WxH overrides -w and -h
    if let Some(size_str) = args.opt(b's') {
        if let Some((w, h)) = parse_size(size_str) {
            out_w = if w > 0 { w } else { 1 };
            out_h_opt = Some(if h > 0 { h } else { 1 });
        }
    } else {
        out_w = args.opt_u32(b'w', 78);
        if out_w == 0 {
            out_w = 1;
        }
        let h = args.opt_u32(b'h', 0);
        if h > 0 {
            out_h_opt = Some(h);
        }
    }

    // Character ramp
    let chars: &[u8] = match args.opt(b'c') {
        Some(s) if !s.is_empty() => s.as_bytes(),
        _ => DEFAULT_CHARS,
    };

    // Boolean flags
    let invert = args.has(b'i');
    let flipx = args.has(b'x');
    let flipy = args.has(b'y');
    let border = args.has(b'b');
    let colors = args.has(b'r');
    let colorfill = args.has(b'f');
    let grayscale = args.has(b'g');
    let clear = args.has(b'l');

    if args.pos_count == 0 {
        anyos_std::println!("jp2a - convert images to ASCII art");
        anyos_std::println!("");
        anyos_std::println!("Usage: jp2a [options] FILE...");
        anyos_std::println!("");
        anyos_std::println!("Options:");
        anyos_std::println!("  -w WIDTH   output width in columns (default 78)");
        anyos_std::println!("  -h HEIGHT  output height in rows (default: auto)");
        anyos_std::println!("  -s WxH     set both width and height");
        anyos_std::println!("  -c CHARS   character ramp, dark to light");
        anyos_std::println!("  -i         invert brightness");
        anyos_std::println!("  -x         flip horizontally");
        anyos_std::println!("  -y         flip vertically");
        anyos_std::println!("  -b         draw border");
        anyos_std::println!("  -r         ANSI color output");
        anyos_std::println!("  -f         fill background color (with -r)");
        anyos_std::println!("  -g         force grayscale (with -r)");
        anyos_std::println!("  -l         clear screen before output");
        anyos_std::println!("");
        anyos_std::println!("Supported formats: JPEG, PNG, BMP, GIF");
        return;
    }

    // Clear screen if requested
    if clear {
        anyos_std::fs::write(1, b"\x1b[2J\x1b[H");
    }

    for i in 0..args.pos_count {
        let path = args.positional[i];

        // Print filename header when processing multiple files
        if args.pos_count > 1 {
            if i > 0 {
                anyos_std::println!("");
            }
            anyos_std::println!("{}:", path);
        }

        process_image(
            path, out_w, out_h_opt, chars,
            invert, flipx, flipy, border,
            colors, colorfill, grayscale,
        );
    }
}
