// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! WebP VP8 lossy decoder debug comparison.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: webp-compare <file.webp>");
        std::process::exit(1);
    }

    let data = std::fs::read(&args[1]).expect("failed to read file");
    eprintln!("[info] input: {} bytes", data.len());

    // Find VP8 chunk
    let mut pos = 12usize;
    let mut vp8_data: Option<&[u8]> = None;
    while pos + 8 <= data.len() {
        let tag = &data[pos..pos + 4];
        let size = u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]])
            as usize;
        let cs = pos + 8;
        if tag == b"VP8 " {
            vp8_data = Some(if cs + size <= data.len() {
                &data[cs..cs + size]
            } else {
                &data[cs..]
            });
            eprintln!("[info] VP8 chunk: {} bytes", size);
        }
        pos += 8 + ((size + 1) & !1);
    }
    let vp8 = vp8_data.expect("No VP8 chunk");

    // Parse frame header
    let tag = (vp8[0] as u32) | ((vp8[1] as u32) << 8) | ((vp8[2] as u32) << 16);
    let first_part_size = tag >> 5;
    let width = u16::from_le_bytes([vp8[6], vp8[7]]) & 0x3FFF;
    let height = u16::from_le_bytes([vp8[8], vp8[9]]) & 0x3FFF;
    eprintln!(
        "[header] {}x{} first_part_size={}",
        width, height, first_part_size
    );
    eprintln!("[header] part1 bytes: {}..{}", 10, 10 + first_part_size);

    // Token partitions start
    let part1_end = 10 + first_part_size as usize;
    eprintln!("[header] token_start={} data_len={}", part1_end, vp8.len());
    eprintln!(
        "[header] first token bytes: {:02X} {:02X} {:02X} {:02X}",
        vp8[part1_end],
        vp8[part1_end + 1],
        vp8[part1_end + 2],
        vp8[part1_end + 3]
    );

    // ── Decode with image crate ──
    let img = image::load_from_memory(&data).expect("image crate failed");
    let rgba = img.to_rgba8();
    let (rw, rh) = image::GenericImageView::dimensions(&rgba);
    let ref_pixels: Vec<u32> = rgba
        .pixels()
        .map(|p| {
            let [r, g, b, a] = p.0;
            ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
        })
        .collect();

    // ── Decode with libimage ──
    let info = libimage::webp::probe(&data).unwrap();
    let mut lib_pixels = vec![0u32; (info.width * info.height) as usize];
    let rc = libimage::webp::decode(&data, &mut lib_pixels, &mut []);
    eprintln!("[libimage] decode rc={}", rc);

    // ── Analyze differences ──
    let w = rw as usize;

    // Check first 4 macroblocks (first row)
    eprintln!("\n=== PIXEL ANALYSIS (first row, y=0) ===");
    for x in (0..64.min(w)).step_by(4) {
        let rp = ref_pixels[x];
        let lp = lib_pixels[x];
        let (rr, rg, rb) = ((rp >> 16) & 0xFF, (rp >> 8) & 0xFF, rp & 0xFF);
        let (lr, lg, lb) = ((lp >> 16) & 0xFF, (lp >> 8) & 0xFF, lp & 0xFF);
        eprintln!(
            "  x={:3}: ref=({:3},{:3},{:3}) lib=({:3},{:3},{:3})  diff=({:+},{:+},{:+})",
            x,
            rr,
            rg,
            rb,
            lr,
            lg,
            lb,
            lr as i32 - rr as i32,
            lg as i32 - rg as i32,
            lb as i32 - rb as i32
        );
    }

    // Check if libimage output is mostly uniform (suggesting prediction-only, no residuals)
    eprintln!("\n=== UNIFORMITY CHECK (first MB, 16x16) ===");
    let mut min_r = 255u32;
    let mut max_r = 0u32;
    let mut min_g = 255u32;
    let mut max_g = 0u32;
    let mut min_b = 255u32;
    let mut max_b = 0u32;
    for y in 0..16 {
        for x in 0..16 {
            let p = lib_pixels[y * w + x];
            let (r, g, b) = ((p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF);
            min_r = min_r.min(r);
            max_r = max_r.max(r);
            min_g = min_g.min(g);
            max_g = max_g.max(g);
            min_b = min_b.min(b);
            max_b = max_b.max(b);
        }
    }
    eprintln!(
        "  lib MB0: R=[{},{}] G=[{},{}] B=[{},{}]",
        min_r, max_r, min_g, max_g, min_b, max_b
    );

    // Same for reference
    let mut min_r = 255u32;
    let mut max_r = 0u32;
    let mut min_g = 255u32;
    let mut max_g = 0u32;
    let mut min_b = 255u32;
    let mut max_b = 0u32;
    for y in 0..16 {
        for x in 0..16 {
            let p = ref_pixels[y * w + x];
            let (r, g, b) = ((p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF);
            min_r = min_r.min(r);
            max_r = max_r.max(r);
            min_g = min_g.min(g);
            max_g = max_g.max(g);
            min_b = min_b.min(b);
            max_b = max_b.max(b);
        }
    }
    eprintln!(
        "  ref MB0: R=[{},{}] G=[{},{}] B=[{},{}]",
        min_r, max_r, min_g, max_g, min_b, max_b
    );

    // Check for macroblock boundary patterns
    eprintln!("\n=== MB BOUNDARY CHECK (y=0, stepping by 16) ===");
    for mb_x in 0..(w / 16).min(8) {
        let x = mb_x * 16;
        let rp = ref_pixels[x];
        let lp = lib_pixels[x];
        let (rr, rg, rb) = ((rp >> 16) & 0xFF, (rp >> 8) & 0xFF, rp & 0xFF);
        let (lr, lg, lb) = ((lp >> 16) & 0xFF, (lp >> 8) & 0xFF, lp & 0xFF);
        eprintln!(
            "  MB({},0): ref=({:3},{:3},{:3}) lib=({:3},{:3},{:3})",
            mb_x, rr, rg, rb, lr, lg, lb
        );
    }

    // Check column x=0 (left edge, MB boundary)
    eprintln!("\n=== LEFT EDGE (x=0, stepping by 16) ===");
    for mb_y in 0..((rh as usize) / 16).min(8) {
        let y = mb_y * 16;
        let rp = ref_pixels[y * w];
        let lp = lib_pixels[y * w];
        let (rr, rg, rb) = ((rp >> 16) & 0xFF, (rp >> 8) & 0xFF, rp & 0xFF);
        let (lr, lg, lb) = ((lp >> 16) & 0xFF, (lp >> 8) & 0xFF, lp & 0xFF);
        eprintln!(
            "  MB(0,{}): ref=({:3},{:3},{:3}) lib=({:3},{:3},{:3})",
            mb_y, rr, rg, rb, lr, lg, lb
        );
    }

    // Overall stats
    let total = ref_pixels.len();
    let mut mismatch = 0u64;
    let mut max_diff = 0u32;
    for i in 0..total {
        let d = channel_max_diff(ref_pixels[i], lib_pixels[i]);
        if d > 2 {
            mismatch += 1;
        }
        max_diff = max_diff.max(d);
    }
    eprintln!("\n=== SUMMARY ===");
    eprintln!(
        "  Mismatched (>2): {} / {} ({:.1}%)",
        mismatch,
        total,
        mismatch as f64 / total as f64 * 100.0
    );
    eprintln!("  Max diff: {}", max_diff);

    save_ppm(&ref_pixels, rw, rh, "/tmp/webp_ref.ppm");
    save_ppm(&lib_pixels, info.width, info.height, "/tmp/webp_lib.ppm");
    eprintln!("Saved: /tmp/webp_ref.ppm /tmp/webp_lib.ppm");
}

fn channel_max_diff(a: u32, b: u32) -> u32 {
    let dr = (((a >> 16) & 0xFF) as i32 - ((b >> 16) & 0xFF) as i32).unsigned_abs();
    let dg = (((a >> 8) & 0xFF) as i32 - ((b >> 8) & 0xFF) as i32).unsigned_abs();
    let db = ((a & 0xFF) as i32 - (b & 0xFF) as i32).unsigned_abs();
    dr.max(dg).max(db)
}

fn save_ppm(pixels: &[u32], w: u32, h: u32, path: &str) {
    use std::io::Write;
    let mut f = std::fs::File::create(path).unwrap();
    write!(f, "P6\n{} {}\n255\n", w, h).unwrap();
    for &px in pixels {
        f.write_all(&[
            ((px >> 16) & 0xFF) as u8,
            ((px >> 8) & 0xFF) as u8,
            (px & 0xFF) as u8,
        ])
        .unwrap();
    }
}
