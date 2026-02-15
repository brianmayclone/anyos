// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! MJV (Motion JPEG Video) container parser.
//!
//! Format: 32-byte header + frame table + concatenated JPEG frames.
//! Each frame is a standalone JPEG decoded via `crate::jpeg`.

use crate::types::*;

/// MJV header size in bytes.
const MJV_HEADER_SIZE: usize = 32;
/// Frame table entry size: offset(u32) + size(u32) = 8 bytes.
const FRAME_ENTRY_SIZE: usize = 8;

fn read_u32_le(data: &[u8], off: usize) -> u32 {
    (data[off] as u32)
        | ((data[off + 1] as u32) << 8)
        | ((data[off + 2] as u32) << 16)
        | ((data[off + 3] as u32) << 24)
}

/// Probe an MJV file and return video metadata.
///
/// Validates the "MJV1" magic, parses the header, and probes the first
/// JPEG frame to determine the scratch buffer size needed for decoding.
pub fn probe(data: &[u8]) -> Option<VideoInfo> {
    if data.len() < MJV_HEADER_SIZE {
        return None;
    }

    // Check magic
    if &data[0..4] != b"MJV1" {
        return None;
    }

    let width = read_u32_le(data, 8);
    let height = read_u32_le(data, 12);
    let fps = read_u32_le(data, 16);
    let num_frames = read_u32_le(data, 20);

    if width == 0 || height == 0 || fps == 0 || num_frames == 0 {
        return None;
    }

    // Validate frame table fits in the file
    let table_end = MJV_HEADER_SIZE + (num_frames as usize) * FRAME_ENTRY_SIZE;
    if data.len() < table_end {
        return None;
    }

    // Probe first JPEG frame to determine scratch_needed
    let scratch_needed = if let Some(jpeg_data) = frame_data(data, num_frames, 0) {
        match crate::jpeg::probe(jpeg_data) {
            Some(info) => info.scratch_needed,
            None => return None, // First frame isn't valid JPEG
        }
    } else {
        return None;
    };

    Some(VideoInfo {
        width,
        height,
        fps,
        num_frames,
        scratch_needed,
    })
}

/// Extract the raw JPEG data for a given frame index.
fn frame_data(data: &[u8], num_frames: u32, idx: u32) -> Option<&[u8]> {
    if idx >= num_frames {
        return None;
    }

    let entry_off = MJV_HEADER_SIZE + (idx as usize) * FRAME_ENTRY_SIZE;
    if entry_off + FRAME_ENTRY_SIZE > data.len() {
        return None;
    }

    let offset = read_u32_le(data, entry_off) as usize;
    let size = read_u32_le(data, entry_off + 4) as usize;

    if size == 0 || offset + size > data.len() {
        return None;
    }

    Some(&data[offset..offset + size])
}

/// Decode a single video frame into ARGB8888 pixels.
///
/// Looks up the frame in the frame table, extracts its JPEG data,
/// and decodes it using the existing JPEG decoder.
pub fn decode_frame(
    data: &[u8],
    num_frames: u32,
    frame_idx: u32,
    out: &mut [u32],
    scratch: &mut [u8],
) -> i32 {
    let jpeg_data = match frame_data(data, num_frames, frame_idx) {
        Some(d) => d,
        None => return ERR_INVALID_DATA,
    };

    crate::jpeg::decode(jpeg_data, out, scratch)
}
