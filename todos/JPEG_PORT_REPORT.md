# JPEG Decoder Port — Final Report

## Goal

Bring `libs/libimage/src/jpeg.rs` closer to feature parity with IJG
libjpeg (v9f reference, used as authoritative source — v10 URL the user
supplied was unreachable, falling back to v9f from www.ijg.org). The
end use case is Surf rendering arbitrary JPEGs encountered on the web.

## Reference material installed

- `third_party/libjpeg-reference/jpeg-9f/` — IJG libjpeg v9f sources.
  Read-only reference, never compiled or linked.
- `third_party/codec-corpus/` — full clone of `imazen/codec-corpus`
  (~5800 files, 257 valid `.jpg`/`.jpeg` files used as the test set).

Both are gitignored and not committed.

## Public API

Unchanged. Callers still see:

- `pub fn jpeg::probe(data: &[u8]) -> Option<ImageInfo>`
- `pub fn jpeg::decode(data: &[u8], out: &mut [u32], scratch: &mut [u8]) -> i32`
- `pub fn jpeg::idct(block: &mut [i32; 64])`

A new `jpeg::idct_p(block, precision)` was added but is purely additive.

## Features added (commit hashes)

| Commit | Feature |
|--------|---------|
| `0358f002` | `test(jpeg): add corpus comparison harness in surf-host` — `tools/surf-host/src/jpeg_corpus.rs` decodes every corpus JPEG with both libimage and the `image` crate and prints stats. Also dropped noisy host-only `eprintln!` traces left over from earlier debugging. |
| `75679fd0` | `feat(jpeg): support SOF1 + 4-component CMYK/YCCK + Adobe APP14` — accept SOF1 as a baseline alias, bump `MAX_COMP` 3→4, parse Adobe APP14 marker, refactor color conversion into `color_convert()` with grayscale (1+2 comp), YCbCr→RGB (default 3 comp), RGB-passthrough (3 comp + APP14 transform=0), YCCK→RGB (4 comp + APP14 transform=2), and CMYK→RGB (4 comp default) paths. |
| `00139f36` | `feat(jpeg): support 12-bit precision` — accept SOF P=12, parameterise IDCT bias and post-IDCT shift on `frame.precision`, widen IDCT bias from `i32` to `i64` via a new `idct_1d_i64` helper. |
| `a1b3fbc1` | `feat(jpeg): standard Annex-K Huffman tables for MJPEG streams` — add `STD_DC_LUMA_BITS`/`VALS`, `STD_DC_CHROMA_BITS`/`VALS`, `STD_AC_LUMA_BITS`/`VALS`, `STD_AC_CHROMA_BITS`/`VALS` from T.81 Annex K. Install them into any DC0/DC1/AC0/AC1 slot the file omitted. Defined-by-DHT tables are NOT overwritten. |
| `7f4d2f2a` | `feat(jpeg): allow h_samples / v_samples up to 4` — JPEG spec permits 1..=4; we now accept 4:1:1-style subsampling like `fox410.jpg`. |
| `721c88c0` | `test(jpeg): unit tests covering all newly-supported variants` — six corpus-driven tests (baseline 3-comp, progressive 3-comp, grayscale, YCCK+APP14, 12-bit, MJPEG-no-DHT) gated on `feature = "host"` and graceful-skip if corpus absent. |

## Test results

`tools/surf-host/src/jpeg_corpus.rs` decodes all 257 valid corpus JPEGs
and compares against the `image` crate (libjpeg-rs based).

| Metric | Before | After | Δ |
|--------|--------|-------|---|
| libimage decoded ok | 214 | **230** | +16 (+7.5%) |
| libimage rejected (rc<0) | 43 | 27 | -16 |
| Both decoders ok, pixels match (≤4 channel diff) | 57 | 66 | +9 |
| `image`-only success | 28 | 15 | -13 |
| Neither decoder | 15 | 12 | -3 |

`cargo +stable test --release --features host --lib` runs all six new
corpus unit tests passing in ~0.15s (when corpus present).

Both anyOS build (`cargo build --target ../../x86_64-anyos.json -Z build-std=core,alloc`)
and surf-host (`./build.sh`) compile cleanly.

## Features still missing and why

| Missing | Corpus impact | Rationale |
|---------|---------------|-----------|
| **Arithmetic coding (SOF9/10/11, jdarith.c)** | 3 files (1.2%) | jdarith.c is ~800 lines; effort/benefit unfavourable. The format is essentially unused on the open web — Surf is unlikely to ever encounter one. |
| **Lossless JPEG (SOF3)** | 0 files in corpus | Different code path entirely (DPCM rather than DCT); never used on the web. |
| **Hierarchical JPEG (SOF5/6/7/13/14/15)** | 0 files in corpus | Essentially unused. |
| **Fancy chroma upsampling (jdmerge.c)** | ~60 files diff in pixel comparison | Quality improvement, not a correctness gap. We use box (nearest-neighbor) upsampling; libjpeg/image-rs default to "fancy" (bilinear-ish) smooth upsampling. The diffs against the reference are typically max 5–30 per channel — visually subtle. Postponed for a follow-up; the file structure in `color_convert()` is now ready for it. |
| **Scale-down decoding (1/2, 1/4, 1/8 via reduced IDCT)** | n/a | Performance optimisation for thumbnails, not a correctness gap. Surf already rescales after decode. |
| **DNL marker** | 0 files | Edge case (height=0 then DNL); not seen in corpus. |
| **`partial_progressive.jpg` / `blank_800x280.jpg`** | 2 files | Specific edge cases (`image` crate succeeds with truncated/odd files via best-effort). Investigating each would take hours; deferred. |

## Notable details

- The IDCT tower is unchanged for 8-bit input (still goes through the
  `i32`-bias `idct_1d`); only 12-bit takes the new `idct_1d_i64` path,
  so we don't pay any overhead in the common case.
- The standard Huffman tables are installed *after* DHT parsing, only
  in still-empty slots. A DHT marker for any of DC0/DC1/AC0/AC1 wins.
- Adobe APP14 parsing only matches when the marker payload literally
  starts with `"Adobe"` and is at least 14 bytes — anything else is
  ignored, so non-Adobe APP14 sub-uses (rare) don't accidentally
  switch our color path.

## Suggested follow-ups

1. **Fancy chroma upsampling** — port `jdsample.c`'s `h2v2_fancy_upsample`
   / `h2v1_fancy_upsample` into `color_convert()`. ~100 lines, would
   bring pixel-match count from 66 to perhaps 130+ of 230.
2. **Arithmetic coding** — port `jdarith.c` (~800 lines) if a real
   PDF/Photoshop arithmetic-coded JPEG ever surfaces.
3. **Robust truncated-stream recovery** — currently we return ERR on
   any out-of-data; the `image` crate fills with whatever it has. For
   browser robustness it might be worth completing partial scans with
   black pixels rather than failing the whole image.
4. **`partial_progressive.jpg` and `blank_800x280.jpg`** — likely
   single-bit bugs in our progressive end-of-scan detection / 1-comp
   2x2-sampled file handling. Worth a focused debugging session.

## Files changed

- `libs/libimage/src/jpeg.rs` — main decoder
- `libs/libimage/src/jpeg_tables.rs` — standard Huffman tables
- `tools/surf-host/Cargo.toml` — registered `jpeg-corpus` bin
- `tools/surf-host/src/jpeg_corpus.rs` — new test harness
- `JPEG_PORT_NOTES.md` — phase 1 survey + plan
- `JPEG_PORT_REPORT.md` — this file
- `.gitignore` — added `third_party/codec-corpus/` and `third_party/libjpeg-reference/`
