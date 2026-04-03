//! Software audio mixer — mixes multiple PCM streams into a single output.
//!
//! Each audio source (application) gets a mixer channel with independent volume.
//! The mixer combines all active channels, applies master volume, clips to 16-bit,
//! and sends the result to the hardware audio driver.
//!
//! Format: 16-bit signed LE stereo, 48 kHz (4 bytes per sample frame).

use alloc::vec;
use alloc::vec::Vec;
use crate::sync::spinlock::Spinlock;

/// Maximum number of simultaneous mixer channels.
const MAX_CHANNELS: usize = 16;

/// Size of each channel's ring buffer in bytes (48 KiB ≈ 250ms at 48kHz stereo 16-bit).
const CHANNEL_BUF_SIZE: usize = 48 * 1024;

/// Size of the mixer output buffer (4 KiB = ~21ms, matches HDA/AC97 DMA buffer).
const MIX_BUF_FRAMES: usize = 1024; // sample frames
const MIX_BUF_BYTES: usize = MIX_BUF_FRAMES * 4; // L16 + R16 per frame

/// A single mixer channel with its own ring buffer and volume.
struct MixerChannel {
    /// Channel is in use.
    active: bool,
    /// Per-channel volume (0-100).
    volume: u8,
    /// Owning process ID (for cleanup on exit).
    pid: u32,
    /// Ring buffer for PCM data.
    buf: Vec<u8>,
    /// Read position in ring buffer.
    read_pos: usize,
    /// Number of valid bytes in buffer (read_pos..read_pos+available).
    available: usize,
}

impl MixerChannel {
    const fn new() -> Self {
        MixerChannel {
            active: false,
            volume: 100,
            pid: 0,
            buf: Vec::new(),
            read_pos: 0,
            available: 0,
        }
    }

    /// Write PCM data into the channel buffer. Returns bytes written.
    fn write(&mut self, data: &[u8]) -> usize {
        if self.buf.is_empty() {
            self.buf = vec![0u8; CHANNEL_BUF_SIZE];
        }
        let free = CHANNEL_BUF_SIZE - self.available;
        let to_write = data.len().min(free);
        if to_write == 0 { return 0; }

        let write_pos = (self.read_pos + self.available) % CHANNEL_BUF_SIZE;
        // Copy in up to two chunks (wrap around ring buffer)
        let first = to_write.min(CHANNEL_BUF_SIZE - write_pos);
        self.buf[write_pos..write_pos + first].copy_from_slice(&data[..first]);
        if to_write > first {
            let second = to_write - first;
            self.buf[..second].copy_from_slice(&data[first..first + second]);
        }
        self.available += to_write;
        to_write
    }

    /// Read `n` bytes from the channel buffer. Returns actual bytes read.
    fn read(&mut self, out: &mut [u8], n: usize) -> usize {
        let to_read = n.min(self.available).min(out.len());
        if to_read == 0 { return 0; }

        let first = to_read.min(CHANNEL_BUF_SIZE - self.read_pos);
        out[..first].copy_from_slice(&self.buf[self.read_pos..self.read_pos + first]);
        if to_read > first {
            let second = to_read - first;
            out[first..first + second].copy_from_slice(&self.buf[..second]);
        }
        self.read_pos = (self.read_pos + to_read) % CHANNEL_BUF_SIZE;
        self.available -= to_read;
        to_read
    }
}

/// The global audio mixer state.
pub struct AudioMixer {
    channels: [MixerChannel; MAX_CHANNELS],
    /// Master volume (0-100).
    master_volume: u8,
    /// Temporary mix buffer (32-bit samples for headroom during mixing).
    mix_buf: Vec<i32>,
}

impl AudioMixer {
    pub const fn new() -> Self {
        // const fn can't allocate, channels/mix_buf initialized lazily
        AudioMixer {
            channels: [
                MixerChannel::new(), MixerChannel::new(), MixerChannel::new(), MixerChannel::new(),
                MixerChannel::new(), MixerChannel::new(), MixerChannel::new(), MixerChannel::new(),
                MixerChannel::new(), MixerChannel::new(), MixerChannel::new(), MixerChannel::new(),
                MixerChannel::new(), MixerChannel::new(), MixerChannel::new(), MixerChannel::new(),
            ],
            master_volume: 100,
            mix_buf: Vec::new(),
        }
    }

    /// Allocate a mixer channel for a process. Returns channel ID or None.
    pub fn open_channel(&mut self, pid: u32) -> Option<u8> {
        for (i, ch) in self.channels.iter_mut().enumerate() {
            if !ch.active {
                ch.active = true;
                ch.volume = 100;
                ch.pid = pid;
                ch.read_pos = 0;
                ch.available = 0;
                return Some(i as u8);
            }
        }
        None
    }

    /// Release a mixer channel.
    pub fn close_channel(&mut self, channel_id: u8) {
        if (channel_id as usize) < MAX_CHANNELS {
            self.channels[channel_id as usize].active = false;
            self.channels[channel_id as usize].available = 0;
        }
    }

