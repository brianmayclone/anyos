//! ARM64 audio subsystem shim.
//!
//! Keeps the shared kernel code using `crate::drivers::audio::*` while the
//! actual ARM64 hardware/audio backend is still being brought up.

#[path = "audio/mixer.rs"]
pub mod mixer;

/// ARM64 currently ships without an audio output backend.
pub fn write_pcm(_data: &[u8]) -> usize {
    0
}

/// No hardware audio backend is available yet on ARM64.
pub fn stop() {}

/// No-op until an ARM64 audio backend exists.
pub fn set_volume(_vol: u8) {}

/// Returns the default master volume used by the software mixer.
pub fn get_volume() -> u8 {
    mixer::master_volume()
}

/// No hardware audio backend is available yet on ARM64.
pub fn is_available() -> bool {
    false
}

/// Playback is effectively off without a backend.
pub fn is_playing() -> bool {
    false
}
