// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Audio playback API.
//!
//! Provides functions to write PCM audio data, control volume, and play WAV files.
//! Audio output is 48 kHz, 16-bit signed stereo (native AC'97 format).

use crate::raw::*;

/// Write raw PCM data to the audio output.
///
/// `data` must contain 16-bit signed little-endian stereo samples at 48 kHz
/// (4 bytes per sample frame). Returns number of bytes accepted.
pub fn audio_write(pcm_data: &[u8]) -> u32 {
    syscall2(SYS_AUDIO_WRITE, pcm_data.as_ptr() as u64, pcm_data.len() as u64)
}

/// Stop audio playback.
pub fn audio_stop() {
    syscall2(SYS_AUDIO_CTL, 0, 0);
}

/// Set master volume (0 = mute, 100 = max).
pub fn audio_set_volume(vol: u8) {
    syscall2(SYS_AUDIO_CTL, 1, vol as u64);
}

/// Get current master volume (0-100).
pub fn audio_get_volume() -> u8 {
    syscall2(SYS_AUDIO_CTL, 2, 0) as u8
}

/// Check if audio playback is active.
pub fn audio_is_playing() -> bool {
    syscall2(SYS_AUDIO_CTL, 3, 0) != 0
}

/// Check if audio hardware is available.
pub fn audio_is_available() -> bool {
    syscall2(SYS_AUDIO_CTL, 4, 0) != 0
}

/// Parse and play a WAV file from raw bytes.
///
/// Supports: PCM format, 8/16-bit, mono/stereo.
/// Resamples to 48 kHz if needed (nearest-neighbor).
pub fn play_wav(data: &[u8]) -> Result<(), &'static str> {
    let wav = parse_wav(data)?;

    // Convert to 48 kHz 16-bit stereo
    let pcm = convert_wav(&wav)?;

    audio_write(&pcm);
    Ok(())
}

struct WavInfo<'a> {
    channels: u16,
    sample_rate: u32,
    bits_per_sample: u16,
    pcm_data: &'a [u8],
}

fn parse_wav(data: &[u8]) -> Result<WavInfo<'_>, &'static str> {
    if data.len() < 44 {
        return Err("WAV too short");
    }

    // RIFF header
    if &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err("Not a WAV file");
    }

    // Find "fmt " chunk
    let mut pos = 12;
    let mut channels = 0u16;
    let mut sample_rate = 0u32;
    let mut bits_per_sample = 0u16;
    let mut fmt_found = false;

    while pos + 8 <= data.len() {
        let chunk_id = &data[pos..pos + 4];
        let chunk_size = u32::from_le_bytes([data[pos+4], data[pos+5], data[pos+6], data[pos+7]]) as usize;
        pos += 8;

        if chunk_id == b"fmt " {
            if chunk_size < 16 || pos + 16 > data.len() {
                return Err("Invalid fmt chunk");
            }
            let audio_format = u16::from_le_bytes([data[pos], data[pos+1]]);
            if audio_format != 1 {
                return Err("Not PCM format");
            }
            channels = u16::from_le_bytes([data[pos+2], data[pos+3]]);
            sample_rate = u32::from_le_bytes([data[pos+4], data[pos+5], data[pos+6], data[pos+7]]);
            bits_per_sample = u16::from_le_bytes([data[pos+14], data[pos+15]]);
            fmt_found = true;
            pos += chunk_size;
            break;
        }

        pos += chunk_size;
        // Chunks are word-aligned
        if chunk_size & 1 != 0 { pos += 1; }
    }

    if !fmt_found {
        return Err("No fmt chunk");
    }

    // Find "data" chunk
    while pos + 8 <= data.len() {
        let chunk_id = &data[pos..pos + 4];
        let chunk_size = u32::from_le_bytes([data[pos+4], data[pos+5], data[pos+6], data[pos+7]]) as usize;
        pos += 8;

        if chunk_id == b"data" {
            let end = (pos + chunk_size).min(data.len());
            return Ok(WavInfo {
                channels,
                sample_rate,
                bits_per_sample,
                pcm_data: &data[pos..end],
            });
        }

        pos += chunk_size;
        if chunk_size & 1 != 0 { pos += 1; }
    }

    Err("No data chunk")
}

fn convert_wav(wav: &WavInfo) -> Result<alloc::vec::Vec<u8>, &'static str> {
    if wav.channels == 0 || wav.channels > 2 {
        return Err("Unsupported channel count");
    }
    if wav.bits_per_sample != 8 && wav.bits_per_sample != 16 {
        return Err("Unsupported bit depth");
    }
    if wav.sample_rate == 0 {
        return Err("Invalid sample rate");
    }

    let bytes_per_sample = (wav.bits_per_sample / 8) as usize;
    let frame_size = bytes_per_sample * wav.channels as usize;
    let num_frames = wav.pcm_data.len() / frame_size;

    if num_frames == 0 {
        return Err("No audio data");
    }

    // Output: 48000 Hz, 16-bit stereo = 4 bytes per frame
    let out_frames = ((num_frames as u64 * 48000) / wav.sample_rate as u64) as usize;
    let mut out = alloc::vec![0u8; out_frames * 4];

    for i in 0..out_frames {
        // Nearest-neighbor resample
        let src_frame = ((i as u64 * wav.sample_rate as u64) / 48000) as usize;
        let src_frame = src_frame.min(num_frames - 1);
        let src_off = src_frame * frame_size;

        let (left, right) = if wav.bits_per_sample == 16 {
            let l = i16::from_le_bytes([wav.pcm_data[src_off], wav.pcm_data[src_off + 1]]);
            let r = if wav.channels == 2 {
                i16::from_le_bytes([wav.pcm_data[src_off + 2], wav.pcm_data[src_off + 3]])
            } else {
                l
            };
            (l, r)
        } else {
            // 8-bit unsigned → 16-bit signed
            let l = ((wav.pcm_data[src_off] as i16) - 128) * 256;
            let r = if wav.channels == 2 {
                ((wav.pcm_data[src_off + 1] as i16) - 128) * 256
            } else {
                l
            };
            (l, r)
        };

        let dst = i * 4;
        out[dst..dst + 2].copy_from_slice(&left.to_le_bytes());
        out[dst + 2..dst + 4].copy_from_slice(&right.to_le_bytes());
    }

    Ok(out)
}
