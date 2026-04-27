// JPEG corpus test harness: decode every .jpg in third_party/codec-corpus
// using libimage and the image crate, compare results.
//
// Usage: cargo run --release --bin jpeg-corpus -- [corpus_dir]
// Default corpus_dir: ../../third_party/codec-corpus

use std::fs;
use std::path::{Path, PathBuf};

fn collect_jpegs(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(root) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_jpegs(&p, out);
        } else if let Some(ext) = p.extension() {
            let ext = ext.to_string_lossy().to_lowercase();
            if ext == "jpg" || ext == "jpeg" {
                out.push(p);
            }
        }
    }
}

#[derive(Default, Debug)]
struct Stats {
    total: usize,
    libimage_ok: usize,
    libimage_invalid_or_unsupported: usize,
    libimage_other_err: usize,
    image_crate_ok: usize,
    image_crate_err: usize,
    both_ok_pixel_match: usize,
    both_ok_pixel_diff: usize,
    libimage_only: usize,
    image_only: usize,
    neither: usize,
}

fn decode_libimage(data: &[u8]) -> Result<(u32, u32, Vec<u32>), i32> {
    use libimage::jpeg;
    let info = jpeg::probe(data).ok_or(-99)?;
    let w = info.width as usize;
    let h = info.height as usize;
    if w == 0 || h == 0 {
        return Err(-100);
    }
    let mut out = vec![0u32; w * h];
    let mut scratch = vec![0u8; info.scratch_needed as usize];
    let rc = jpeg::decode(data, &mut out, &mut scratch);
    if rc == 0 {
        Ok((info.width, info.height, out))
    } else {
        Err(rc)
    }
}

fn decode_image(data: &[u8]) -> Result<(u32, u32, Vec<u32>), String> {
    let img = image::load_from_memory_with_format(data, image::ImageFormat::Jpeg)
        .map_err(|e| e.to_string())?;
    let rgba = img.to_rgba8();
    let w = rgba.width();
    let h = rgba.height();
    let mut out = Vec::with_capacity((w * h) as usize);
    for px in rgba.pixels() {
        let [r, g, b, a] = px.0;
        out.push((a as u32) << 24 | (r as u32) << 16 | (g as u32) << 8 | b as u32);
    }
    Ok((w, h, out))
}

fn pixel_compare(a: &[u32], b: &[u32]) -> (u64, u64) {
    // returns (max_channel_diff, total_channels_diff_gt_3)
    let mut max_d = 0u64;
    let mut over = 0u64;
    for (pa, pb) in a.iter().zip(b.iter()) {
        for shift in [0, 8, 16] {
            let ca = ((*pa >> shift) & 0xFF) as i32;
            let cb = ((*pb >> shift) & 0xFF) as i32;
            let d = (ca - cb).unsigned_abs() as u64;
            if d > max_d {
                max_d = d;
            }
            if d > 3 {
                over += 1;
            }
        }
    }
    (max_d, over)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let verbose = args.iter().any(|a| a == "-v");
    let only_valid = !args.iter().any(|a| a == "--include-invalid");
    let corpus = args
        .iter()
        .skip(1)
        .find(|a| !a.starts_with('-'))
        .cloned()
        .unwrap_or_else(|| "../../third_party/codec-corpus".to_string());

    let mut paths = Vec::new();
    collect_jpegs(Path::new(&corpus), &mut paths);
    if only_valid {
        paths.retain(|p| !p.to_string_lossy().contains("/invalid/"));
    }
    paths.sort();
    eprintln!("Testing {} JPEGs from {}", paths.len(), corpus);

    let mut s = Stats::default();
    let mut diffs: Vec<(String, u64, u64)> = Vec::new();
    let mut libimage_failed: Vec<(String, i32)> = Vec::new();

    for p in &paths {
        s.total += 1;
        let Ok(data) = fs::read(p) else { continue };
        let lib_r = decode_libimage(&data);
        let img_r = decode_image(&data);

        match (&lib_r, &img_r) {
            (Ok(_), Ok(_)) => {}
            (Ok(_), Err(_)) => s.libimage_only += 1,
            (Err(_), Ok(_)) => s.image_only += 1,
            (Err(_), Err(_)) => s.neither += 1,
        }
        if lib_r.is_ok() {
            s.libimage_ok += 1;
        } else if let Err(rc) = &lib_r {
            if *rc == -1 || *rc == -2 || *rc == -99 {
                s.libimage_invalid_or_unsupported += 1;
            } else {
                s.libimage_other_err += 1;
            }
            libimage_failed.push((p.display().to_string(), *rc));
        }
        if img_r.is_ok() {
            s.image_crate_ok += 1;
        } else {
            s.image_crate_err += 1;
        }
        if let (Ok((w1, h1, p1)), Ok((w2, h2, p2))) = (&lib_r, &img_r) {
            if w1 == w2 && h1 == h2 && p1.len() == p2.len() {
                let (md, over) = pixel_compare(p1, p2);
                if md <= 4 {
                    s.both_ok_pixel_match += 1;
                } else {
                    s.both_ok_pixel_diff += 1;
                    diffs.push((p.display().to_string(), md, over));
                }
            } else {
                s.both_ok_pixel_diff += 1;
                diffs.push((p.display().to_string(), 999, 999));
            }
        }
    }

    println!("\n=== JPEG corpus summary ===");
    println!("Total:                    {}", s.total);
    println!("libimage decoded ok:      {}", s.libimage_ok);
    println!(
        "  invalid/unsupported:    {}",
        s.libimage_invalid_or_unsupported
    );
    println!("  other error:            {}", s.libimage_other_err);
    println!("image crate decoded ok:   {}", s.image_crate_ok);
    println!("Both ok, pixels match:    {}", s.both_ok_pixel_match);
    println!("Both ok, pixels DIFFER:   {}", s.both_ok_pixel_diff);
    println!("libimage-only success:    {}", s.libimage_only);
    println!("image-only success:       {}", s.image_only);
    println!("Neither decoded:          {}", s.neither);

    if verbose {
        println!("\nlibimage failures:");
        for (p, rc) in &libimage_failed {
            println!("  rc={} {}", rc, p);
        }
        println!("\nPixel diffs:");
        for (p, md, over) in &diffs {
            println!("  max_d={} over3={} {}", md, over, p);
        }
    }
}
