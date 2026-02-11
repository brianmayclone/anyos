// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! play — WAV audio file player for anyOS.
//!
//! Usage: play <file.wav>

#![no_std]
#![no_main]

use anyos_std::Vec;

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let path = anyos_std::process::args(&mut args_buf).trim();

    if path.is_empty() {
        anyos_std::println!("Usage: play <file.wav>");
        return;
    }

    if !anyos_std::audio::audio_is_available() {
        anyos_std::println!("play: no audio device available");
        return;
    }

    // Read file
    let fd = anyos_std::fs::open(path, 0);
    if fd == u32::MAX {
        anyos_std::println!("play: cannot open '{}'", path);
        return;
    }

    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = anyos_std::fs::read(fd, &mut buf);
        if n == 0 || n == u32::MAX { break; }
        data.extend_from_slice(&buf[..n as usize]);
    }
    anyos_std::fs::close(fd);

    if data.is_empty() {
        anyos_std::println!("play: empty file");
        return;
    }

    anyos_std::println!("Playing {}...", path);

    match anyos_std::audio::play_wav(&data) {
        Ok(()) => {
            // Wait for playback to finish
            while anyos_std::audio::audio_is_playing() {
                anyos_std::process::sleep(10); // poll audio status, not busy-wait
            }
            anyos_std::println!("Done.");
        }
        Err(e) => {
            anyos_std::println!("play: {}", e);
        }
    }
}
