// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! Host-mode IPC stubs for unit tests.

pub fn pipe_bytes_available_fd(_fd: u32) -> u32 {
    0
}

pub fn pipe_create(_name: &str) -> u32 {
    1
}

pub fn pipe_read(_pipe_id: u32, _buf: &mut [u8]) -> u32 {
    0
}

pub fn pipe_write(_pipe_id: u32, data: &[u8]) -> u32 {
    data.len() as u32
}

pub fn pipe_open(_name: &str) -> u32 {
    1
}

pub fn pipe_close(_pipe_id: u32) -> u32 {
    0
}

pub fn evt_sys_subscribe(_filter: u32) -> u32 {
    0
}

pub fn evt_sys_poll(_sub_id: u32, _buf: &mut [u32; 5]) -> bool {
    false
}

pub fn evt_sys_unsubscribe(_sub_id: u32) {}

pub fn evt_chan_create(_name: &str) -> u32 {
    0
}

pub fn evt_chan_subscribe(_channel_id: u32, _filter: u32) -> u32 {
    0
}

pub fn evt_chan_emit(_channel_id: u32, _event: &[u32; 5]) {}

pub fn evt_chan_emit_to(_channel_id: u32, _sub_id: u32, _event: &[u32; 5]) {}

pub fn evt_chan_poll(_channel_id: u32, _sub_id: u32, _buf: &mut [u32; 5]) -> bool {
    false
}

pub fn evt_chan_unsubscribe(_channel_id: u32, _sub_id: u32) {}

pub fn evt_chan_destroy(_channel_id: u32) {}

pub fn evt_chan_wait(_channel_id: u32, _sub_id: u32, _timeout_ms: u32) -> u32 {
    0
}

pub fn shm_create(_size: u32) -> u32 {
    0
}

pub fn shm_map(_shm_id: u32) -> u32 {
    0
}

pub fn shm_unmap(_shm_id: u32) -> u32 {
    0
}

pub fn shm_destroy(_shm_id: u32) -> u32 {
    0
}

pub fn register_sessionhost() -> u32 {
    0
}

pub fn register_compositor() -> u32 {
    0
}

#[repr(C)]
pub struct FbMapInfo {
    pub fb_addr: u32,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
}

pub fn map_framebuffer() -> Option<FbMapInfo> {
    None
}

pub fn grant_framebuffer(_target_tid: u32, _out_info: &mut FbMapInfo) -> u32 {
    u32::MAX
}

pub fn revoke_framebuffer(_target_tid: u32) -> u32 {
    u32::MAX
}

pub fn gpu_command(_cmds: &[[u32; 9]]) -> u32 {
    0
}

pub fn gpu_vram_size() -> u32 {
    0
}

pub fn vram_map(_target_tid: u32, _vram_byte_offset: u32, _num_bytes: u32) -> u32 {
    0
}

pub fn gpu_register_backbuffer(_buf_ptr: u32, _buf_size: u32) -> u32 {
    u32::MAX
}

pub fn input_poll(_buf: &mut [[u32; 5]]) -> u32 {
    0
}
