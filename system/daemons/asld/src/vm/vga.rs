use alloc::vec::Vec;

use super::{exit_reason, VmExitInfo};

pub(super) const VGA_FB_BASE: u64 = 0xfd00_0000;
pub(super) const VGA_FB_SIZE: u64 = 16 * 1024 * 1024;

const BGA_INDEX_PORT: u16 = 0x01ce;
const BGA_DATA_PORT: u16 = 0x01cf;

const BGA_INDEX_ID: usize = 0;
const BGA_INDEX_XRES: usize = 1;
const BGA_INDEX_YRES: usize = 2;
const BGA_INDEX_BPP: usize = 3;
const BGA_INDEX_ENABLE: usize = 4;
const BGA_INDEX_BANK: usize = 5;
const BGA_INDEX_VIRT_WIDTH: usize = 6;
const BGA_INDEX_VIRT_HEIGHT: usize = 7;
const BGA_INDEX_X_OFFSET: usize = 8;
const BGA_INDEX_Y_OFFSET: usize = 9;
const BGA_REG_COUNT: usize = 16;

const BGA_ID: u16 = 0xb0c5;
const BGA_ENABLE: u16 = 0x0001;
const MAX_WIDTH: u16 = 1024;
const MAX_HEIGHT: u16 = 768;
const PREVIEW_MAX_W: usize = 40;
const PREVIEW_MAX_H: usize = 30;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct GuestFramebuffer {
    index: u16,
    regs: [u16; BGA_REG_COUNT],
    buffer: Vec<u8>,
    dirty_writes: u32,
    last_publish_ms: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct VgaAction {
    pub(super) read_value: Option<u32>,
    pub(super) publish: bool,
}

impl Default for GuestFramebuffer {
    fn default() -> Self {
        let mut regs = [0u16; BGA_REG_COUNT];
        regs[BGA_INDEX_ID] = BGA_ID;
        regs[BGA_INDEX_XRES] = 1024;
        regs[BGA_INDEX_YRES] = 768;
        regs[BGA_INDEX_BPP] = 32;
        regs[BGA_INDEX_VIRT_WIDTH] = 1024;
        regs[BGA_INDEX_VIRT_HEIGHT] = 768;
        Self {
            index: BGA_INDEX_ID as u16,
            regs,
            buffer: alloc::vec![0; 1024 * 768 * 4],
            dirty_writes: 0,
            last_publish_ms: 0,
        }
    }
}

impl GuestFramebuffer {
    pub(super) fn io_action(&mut self, exit: &VmExitInfo) -> Option<VgaAction> {
        if exit.reason != exit_reason::IO_INSTRUCTION || !is_bga_port(exit.io_port) {
            return None;
        }
        if exit.is_read != 0 {
            let value = if exit.io_port == BGA_INDEX_PORT {
                self.index as u32
            } else {
                self.read_selected_reg() as u32
            };
            return Some(VgaAction {
                read_value: Some(mask_width(value, exit.access_size)),
                publish: false,
            });
        }

        if exit.io_port == BGA_INDEX_PORT {
            self.index = (exit.io_data as u16) & 0x0f;
        } else {
            self.write_selected_reg(exit.io_data as u16);
        }
        Some(VgaAction::default())
    }

    pub(super) fn mmio_action(&mut self, exit: &VmExitInfo) -> Option<VgaAction> {
        if exit.reason != exit_reason::EPT_VIOLATION || !is_vga_mmio_region(exit.guest_phys_addr) {
            return None;
        }
        let offset = exit.guest_phys_addr.saturating_sub(VGA_FB_BASE) as usize;
        if exit.is_read != 0 {
            return Some(VgaAction {
                read_value: Some(self.read_fb(offset, exit.access_size)),
                publish: false,
            });
        }
        self.write_fb(offset, exit.access_size, exit.io_data);
        Some(VgaAction {
            read_value: None,
            publish: self.note_dirty_write(),
        })
    }

    pub(super) fn preview_rgb565(&self) -> (u16, u16, Vec<u16>) {
        let width = self.width().max(1) as usize;
        let height = self.height().max(1) as usize;
        let preview_w = width.min(PREVIEW_MAX_W).max(1);
        let preview_h = height.min(PREVIEW_MAX_H).max(1);
        let mut pixels = Vec::with_capacity(preview_w * preview_h);
        for y in 0..preview_h {
            let sy = y * height / preview_h;
            for x in 0..preview_w {
                let sx = x * width / preview_w;
                pixels.push(self.pixel_rgb565(sx, sy));
            }
        }
        (preview_w as u16, preview_h as u16, pixels)
    }

    fn read_selected_reg(&self) -> u16 {
        let index = self.index as usize;
        if index < self.regs.len() {
            self.regs[index]
        } else {
            0
        }
    }

    fn write_selected_reg(&mut self, value: u16) {
        let index = self.index as usize;
        match index {
            BGA_INDEX_ID => self.regs[BGA_INDEX_ID] = BGA_ID,
            BGA_INDEX_XRES => self.regs[BGA_INDEX_XRES] = value.clamp(1, MAX_WIDTH),
            BGA_INDEX_YRES => self.regs[BGA_INDEX_YRES] = value.clamp(1, MAX_HEIGHT),
            BGA_INDEX_BPP => self.regs[BGA_INDEX_BPP] = normalize_bpp(value),
            BGA_INDEX_ENABLE => {
                self.regs[BGA_INDEX_ENABLE] = value;
                if value & BGA_ENABLE != 0 {
                    self.resize_for_mode();
                }
            }
            BGA_INDEX_BANK
            | BGA_INDEX_VIRT_WIDTH
            | BGA_INDEX_VIRT_HEIGHT
            | BGA_INDEX_X_OFFSET
            | BGA_INDEX_Y_OFFSET => self.regs[index] = value,
            _ if index < self.regs.len() => self.regs[index] = value,
            _ => {}
        }
    }

    fn resize_for_mode(&mut self) {
        let width = self.width() as usize;
        let height = self.height() as usize;
        let len = width
            .saturating_mul(height)
            .saturating_mul(self.bytes_per_pixel());
        self.buffer.resize(len.min(VGA_FB_SIZE as usize), 0);
        self.regs[BGA_INDEX_VIRT_WIDTH] = self.regs[BGA_INDEX_XRES];
        self.regs[BGA_INDEX_VIRT_HEIGHT] = self.regs[BGA_INDEX_YRES];
        self.dirty_writes = self.dirty_writes.saturating_add(1);
    }

    fn width(&self) -> u16 {
        self.regs[BGA_INDEX_XRES].clamp(1, MAX_WIDTH)
    }

    fn height(&self) -> u16 {
        self.regs[BGA_INDEX_YRES].clamp(1, MAX_HEIGHT)
    }

    fn bytes_per_pixel(&self) -> usize {
        match self.regs[BGA_INDEX_BPP] {
            8 => 1,
            15 | 16 => 2,
            24 => 3,
            _ => 4,
        }
    }

    fn read_fb(&self, offset: usize, access_size: u8) -> u32 {
        let mut value = 0u32;
        for index in 0..(access_size as usize).min(4) {
            if let Some(byte) = self.buffer.get(offset + index) {
                value |= (*byte as u32) << (index * 8);
            }
        }
        value
    }

    fn write_fb(&mut self, offset: usize, access_size: u8, value: u64) {
        for index in 0..(access_size as usize).min(8) {
            if let Some(byte) = self.buffer.get_mut(offset + index) {
                *byte = ((value >> (index * 8)) & 0xff) as u8;
            }
        }
    }

    fn note_dirty_write(&mut self) -> bool {
        self.dirty_writes = self.dirty_writes.saturating_add(1);
        if self.dirty_writes < 4096 {
            return false;
        }
        self.dirty_writes = 0;
        true
    }

    fn pixel_rgb565(&self, x: usize, y: usize) -> u16 {
        let bpp = self.bytes_per_pixel();
        let offset = (y * self.width() as usize + x).saturating_mul(bpp);
        let (r, g, b) = match self.regs[BGA_INDEX_BPP] {
            8 => {
                let v = self.buffer.get(offset).copied().unwrap_or(0);
                (v, v, v)
            }
            15 => {
                let raw = self.read_u16(offset);
                (
                    (((raw >> 10) & 0x1f) << 3) as u8,
                    (((raw >> 5) & 0x1f) << 3) as u8,
                    ((raw & 0x1f) << 3) as u8,
                )
            }
            16 => {
                let raw = self.read_u16(offset);
                (
                    (((raw >> 11) & 0x1f) << 3) as u8,
                    (((raw >> 5) & 0x3f) << 2) as u8,
                    ((raw & 0x1f) << 3) as u8,
                )
            }
            24 | 32 => (
                self.buffer.get(offset + 2).copied().unwrap_or(0),
                self.buffer.get(offset + 1).copied().unwrap_or(0),
                self.buffer.get(offset).copied().unwrap_or(0),
            ),
            _ => (0, 0, 0),
        };
        rgb565(r, g, b)
    }

    fn read_u16(&self, offset: usize) -> u16 {
        let lo = self.buffer.get(offset).copied().unwrap_or(0) as u16;
        let hi = self.buffer.get(offset + 1).copied().unwrap_or(0) as u16;
        lo | (hi << 8)
    }
}

pub(super) fn is_vga_mmio_region(gpa: u64) -> bool {
    (VGA_FB_BASE..VGA_FB_BASE + VGA_FB_SIZE).contains(&gpa)
}

#[cfg(not(target_os = "linux"))]
pub(super) fn handle_vga_exit(
    instance: &mut super::VmInstance,
    vcpu: &libavm::AvmVcpu,
    exit: &VmExitInfo,
) -> Result<bool, crate::errors::AsldError> {
    if let Some(action) = instance.display.io_action(exit) {
        finish_vga_action(instance, vcpu, exit, None, action, exit.instruction_len)?;
        return Ok(true);
    }
    if exit.reason != exit_reason::EPT_VIOLATION || !is_vga_mmio_region(exit.guest_phys_addr) {
        return Ok(false);
    }
    let prepared = super::mmio::prepare_mmio_exit(instance, vcpu, exit)?;
    let Some(action) = instance.display.mmio_action(&prepared.exit) else {
        return Ok(false);
    };
    finish_vga_action(
        instance,
        vcpu,
        &prepared.exit,
        Some(&prepared),
        action,
        prepared.instruction_len(),
    )?;
    Ok(true)
}

#[cfg(not(target_os = "linux"))]
fn finish_vga_action(
    instance: &mut super::VmInstance,
    vcpu: &libavm::AvmVcpu,
    exit: &VmExitInfo,
    prepared: Option<&super::mmio::PreparedMmioExit>,
    action: VgaAction,
    instruction_len: u32,
) -> Result<(), crate::errors::AsldError> {
    if let Some(value) = action.read_value {
        if let Some(prepared) = prepared {
            super::mmio::complete_mmio_read(vcpu, &prepared, value)?;
        } else {
            super::vcpu::write_io_read_value(vcpu, exit.access_size, value)?;
        }
    }
    super::vcpu::advance_guest_rip(vcpu, instruction_len)?;
    if action.publish {
        let (w, h, pixels) = instance.display.preview_rgb565();
        let _ =
            crate::broker::write_console_framebuffer_preview(&instance.distro_name, w, h, &pixels);
    }
    Ok(())
}

fn is_bga_port(port: u16) -> bool {
    port == BGA_INDEX_PORT || port == BGA_DATA_PORT
}

fn normalize_bpp(value: u16) -> u16 {
    match value {
        8 | 15 | 16 | 24 | 32 => value,
        _ => 32,
    }
}

fn mask_width(value: u32, access_size: u8) -> u32 {
    match access_size {
        1 => value & 0xff,
        2 => value & 0xffff,
        _ => value,
    }
}

fn rgb565(r: u8, g: u8, b: u8) -> u16 {
    (((r as u16) & 0xf8) << 8) | (((g as u16) & 0xfc) << 3) | ((b as u16) >> 3)
}

#[cfg(test)]
mod tests {
    use super::{GuestFramebuffer, BGA_DATA_PORT, BGA_INDEX_PORT, VGA_FB_BASE};
    use crate::vm::{exit_reason, VmExitInfo};

    fn outw(fb: &mut GuestFramebuffer, port: u16, value: u16) {
        let _ = fb.io_action(&VmExitInfo {
            reason: exit_reason::IO_INSTRUCTION,
            io_port: port,
            access_size: 2,
            io_data: value as u64,
            ..VmExitInfo::default()
        });
    }

    #[test]
    fn bga_mode_resize_tracks_guest_resolution() {
        let mut fb = GuestFramebuffer::default();
        outw(&mut fb, BGA_INDEX_PORT, 1);
        outw(&mut fb, BGA_DATA_PORT, 640);
        outw(&mut fb, BGA_INDEX_PORT, 2);
        outw(&mut fb, BGA_DATA_PORT, 480);
        outw(&mut fb, BGA_INDEX_PORT, 3);
        outw(&mut fb, BGA_DATA_PORT, 32);
        outw(&mut fb, BGA_INDEX_PORT, 4);
        outw(&mut fb, BGA_DATA_PORT, 1);
        assert_eq!(fb.buffer.len(), 640 * 480 * 4);
    }

    #[test]
    fn mmio_write_updates_preview_pixels() {
        let mut fb = GuestFramebuffer::default();
        let _ = fb.mmio_action(&VmExitInfo {
            reason: exit_reason::EPT_VIOLATION,
            guest_phys_addr: VGA_FB_BASE,
            access_size: 4,
            io_data: 0x00_33_66_99,
            ..VmExitInfo::default()
        });
        let (_, _, pixels) = fb.preview_rgb565();
        assert_eq!(pixels[0], 0x3333);
    }
}
