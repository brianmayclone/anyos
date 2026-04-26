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

// ---------------------------------------------------------------------------
// Standard Huffman tables (T.81 Annex K), used when the JPEG omits DHT
// markers (e.g. MJPEG streams from AVI / webcams).
// `bits[0]` is unused (1-based in spec); only entries 1..=16 carry counts.
// ---------------------------------------------------------------------------

pub const STD_DC_LUMA_BITS: [u8; 16] =
    [0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0];
pub const STD_DC_LUMA_VALS: [u8; 12] =
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];

pub const STD_DC_CHROMA_BITS: [u8; 16] =
    [0, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0];
pub const STD_DC_CHROMA_VALS: [u8; 12] =
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];

pub const STD_AC_LUMA_BITS: [u8; 16] =
    [0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 0x7d];
pub const STD_AC_LUMA_VALS: [u8; 162] = [
    0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12,
    0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07,
    0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xa1, 0x08,
    0x23, 0x42, 0xb1, 0xc1, 0x15, 0x52, 0xd1, 0xf0,
    0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0a, 0x16,
    0x17, 0x18, 0x19, 0x1a, 0x25, 0x26, 0x27, 0x28,
    0x29, 0x2a, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39,
    0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49,
    0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59,
    0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69,
    0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79,
    0x7a, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89,
    0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98,
    0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7,
    0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6,
    0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5,
    0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4,
    0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda, 0xe1, 0xe2,
    0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea,
    0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8,
    0xf9, 0xfa,
];

pub const STD_AC_CHROMA_BITS: [u8; 16] =
    [0, 2, 1, 2, 4, 4, 3, 4, 7, 5, 4, 4, 0, 1, 2, 0x77];
pub const STD_AC_CHROMA_VALS: [u8; 162] = [
    0x00, 0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21,
    0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71,
    0x13, 0x22, 0x32, 0x81, 0x08, 0x14, 0x42, 0x91,
    0xa1, 0xb1, 0xc1, 0x09, 0x23, 0x33, 0x52, 0xf0,
    0x15, 0x62, 0x72, 0xd1, 0x0a, 0x16, 0x24, 0x34,
    0xe1, 0x25, 0xf1, 0x17, 0x18, 0x19, 0x1a, 0x26,
    0x27, 0x28, 0x29, 0x2a, 0x35, 0x36, 0x37, 0x38,
    0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48,
    0x49, 0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58,
    0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68,
    0x69, 0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78,
    0x79, 0x7a, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87,
    0x88, 0x89, 0x8a, 0x92, 0x93, 0x94, 0x95, 0x96,
    0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5,
    0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4,
    0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3,
    0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2,
    0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda,
    0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9,
    0xea, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8,
    0xf9, 0xfa,
];
