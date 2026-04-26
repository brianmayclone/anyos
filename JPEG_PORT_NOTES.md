# JPEG Decoder Port Notes

## Phase 1: Survey

### Existing decoder (libs/libimage/src/jpeg.rs, 1586 lines)

**Public API (must NOT change):**
- `pub fn probe(data: &[u8]) -> Option<ImageInfo>`
- `pub fn decode(data: &[u8], out: &mut [u32], scratch: &mut [u8]) -> i32`
- `pub fn idct(block: &mut [i32; 64])`

**Currently supported:**
- SOF0 (Baseline DCT, 8-bit)
- SOF2 (Progressive DCT, 8-bit) — full progressive: DC initial+refinement, AC initial+refinement, EOB runs
- 1, 2, or 3 components (Y or YCbCr); rejects >3 components (`MAX_COMP = 3`)
- 8-bit precision only — explicitly rejects 12-bit
- Chroma subsampling: any combo where h_samples ≤ 2 and v_samples ≤ 2 (covers 4:4:4, 4:2:2, 4:2:0, 4:1:1, 4:4:0, 4:1:0)
- Huffman coded (DHT marker) only
- Restart intervals (DRI marker) — only inside scans, RST handling via `BitReader.marker_seen`
- DQT 8-bit and 16-bit quant tables
- Markers parsed: SOI, EOI, SOF0, SOF2, DHT, DQT, DRI, SOS. Everything else skipped by length.
- Box upsampling (nearest neighbor: `cx = x * h_samples / max_h`)
- Output: ARGB8888 only. Grayscale → R=G=B path. YCbCr → BT.601 conversion (fixed-point).

**Reference: libjpeg-9f** (third_party/libjpeg-reference/jpeg-9f/)
- Baseline + Extended Sequential + Progressive + Lossless: jdhuff.c, jdcoefct.c
- Arithmetic coding: jdarith.c (~800 lines, alternative entropy decoder for SOF9/10/11)
- Color: jdcolor.c (YCbCr/RGB/CMYK/YCCK→RGB), jdmerge.c (upsample+convert in one pass for 4:2:2/4:2:0)
- Sample upsampling: jdsample.c — fancy (smooth) upsampling, not box
- IDCT: jidctint.c (slow), jidctflt.c, jidctfst.c (fast), jidctred.c (reduced 4x4/2x2/1x1 for scale-down)
- DNL marker (Define Number of Lines): jdmarker.c — height initially 0, set after first scan
- Adobe APP14 marker: jdmarker.c lines around `examine_app14` — color-transform byte distinguishes RGB vs YCbCr (3-comp) and CMYK vs YCCK (4-comp)
- JFIF APP0 marker: identification + thumbnail (skipped is fine for decoder)
- Hierarchical JPEG (SOF5/6/7/13/14/15): a separate frame mode, very rarely used; libjpeg supports decoding but it's a deeply different scan model

### Gap list (libjpeg has, we don't)

| # | Feature | Effort | Real-world impact |
|---|---------|--------|-------------------|
| 1 | 4-component (CMYK/YCCK) + Adobe APP14 marker | Medium | High — Photoshop JPEGs in corpus (7 files = 1.8%) |
| 2 | Grayscale completeness (already mostly there; verify SOF1/extended) | Low | High — 14 files = 3.7% |
| 3 | SOF1 (Extended sequential, 8-bit Huffman) | Tiny | Low — 1 file. SOF1 8-bit decodes identically to SOF0 |
| 4 | 12-bit precision (SOF1 12-bit, SOF0 12-bit) | High | Low — 3 files = 0.8%, mostly testimgs. Requires wider IDCT, dequant, sample paths |
| 5 | Arithmetic coding (SOF9/10/11) | High | Tiny — 2 files in corpus, real-world: extremely rare |
| 6 | Lossless (SOF3) | High | Tiny — different code path entirely |
| 7 | Hierarchical (SOF5/6/7/13-15) | Very High | Tiny — essentially unused |
| 8 | Fancy upsampling (smooth chroma 4:2:0/4:2:2) | Medium | Medium — visible quality improvement, libjpeg's default |
| 9 | DNL marker | Tiny | Tiny — height=0 then DNL — uncommon |
| 10 | Restart marker recovery (RST counter check) | Tiny | Robustness only |
| 11 | Scale-down decoding (1/2, 1/4, 1/8 via reduced IDCT) | Medium | Medium — browser thumbnails, but Surf rescales after. Not strictly needed |

### Corpus distribution (third_party/codec-corpus, 250 valid JPEGs surveyed)

| Count | SOF | Precision | Components | Note |
|-------|-----|-----------|------------|------|
| 163 | C0 (Baseline) | 8 | 3 | Standard YCbCr |
| 35 | C2 (Progressive) | 8 | 3 | Standard YCbCr |
| 18 | C0 | 8 | 3 | + APP14 (Adobe RGB or YCbCr) |
| 9 | C0 | 8 | 1 | Grayscale baseline |
| 5 | C2 | 8 | 3 | + APP14 |
| 5 | C2 | 8 | 1 | Grayscale progressive |
| 4 | C0 | 8 | 4 | + APP14 (CMYK/YCCK) |
| 3 | C2 | 8 | 4 | + APP14 (CMYK/YCCK) |
| 2 | C9 (Arith ext seq) | 8 | 3 | Arithmetic coding |
| 2 | C1 (Ext sequential) | 12 | 3 | 12-bit |
| 1 | C1 | 8 | 3 | Ext sequential 8-bit (== baseline) |
| 1 | C0 | 8 | 4 | CMYK without APP14 |
| 1 | C0 | 8 | 2 | 2-component (rare) |
| 1 | C0 | 8 | 1 | + APP14 |
| 1 | C0 | 12 | 3 | 12-bit baseline |

Total currently-decodable estimate (before this work): ~213/250 = 85% (3-comp baseline+progressive grayscale, ignoring whether the APP14 transform matters).

### Plan

Implementation priority order:

1. **SOF1 alias (Extended Sequential 8-bit Huffman)** — accept it like SOF0. Trivial.
2. **4-component support + Adobe APP14 marker** — bump MAX_COMP to 4, parse APP14, extend `frame.comp` to 4, add CMYK→RGB and YCCK→RGB color conversion. (+8 corpus files explicitly, helps any 4-comp)
3. **Grayscale variants completeness verification** — already supported, just confirm via test.
4. **DNL marker** — small fix.
5. **12-bit precision** — separate code path; defer / document if too large.
6. **Arithmetic coding** — port jdarith.c; well-defined but ~800 lines. Defer until later if time pressed.
7. **Fancy upsampling** — smooth 4:2:0/4:2:2 chroma; quality improvement.
8. **Lossless / Hierarchical** — out of scope unless trivial.

### Strategy

- Keep file as a single `jpeg.rs` since size is manageable; only split into module if it grows past ~3000 lines.
- Do NOT change public API.
- For each priority item: implement, build for both `cargo +stable build --release` (host check via `tools/surf-host`) and `ninja libimage` (anyOS).
- Add a Rust `#[cfg(test)]` block at end of jpeg.rs with corpus-pointing tests (using env var or relative path), gated on `feature = "host"` so they only run during host builds.
- Commit per feature with clear messages.

