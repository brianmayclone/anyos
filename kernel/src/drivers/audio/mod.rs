// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Audio driver subsystem.
//!
//! Provides AC'97 codec driver with DMA playback. The driver registers
//! with the HAL and can be accessed through audio syscalls.

pub mod ac97;

/// Write PCM samples to the audio output.
///
/// `data` must contain 16-bit signed little-endian stereo samples at 48 kHz
/// (4 bytes per sample frame: L16 + R16). Returns number of bytes accepted.
pub fn write_pcm(data: &[u8]) -> usize {
    ac97::write_pcm(data)
}

/// Stop all audio playback.
pub fn stop() {
    ac97::stop();
}

/// Set master volume (0–100).
pub fn set_volume(vol: u8) {
    ac97::set_volume(vol);
}

/// Get current master volume (0–100).
pub fn get_volume() -> u8 {
    ac97::get_volume()
}

/// Check if audio hardware is available and initialized.
pub fn is_available() -> bool {
    ac97::is_available()
}

/// Check if playback is currently active.
pub fn is_playing() -> bool {
    ac97::is_playing()
}
