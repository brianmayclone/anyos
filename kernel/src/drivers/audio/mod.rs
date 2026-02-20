// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Audio driver subsystem.
//!
//! Provides a unified [`AudioDriver`] trait for audio output drivers (AC'97, Intel HDA, etc.).
//! Drivers register dynamically via PCI detection in the HAL.
//! Userspace accesses audio through SYS_AUDIO_WRITE / SYS_AUDIO_CTL syscalls.

pub mod ac97;
pub mod hda;

use alloc::boxed::Box;
use crate::sync::spinlock::Spinlock;

/// Unified audio driver interface.
pub trait AudioDriver: Send {
    /// Human-readable driver name.
    fn name(&self) -> &str;
    /// Write PCM samples (16-bit signed LE stereo, 48 kHz). Returns bytes consumed.
    fn write_pcm(&mut self, data: &[u8]) -> usize;
    /// Stop all playback.
    fn stop(&mut self);
    /// Set master volume (0–100).
    fn set_volume(&mut self, vol: u8);
    /// Get current master volume (0–100).
    fn get_volume(&self) -> u8;
    /// Check if playback is currently active.
    fn is_playing(&self) -> bool;
    /// Get the sample rate in Hz (typically 48000).
    fn sample_rate(&self) -> u32;
}

/// Global audio driver instance, set during PCI probe.
static AUDIO: Spinlock<Option<Box<dyn AudioDriver>>> = Spinlock::new(None);

/// Register an audio driver (called from driver init during PCI probe).
pub fn register(driver: Box<dyn AudioDriver>) {
    crate::serial_println!("  Audio: registered '{}'", driver.name());
    let mut audio = AUDIO.lock();
    *audio = Some(driver);
}

/// Access the registered audio driver within a closure.
pub fn with_audio<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut dyn AudioDriver) -> R,
{
    let mut audio = AUDIO.lock();
    let driver = audio.as_mut()?;
    Some(f(driver.as_mut()))
}

/// Write PCM samples to the audio output.
///
/// `data` must contain 16-bit signed little-endian stereo samples at 48 kHz
/// (4 bytes per sample frame: L16 + R16). Returns number of bytes accepted.
pub fn write_pcm(data: &[u8]) -> usize {
    with_audio(|d| d.write_pcm(data)).unwrap_or(0)
}

/// Stop all audio playback.
pub fn stop() {
    with_audio(|d| d.stop());
}

/// Set master volume (0–100).
pub fn set_volume(vol: u8) {
    with_audio(|d| d.set_volume(vol));
}

/// Get current master volume (0–100).
pub fn get_volume() -> u8 {
    with_audio(|d| d.get_volume()).unwrap_or(0)
}

/// Check if audio hardware is available and initialized.
pub fn is_available() -> bool {
    AUDIO.lock().is_some()
}

/// Check if playback is currently active.
pub fn is_playing() -> bool {
    with_audio(|d| d.is_playing()).unwrap_or(false)
}

// ── HAL integration ─────────────────────────────────────────────────────────

use crate::drivers::hal::{Driver, DriverType, DriverError,
    IOCTL_AUDIO_GET_SAMPLE_RATE, IOCTL_AUDIO_SET_VOLUME, IOCTL_AUDIO_GET_VOLUME};

struct AudioHalDriver {
    name: &'static str,
}

impl Driver for AudioHalDriver {
    fn name(&self) -> &str { self.name }
    fn driver_type(&self) -> DriverType { DriverType::Audio }
    fn init(&mut self) -> Result<(), DriverError> { Ok(()) }
    fn read(&self, _offset: usize, _buf: &mut [u8]) -> Result<usize, DriverError> {
        Err(DriverError::NotSupported)
    }
    fn write(&self, _offset: usize, buf: &[u8]) -> Result<usize, DriverError> {
        if !is_available() { return Err(DriverError::NotSupported); }
        Ok(write_pcm(buf))
    }
    fn ioctl(&mut self, cmd: u32, arg: u32) -> Result<u32, DriverError> {
        if !is_available() { return Err(DriverError::NotSupported); }
        match cmd {
            IOCTL_AUDIO_GET_SAMPLE_RATE => {
                Ok(with_audio(|d| d.sample_rate()).unwrap_or(48000))
            }
            IOCTL_AUDIO_SET_VOLUME => { set_volume(arg as u8); Ok(0) }
            IOCTL_AUDIO_GET_VOLUME => Ok(get_volume() as u32),
            _ => Err(DriverError::NotSupported),
        }
    }
}

/// Create a HAL Driver wrapper for the audio subsystem (called from driver probe).
pub(crate) fn create_hal_driver(name: &'static str) -> Option<Box<dyn Driver>> {
    Some(Box::new(AudioHalDriver { name }))
}
