// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Constant tables for the baseline JPEG decoder.

/// Zig-zag scan order: maps coefficient index 0..63 to the (row*8+col) position
/// inside an 8x8 block.
pub const ZIGZAG: [u8; 64] = [
     0,  1,  8, 16,  9,  2,  3, 10,
    17, 24, 32, 25, 18, 11,  4,  5,
    12, 19, 26, 33, 40, 48, 41, 34,
    27, 20, 13,  6,  7, 14, 21, 28,
    35, 42, 49, 56, 57, 50, 43, 36,
    29, 22, 15, 23, 30, 37, 44, 51,
    58, 59, 52, 45, 38, 31, 39, 46,
    53, 60, 61, 54, 47, 55, 62, 63,
];

/// Inverse zig-zag: maps (row*8+col) position back to coefficient index.
pub const IZIGZAG: [u8; 64] = [
     0,  1,  5,  6, 14, 15, 27, 28,
     2,  4,  7, 13, 16, 26, 29, 42,
     3,  8, 12, 17, 25, 30, 41, 43,
     9, 11, 18, 24, 31, 40, 44, 53,
    10, 19, 23, 32, 39, 45, 52, 54,
    20, 22, 33, 38, 46, 51, 55, 60,
    21, 34, 37, 47, 50, 56, 59, 61,
    35, 36, 48, 49, 57, 58, 62, 63,
];

/// AAN IDCT prescale factors in Q15 fixed-point (scaled by 2^15 = 32768).
///
/// Each entry is `round(cos(k*pi/16) * sqrt(2) * 32768)` for k = 0..7,
/// with the 1/sqrt(8) normalization folded in later during dequant.
///
/// Row i, col j prescale = AANSCALES[i] * AANSCALES[j] >> 15.
/// These are used with the AAN (Arai-Agui-Nakajima) fast IDCT algorithm.
pub const AANSCALES: [i32; 8] = [
    16384, // cos(0)       * sqrt(2) * 2^14 = 1.0 * 1.414 * 16384 ≈ 23170 ...
           // Actually we use simpler uniform scale: 2^14
    22725, // cos(pi/16)   * sqrt(2) * 2^14
    21407, // cos(2*pi/16) * sqrt(2) * 2^14
    19266, // cos(3*pi/16) * sqrt(2) * 2^14
    16384, // cos(4*pi/16) * sqrt(2) * 2^14
    12873, // cos(5*pi/16) * sqrt(2) * 2^14
     8867, // cos(6*pi/16) * sqrt(2) * 2^14
     4520, // cos(7*pi/16) * sqrt(2) * 2^14
];

/// IDCT constants in Q13 fixed-point for the LLM (Loeffler-Ligtenberg-Moschytz) algorithm.
/// These represent: C1 = cos(pi/16)*sqrt(2), C2 = cos(2*pi/16)*sqrt(2), etc.
///
/// FIX_0_298 .. FIX_3_072 are derived from the rotation constants used in
/// the LLM decomposition of the 8-point DCT.
pub const FIX_0_298: i32 = 2446;   // 0.298631336 * 2^13
pub const FIX_0_390: i32 = 3196;   // 0.390180644 * 2^13
pub const FIX_0_541: i32 = 4433;   // 0.541196100 * 2^13
pub const FIX_0_765: i32 = 6270;   // 0.765366865 * 2^13
pub const FIX_0_899: i32 = 7373;   // 0.899976223 * 2^13
pub const FIX_1_175: i32 = 9633;   // 1.175875602 * 2^13
pub const FIX_1_501: i32 = 12299;  // 1.501321110 * 2^13
pub const FIX_1_847: i32 = 15137;  // 1.847759065 * 2^13
pub const FIX_1_961: i32 = 16069;  // 1.961570560 * 2^13
pub const FIX_2_053: i32 = 16819;  // 2.053119869 * 2^13
pub const FIX_2_562: i32 = 20995;  // 2.562915447 * 2^13
pub const FIX_3_072: i32 = 25172;  // 3.072711026 * 2^13

/// Default luminance quantization table (JPEG Annex K, Table K.1).
/// Used when the file omits a DQT marker (rare, but useful for reference).
pub const DEFAULT_LUMA_QUANT: [u8; 64] = [
    16, 11, 10, 16,  24,  40,  51,  61,
    12, 12, 14, 19,  26,  58,  60,  55,
    14, 13, 16, 24,  40,  57,  69,  56,
    14, 17, 22, 29,  51,  87,  80,  62,
    18, 22, 37, 56,  68, 109, 103,  77,
    24, 35, 55, 64,  81, 104, 113,  92,
    49, 64, 78, 87, 103, 121, 120, 101,
    72, 92, 95, 98, 112, 100, 103,  99,
];

/// Default chrominance quantization table (JPEG Annex K, Table K.2).
pub const DEFAULT_CHROMA_QUANT: [u8; 64] = [
    17, 18, 24, 47, 99, 99, 99, 99,
    18, 21, 26, 66, 99, 99, 99, 99,
    24, 26, 56, 99, 99, 99, 99, 99,
    47, 66, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99,
];