    /// Release all channels owned by a process (for cleanup on exit).
    pub fn close_channels_for_pid(&mut self, pid: u32) {
        for ch in &mut self.channels {
            if ch.active && ch.pid == pid {
                ch.active = false;
                ch.available = 0;
            }
        }
    }

    /// Write PCM data into a specific channel. Returns bytes accepted.
    pub fn write_channel(&mut self, channel_id: u8, data: &[u8]) -> usize {
        if (channel_id as usize) >= MAX_CHANNELS { return 0; }
        let ch = &mut self.channels[channel_id as usize];
        if !ch.active { return 0; }
        ch.write(data)
    }

    /// Set volume for a specific channel (0-100).
    pub fn set_channel_volume(&mut self, channel_id: u8, vol: u8) {
        if (channel_id as usize) < MAX_CHANNELS {
            self.channels[channel_id as usize].volume = vol.min(100);
        }
    }

    /// Set master volume (0-100).
    pub fn set_master_volume(&mut self, vol: u8) {
        self.master_volume = vol.min(100);
    }

    /// Get master volume.
    pub fn master_volume(&self) -> u8 {
        self.master_volume
    }

    /// Mix all active channels and produce output PCM data.
    /// Returns a slice of mixed 16-bit stereo PCM, or empty if no channels have data.
    pub fn mix(&mut self, output: &mut [u8]) -> usize {
        let frames = output.len() / 4; // 4 bytes per frame (L16 + R16)
        if frames == 0 { return 0; }
        let samples = frames * 2; // L + R per frame

        // Ensure mix buffer is allocated
        if self.mix_buf.len() < samples {
            self.mix_buf = vec![0i32; samples];
        }

        // Zero mix buffer
        for s in &mut self.mix_buf[..samples] { *s = 0; }

        let mut any_data = false;
        let read_bytes = frames * 4;
        let mut tmp = vec![0u8; read_bytes];

        for ch in &mut self.channels {
            if !ch.active || ch.available == 0 { continue; }

            let got = ch.read(&mut tmp, read_bytes);
            if got == 0 { continue; }
            any_data = true;

            let ch_vol = ch.volume as i32;
            let sample_count = got / 2; // 16-bit samples

            for s in 0..sample_count {
                let sample = i16::from_le_bytes([
                    tmp[s * 2],
                    tmp[s * 2 + 1],
                ]) as i32;
                // Apply per-channel volume
                self.mix_buf[s] += (sample * ch_vol) / 100;
            }
        }

        if !any_data { return 0; }

        // Apply master volume and clip to i16 range
        let master = self.master_volume as i32;
        let out_samples = frames * 2;
        for s in 0..out_samples {
            let mixed = (self.mix_buf[s] * master) / 100;
            let clipped = mixed.max(-32768).min(32767) as i16;
            let bytes = clipped.to_le_bytes();
            output[s * 2] = bytes[0];
            output[s * 2 + 1] = bytes[1];
        }

        out_samples * 2 // bytes written
    }

    /// Check if any channel has data available for mixing.
    pub fn has_data(&self) -> bool {
        self.channels.iter().any(|ch| ch.active && ch.available > 0)
    }

    /// Get number of active channels.
    pub fn active_channels(&self) -> u8 {
        self.channels.iter().filter(|ch| ch.active).count() as u8
    }
}


// ── Global mixer instance ───────────────────────────────────────────────────

static MIXER: Spinlock<AudioMixer> = Spinlock::new(AudioMixer::new());

/// Open a mixer channel for a process. Returns channel ID (0-15) or None.
pub fn open_channel(pid: u32) -> Option<u8> {
    MIXER.lock().open_channel(pid)
}

/// Close a mixer channel.
pub fn close_channel(channel_id: u8) {
    MIXER.lock().close_channel(channel_id);
}

/// Close all mixer channels for a process (cleanup on exit).
pub fn close_channels_for_pid(pid: u32) {
    MIXER.lock().close_channels_for_pid(pid);
}

/// Write PCM data to a mixer channel. Returns bytes accepted.
pub fn write_channel(channel_id: u8, data: &[u8]) -> usize {
    MIXER.lock().write_channel(channel_id, data)
}

/// Set per-channel volume (0-100).
pub fn set_channel_volume(channel_id: u8, vol: u8) {
    MIXER.lock().set_channel_volume(channel_id, vol);
}

/// Set master volume (0-100).
pub fn set_master_volume(vol: u8) {
    MIXER.lock().set_master_volume(vol);
}

/// Get master volume.
pub fn master_volume() -> u8 {
    MIXER.lock().master_volume()
}

/// Mix all channels and write to hardware. Returns bytes sent to hardware.
pub fn flush_to_hardware() -> usize {
    let mut output = [0u8; MIX_BUF_BYTES];
    let mixed = MIXER.lock().mix(&mut output);
    if mixed > 0 {
        super::write_pcm(&output[..mixed])
    } else {
        0
    }
}

/// Check if any mixer channel has pending data.
pub fn has_pending_data() -> bool {
    MIXER.lock().has_data()
}
