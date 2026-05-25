use alloc::vec::Vec;

use crate::errors::AsldError;

use super::{exit_reason, VmExitInfo};

pub(super) const VIRTIO_GPU_MMIO_BASE: u64 = 0xfe00_0000;
pub(super) const VIRTIO_GPU_MMIO_SIZE: u64 = 0x1000;

const COMMON_OFFSET: u64 = 0x000;
const NOTIFY_OFFSET: u64 = 0x100;
const ISR_OFFSET: u64 = 0x200;
const DEVICE_OFFSET: u64 = 0x300;

const COMMON_DEVICE_FEATURE_SELECT: u64 = 0x00;
const COMMON_DEVICE_FEATURE: u64 = 0x04;
const COMMON_DRIVER_FEATURE_SELECT: u64 = 0x08;
const COMMON_DRIVER_FEATURE: u64 = 0x0c;
const COMMON_MSIX_CONFIG: u64 = 0x10;
const COMMON_NUM_QUEUES: u64 = 0x12;
const COMMON_DEVICE_STATUS: u64 = 0x14;
const COMMON_CONFIG_GENERATION: u64 = 0x15;
const COMMON_QUEUE_SELECT: u64 = 0x16;
const COMMON_QUEUE_SIZE: u64 = 0x18;
const COMMON_QUEUE_MSIX_VECTOR: u64 = 0x1a;
const COMMON_QUEUE_ENABLE: u64 = 0x1c;
const COMMON_QUEUE_NOTIFY_OFF: u64 = 0x1e;
const COMMON_QUEUE_DESC_LO: u64 = 0x20;
const COMMON_QUEUE_DESC_HI: u64 = 0x24;
const COMMON_QUEUE_AVAIL_LO: u64 = 0x28;
const COMMON_QUEUE_AVAIL_HI: u64 = 0x2c;
const COMMON_QUEUE_USED_LO: u64 = 0x30;
const COMMON_QUEUE_USED_HI: u64 = 0x34;

const VIRTIO_F_VERSION_1: u64 = 1 << 32;
const QUEUE_COUNT: usize = 2;
const QUEUE_SIZE: u16 = 64;
const VIRTIO_GPU_IRQ: u8 = 10;

const VIRTQ_DESC_F_NEXT: u16 = 1;
const VIRTQ_DESC_F_WRITE: u16 = 2;

const CMD_GET_DISPLAY_INFO: u32 = 0x0100;
const CMD_RESOURCE_CREATE_2D: u32 = 0x0101;
const CMD_RESOURCE_UNREF: u32 = 0x0102;
const CMD_SET_SCANOUT: u32 = 0x0103;
const CMD_RESOURCE_FLUSH: u32 = 0x0104;
const CMD_TRANSFER_TO_HOST_2D: u32 = 0x0105;
const CMD_RESOURCE_ATTACH_BACKING: u32 = 0x0106;
const CMD_RESOURCE_DETACH_BACKING: u32 = 0x0107;
const CMD_UPDATE_CURSOR: u32 = 0x0300;
const CMD_MOVE_CURSOR: u32 = 0x0301;

const RESP_OK_NODATA: u32 = 0x1100;
const RESP_OK_DISPLAY_INFO: u32 = 0x1101;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct VirtioGpuDevice {
    device_feature_select: u32,
    driver_feature_select: u32,
    driver_features: u64,
    status: u8,
    queue_select: u16,
    isr_status: u8,
    queues: [VirtQueueState; QUEUE_COUNT],
    resources: Vec<GpuResource>,
    scanout_resource_id: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct VirtQueueState {
    size: u16,
    enabled: bool,
    desc: u64,
    avail: u64,
    used: u64,
    last_avail_idx: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct GpuResource {
    id: u32,
    width: u32,
    height: u32,
    format: u32,
    backing: Vec<MemEntry>,
    pixels: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MemEntry {
    addr: u64,
    len: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Descriptor {
    addr: u64,
    len: u32,
    flags: u16,
    next: u16,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct VirtioAction {
    pub(super) read_value: Option<u32>,
    pub(super) interrupt: bool,
}

impl Default for VirtioGpuDevice {
    fn default() -> Self {
        Self {
            device_feature_select: 0,
            driver_feature_select: 0,
            driver_features: 0,
            status: 0,
            queue_select: 0,
            isr_status: 0,
            queues: [VirtQueueState {
                size: QUEUE_SIZE,
                enabled: false,
                desc: 0,
                avail: 0,
                used: 0,
                last_avail_idx: 0,
            }; QUEUE_COUNT],
            resources: Vec::new(),
            scanout_resource_id: 0,
        }
    }
}

impl VirtioGpuDevice {
    pub(super) fn mmio_action<R, W>(
        &mut self,
        exit: &VmExitInfo,
        distro_name: &str,
        mut read_guest: R,
        mut write_guest: W,
    ) -> Result<Option<VirtioAction>, AsldError>
    where
        R: FnMut(u64, &mut [u8]) -> Result<(), AsldError>,
        W: FnMut(u64, &[u8]) -> Result<(), AsldError>,
    {
        if exit.reason != exit_reason::EPT_VIOLATION
            || !is_virtio_gpu_mmio_region(exit.guest_phys_addr)
        {
            return Ok(None);
        }
        let offset = exit.guest_phys_addr.saturating_sub(VIRTIO_GPU_MMIO_BASE);
        if exit.is_read != 0 {
            return Ok(Some(VirtioAction {
                read_value: Some(mask_width(self.read_reg(offset), exit.access_size)),
                interrupt: false,
            }));
        }

        let mut interrupt = false;
        if (NOTIFY_OFFSET..ISR_OFFSET).contains(&offset) {
            let queue = exit.io_data as u16;
            interrupt =
                self.process_queue(queue, distro_name, &mut read_guest, &mut write_guest)?;
        } else {
            self.write_reg(offset, exit.io_data as u32);
        }
        Ok(Some(VirtioAction {
            read_value: None,
            interrupt,
        }))
    }

    fn read_reg(&mut self, offset: u64) -> u32 {
        if (COMMON_OFFSET..NOTIFY_OFFSET).contains(&offset) {
            return self.read_common(offset - COMMON_OFFSET);
        }
        if offset == ISR_OFFSET {
            let status = self.isr_status as u32;
            self.isr_status = 0;
            return status;
        }
        if (DEVICE_OFFSET..DEVICE_OFFSET + 0x10).contains(&offset) {
            return match offset - DEVICE_OFFSET {
                8 => 1,
                12 => 0,
                _ => 0,
            };
        }
        0
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        if (COMMON_OFFSET..NOTIFY_OFFSET).contains(&offset) {
            self.write_common(offset - COMMON_OFFSET, value);
        } else if (DEVICE_OFFSET..DEVICE_OFFSET + 0x10).contains(&offset) {
            let _ = value;
        }
    }

    fn read_common(&self, offset: u64) -> u32 {
        let queue = self.selected_queue();
        match offset {
            COMMON_DEVICE_FEATURE_SELECT => self.device_feature_select,
            COMMON_DEVICE_FEATURE => {
                if self.device_feature_select == 1 {
                    (VIRTIO_F_VERSION_1 >> 32) as u32
                } else {
                    0
                }
            }
            COMMON_DRIVER_FEATURE_SELECT => self.driver_feature_select,
            COMMON_DRIVER_FEATURE => {
                if self.driver_feature_select == 1 {
                    (self.driver_features >> 32) as u32
                } else {
                    self.driver_features as u32
                }
            }
            COMMON_MSIX_CONFIG => 0xffff,
            COMMON_NUM_QUEUES => QUEUE_COUNT as u32,
            COMMON_DEVICE_STATUS => self.status as u32,
            COMMON_CONFIG_GENERATION => 0,
            COMMON_QUEUE_SELECT => self.queue_select as u32,
            COMMON_QUEUE_SIZE => queue.map(|q| q.size as u32).unwrap_or(0),
            COMMON_QUEUE_MSIX_VECTOR => 0xffff,
            COMMON_QUEUE_ENABLE => queue.map(|q| q.enabled as u32).unwrap_or(0),
            COMMON_QUEUE_NOTIFY_OFF => self.queue_select as u32,
            COMMON_QUEUE_DESC_LO => queue.map(|q| q.desc as u32).unwrap_or(0),
            COMMON_QUEUE_DESC_HI => queue.map(|q| (q.desc >> 32) as u32).unwrap_or(0),
            COMMON_QUEUE_AVAIL_LO => queue.map(|q| q.avail as u32).unwrap_or(0),
            COMMON_QUEUE_AVAIL_HI => queue.map(|q| (q.avail >> 32) as u32).unwrap_or(0),
            COMMON_QUEUE_USED_LO => queue.map(|q| q.used as u32).unwrap_or(0),
            COMMON_QUEUE_USED_HI => queue.map(|q| (q.used >> 32) as u32).unwrap_or(0),
            _ => 0,
        }
    }

    fn write_common(&mut self, offset: u64, value: u32) {
        match offset {
            COMMON_DEVICE_FEATURE_SELECT => self.device_feature_select = value,
            COMMON_DRIVER_FEATURE_SELECT => self.driver_feature_select = value,
            COMMON_DRIVER_FEATURE => {
                if self.driver_feature_select == 1 {
                    self.driver_features =
                        (self.driver_features & 0xffff_ffff) | ((value as u64) << 32);
                } else {
                    self.driver_features = (self.driver_features & !0xffff_ffff) | value as u64;
                }
            }
            COMMON_DEVICE_STATUS => {
                self.status = value as u8;
                if self.status == 0 {
                    self.driver_features = 0;
                    for queue in &mut self.queues {
                        queue.enabled = false;
                        queue.last_avail_idx = 0;
                    }
                }
            }
            COMMON_QUEUE_SELECT => self.queue_select = value as u16,
            COMMON_QUEUE_SIZE => {
                if let Some(queue) = self.selected_queue_mut() {
                    queue.size = (value as u16).clamp(1, QUEUE_SIZE);
                }
            }
            COMMON_QUEUE_ENABLE => {
                if let Some(queue) = self.selected_queue_mut() {
                    queue.enabled = value != 0;
                }
            }
            COMMON_QUEUE_DESC_LO => {
                if let Some(queue) = self.selected_queue_mut() {
                    queue.desc = (queue.desc & !0xffff_ffff) | value as u64;
                }
            }
            COMMON_QUEUE_DESC_HI => {
                if let Some(queue) = self.selected_queue_mut() {
                    queue.desc = (queue.desc & 0xffff_ffff) | ((value as u64) << 32);
                }
            }
            COMMON_QUEUE_AVAIL_LO => {
                if let Some(queue) = self.selected_queue_mut() {
                    queue.avail = (queue.avail & !0xffff_ffff) | value as u64;
                }
            }
            COMMON_QUEUE_AVAIL_HI => {
                if let Some(queue) = self.selected_queue_mut() {
                    queue.avail = (queue.avail & 0xffff_ffff) | ((value as u64) << 32);
                }
            }
            COMMON_QUEUE_USED_LO => {
                if let Some(queue) = self.selected_queue_mut() {
                    queue.used = (queue.used & !0xffff_ffff) | value as u64;
                }
            }
            COMMON_QUEUE_USED_HI => {
                if let Some(queue) = self.selected_queue_mut() {
                    queue.used = (queue.used & 0xffff_ffff) | ((value as u64) << 32);
                }
            }
            _ => {}
        }
    }

    fn process_queue<R, W>(
        &mut self,
        queue_index: u16,
        distro_name: &str,
        read_guest: &mut R,
        write_guest: &mut W,
    ) -> Result<bool, AsldError>
    where
        R: FnMut(u64, &mut [u8]) -> Result<(), AsldError>,
        W: FnMut(u64, &[u8]) -> Result<(), AsldError>,
    {
        let Some(queue) = self.queues.get(queue_index as usize).copied() else {
            return Ok(false);
        };
        if !queue.enabled || queue.desc == 0 || queue.avail == 0 || queue.used == 0 {
            return Ok(false);
        }

        let avail_idx = read_guest_u16(read_guest, queue.avail + 2)?;
        let mut processed = false;
        while self.queues[queue_index as usize].last_avail_idx != avail_idx {
            let last = self.queues[queue_index as usize].last_avail_idx;
            let ring = queue.avail + 4 + ((last % queue.size) as u64 * 2);
            let head = read_guest_u16(read_guest, ring)?;
            let response_len =
                self.process_descriptor_chain(queue, head, distro_name, read_guest, write_guest)?;
            self.write_used_entry(
                queue_index,
                queue,
                head,
                response_len,
                read_guest,
                write_guest,
            )?;
            self.queues[queue_index as usize].last_avail_idx = last.wrapping_add(1);
            processed = true;
        }
        if processed {
            self.isr_status |= 1;
        }
        Ok(processed)
    }

    fn process_descriptor_chain<R, W>(
        &mut self,
        queue: VirtQueueState,
        head: u16,
        distro_name: &str,
        read_guest: &mut R,
        write_guest: &mut W,
    ) -> Result<u32, AsldError>
    where
        R: FnMut(u64, &mut [u8]) -> Result<(), AsldError>,
        W: FnMut(u64, &[u8]) -> Result<(), AsldError>,
    {
        let descs = read_descriptor_chain(queue, head, read_guest)?;
        let mut command = Vec::new();
        let mut response_desc = None;
        for desc in &descs {
            if desc.flags & VIRTQ_DESC_F_WRITE != 0 {
                if response_desc.is_none() {
                    response_desc = Some(*desc);
                }
                continue;
            }
            let old_len = command.len();
            command.resize(old_len.saturating_add(desc.len as usize).min(4096), 0);
            read_guest(desc.addr, &mut command[old_len..])?;
        }
        let response = self.handle_gpu_command(&command, distro_name, read_guest)?;
        if let Some(desc) = response_desc {
            let len = response.len().min(desc.len as usize);
            write_guest(desc.addr, &response[..len])?;
            Ok(len as u32)
        } else {
            let _ = write_guest;
            Ok(0)
        }
    }

    fn handle_gpu_command<R>(
        &mut self,
        command: &[u8],
        distro_name: &str,
        read_guest: &mut R,
    ) -> Result<Vec<u8>, AsldError>
    where
        R: FnMut(u64, &mut [u8]) -> Result<(), AsldError>,
    {
        let cmd = read_u32(command, 0);
        match cmd {
            CMD_GET_DISPLAY_INFO => Ok(display_info_response()),
            CMD_RESOURCE_CREATE_2D => {
                let id = read_u32(command, 24);
                let format = read_u32(command, 28);
                let width = read_u32(command, 32).clamp(1, 4096);
                let height = read_u32(command, 36).clamp(1, 4096);
                self.upsert_resource(id, width, height, format);
                Ok(status_response(RESP_OK_NODATA))
            }
            CMD_RESOURCE_ATTACH_BACKING => {
                let id = read_u32(command, 24);
                let nr = read_u32(command, 28).min(256);
                let mut entries = Vec::new();
                for index in 0..nr as usize {
                    let off = 32 + index * 16;
                    if off + 16 > command.len() {
                        break;
                    }
                    entries.push(MemEntry {
                        addr: read_u64(command, off),
                        len: read_u32(command, off + 8),
                    });
                }
                if let Some(resource) = self.resource_mut(id) {
                    resource.backing = entries;
                }
                Ok(status_response(RESP_OK_NODATA))
            }
            CMD_SET_SCANOUT => {
                self.scanout_resource_id = read_u32(command, 44);
                Ok(status_response(RESP_OK_NODATA))
            }
            CMD_TRANSFER_TO_HOST_2D => {
                self.transfer_to_host_2d(command, read_guest)?;
                Ok(status_response(RESP_OK_NODATA))
            }
            CMD_RESOURCE_FLUSH => {
                let id = read_u32(command, 40);
                self.publish_resource(distro_name, id);
                Ok(status_response(RESP_OK_NODATA))
            }
            CMD_RESOURCE_UNREF | CMD_RESOURCE_DETACH_BACKING => {
                let id = read_u32(command, 24);
                self.resources.retain(|resource| resource.id != id);
                Ok(status_response(RESP_OK_NODATA))
            }
            CMD_UPDATE_CURSOR | CMD_MOVE_CURSOR => Ok(status_response(RESP_OK_NODATA)),
            _ => Ok(status_response(RESP_OK_NODATA)),
        }
    }

    fn transfer_to_host_2d<R>(
        &mut self,
        command: &[u8],
        read_guest: &mut R,
    ) -> Result<(), AsldError>
    where
        R: FnMut(u64, &mut [u8]) -> Result<(), AsldError>,
    {
        let x = read_u32(command, 24);
        let y = read_u32(command, 28);
        let width = read_u32(command, 32);
        let height = read_u32(command, 36);
        let offset = read_u64(command, 40);
        let id = read_u32(command, 48);
        let Some(resource) = self.resource_mut(id) else {
            return Ok(());
        };
        let bpp = 4usize;
        let row_bytes = (width as usize).saturating_mul(bpp);
        let mut row = alloc::vec![0u8; row_bytes];
        for row_index in 0..height as usize {
            row.fill(0);
            let src = offset.saturating_add((row_index * row_bytes) as u64);
            read_backing(&resource.backing, src, &mut row, read_guest)?;
            let dst_y = y as usize + row_index;
            let dst_x = x as usize;
            if dst_y >= resource.height as usize || dst_x >= resource.width as usize {
                continue;
            }
            let max = (resource.width as usize - dst_x).saturating_mul(bpp);
            let copy_len = row.len().min(max);
            let dst = (dst_y * resource.width as usize + dst_x).saturating_mul(bpp);
            if dst + copy_len <= resource.pixels.len() {
                resource.pixels[dst..dst + copy_len].copy_from_slice(&row[..copy_len]);
            }
        }
        Ok(())
    }

    fn publish_resource(&self, distro_name: &str, id: u32) {
        if id == 0 || id != self.scanout_resource_id {
            return;
        }
        let Some(resource) = self.resources.iter().find(|resource| resource.id == id) else {
            return;
        };
        let (w, h, pixels) = resource.preview_rgb565();
        let _ = crate::broker::write_console_framebuffer_preview(distro_name, w, h, &pixels);
    }

    fn upsert_resource(&mut self, id: u32, width: u32, height: u32, format: u32) {
        if let Some(resource) = self.resource_mut(id) {
            resource.width = width;
            resource.height = height;
            resource.format = format;
            resource
                .pixels
                .resize(width as usize * height as usize * 4, 0);
            return;
        }
        self.resources.push(GpuResource {
            id,
            width,
            height,
            format,
            backing: Vec::new(),
            pixels: alloc::vec![0; width as usize * height as usize * 4],
        });
    }

    fn resource_mut(&mut self, id: u32) -> Option<&mut GpuResource> {
        self.resources.iter_mut().find(|resource| resource.id == id)
    }

    fn selected_queue(&self) -> Option<&VirtQueueState> {
        self.queues.get(self.queue_select as usize)
    }

    fn selected_queue_mut(&mut self) -> Option<&mut VirtQueueState> {
        self.queues.get_mut(self.queue_select as usize)
    }

    fn write_used_entry<R, W>(
        &mut self,
        queue_index: u16,
        queue: VirtQueueState,
        head: u16,
        len: u32,
        read_guest: &mut R,
        write_guest: &mut W,
    ) -> Result<(), AsldError>
    where
        R: FnMut(u64, &mut [u8]) -> Result<(), AsldError>,
        W: FnMut(u64, &[u8]) -> Result<(), AsldError>,
    {
        let used_idx = read_guest_u16(read_guest, queue.used + 2)?;
        let slot = used_idx % queue.size;
        write_guest_u32(write_guest, queue.used + 4 + slot as u64 * 8, head as u32)?;
        write_guest_u32(write_guest, queue.used + 8 + slot as u64 * 8, len)?;
        write_guest_u16(write_guest, queue.used + 2, used_idx.wrapping_add(1))?;
        Ok(())
    }
}

impl GpuResource {
    fn preview_rgb565(&self) -> (u16, u16, Vec<u16>) {
        let sw = self.width.max(1) as usize;
        let sh = self.height.max(1) as usize;
        let pw = sw.min(40).max(1);
        let ph = sh.min(30).max(1);
        let mut out = Vec::with_capacity(pw * ph);
        for y in 0..ph {
            let sy = y * sh / ph;
            for x in 0..pw {
                let sx = x * sw / pw;
                let off = (sy * sw + sx) * 4;
                let b = self.pixels.get(off).copied().unwrap_or(0);
                let g = self.pixels.get(off + 1).copied().unwrap_or(0);
                let r = self.pixels.get(off + 2).copied().unwrap_or(0);
                out.push(rgb565(r, g, b));
            }
        }
        (pw as u16, ph as u16, out)
    }
}

#[cfg(not(target_os = "linux"))]
pub(super) fn handle_virtio_gpu_exit(
    instance: &mut super::VmInstance,
    vcpu: &libavm::AvmVcpu,
    exit: &VmExitInfo,
) -> Result<bool, AsldError> {
    if exit.reason != exit_reason::EPT_VIOLATION || !is_virtio_gpu_mmio_region(exit.guest_phys_addr)
    {
        return Ok(false);
    }
    let prepared = super::mmio::prepare_mmio_exit(instance, vcpu, exit)?;
    let memory_addr = instance.guest_memory_addr;
    let memory_size = instance.guest_memory_size;
    let Some(action) = instance.virtio_gpu.mmio_action(
        &prepared.exit,
        &instance.distro_name,
        |gpa, dest| {
            memory_result(super::memory::read_guest_physical(
                memory_addr,
                memory_size,
                gpa,
                dest,
            ))
        },
        |gpa, bytes| {
            memory_result(super::memory::write_guest_physical(
                memory_addr,
                memory_size,
                gpa,
                bytes,
            ))
        },
    )?
    else {
        return Ok(false);
    };
    if let Some(value) = action.read_value {
        super::mmio::complete_mmio_read(vcpu, &prepared, value)?;
    }
    super::vcpu::advance_guest_rip(vcpu, prepared.instruction_len())?;
    if action.interrupt {
        let _ = super::inject_device_irq(instance, vcpu, VIRTIO_GPU_IRQ)?;
    }
    Ok(true)
}

pub(super) fn is_virtio_gpu_mmio_region(gpa: u64) -> bool {
    (VIRTIO_GPU_MMIO_BASE..VIRTIO_GPU_MMIO_BASE + VIRTIO_GPU_MMIO_SIZE).contains(&gpa)
}

#[cfg(not(target_os = "linux"))]
fn memory_result(ok: bool) -> Result<(), AsldError> {
    if ok {
        Ok(())
    } else {
        Err(AsldError::BackendUnavailable("virtio guest memory access"))
    }
}

fn read_descriptor_chain<R>(
    queue: VirtQueueState,
    head: u16,
    read_guest: &mut R,
) -> Result<Vec<Descriptor>, AsldError>
where
    R: FnMut(u64, &mut [u8]) -> Result<(), AsldError>,
{
    let mut out = Vec::new();
    let mut index = head;
    for _ in 0..queue.size {
        let desc = read_descriptor(read_guest, queue.desc + index as u64 * 16)?;
        out.push(desc);
        if desc.flags & VIRTQ_DESC_F_NEXT == 0 {
            break;
        }
        index = desc.next;
    }
    Ok(out)
}

fn read_descriptor<R>(read_guest: &mut R, addr: u64) -> Result<Descriptor, AsldError>
where
    R: FnMut(u64, &mut [u8]) -> Result<(), AsldError>,
{
    let mut bytes = [0u8; 16];
    read_guest(addr, &mut bytes)?;
    Ok(Descriptor {
        addr: read_u64(&bytes, 0),
        len: read_u32(&bytes, 8),
        flags: read_u16(&bytes, 12),
        next: read_u16(&bytes, 14),
    })
}

fn read_backing<R>(
    entries: &[MemEntry],
    offset: u64,
    dest: &mut [u8],
    read_guest: &mut R,
) -> Result<(), AsldError>
where
    R: FnMut(u64, &mut [u8]) -> Result<(), AsldError>,
{
    let mut remaining_offset = offset;
    let mut written = 0usize;
    for entry in entries {
        if remaining_offset >= entry.len as u64 {
            remaining_offset -= entry.len as u64;
            continue;
        }
        let entry_off = remaining_offset as usize;
        let available = entry.len as usize - entry_off;
        let count = available.min(dest.len().saturating_sub(written));
        read_guest(
            entry.addr + entry_off as u64,
            &mut dest[written..written + count],
        )?;
        written += count;
        remaining_offset = 0;
        if written == dest.len() {
            break;
        }
    }
    Ok(())
}

fn display_info_response() -> Vec<u8> {
    let mut out = alloc::vec![0u8; 24 + 16 * 24];
    write_u32_slice(&mut out, 0, RESP_OK_DISPLAY_INFO);
    write_u32_slice(&mut out, 24 + 8, 1024);
    write_u32_slice(&mut out, 24 + 12, 768);
    write_u32_slice(&mut out, 24 + 16, 1);
    out
}

fn status_response(kind: u32) -> Vec<u8> {
    let mut out = alloc::vec![0u8; 24];
    write_u32_slice(&mut out, 0, kind);
    out
}

fn read_guest_u16<R>(read_guest: &mut R, addr: u64) -> Result<u16, AsldError>
where
    R: FnMut(u64, &mut [u8]) -> Result<(), AsldError>,
{
    let mut bytes = [0u8; 2];
    read_guest(addr, &mut bytes)?;
    Ok(u16::from_le_bytes(bytes))
}

fn write_guest_u16<W>(write_guest: &mut W, addr: u64, value: u16) -> Result<(), AsldError>
where
    W: FnMut(u64, &[u8]) -> Result<(), AsldError>,
{
    write_guest(addr, &value.to_le_bytes())
}

fn write_guest_u32<W>(write_guest: &mut W, addr: u64, value: u32) -> Result<(), AsldError>
where
    W: FnMut(u64, &[u8]) -> Result<(), AsldError>,
{
    write_guest(addr, &value.to_le_bytes())
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([
        bytes.get(offset).copied().unwrap_or(0),
        bytes.get(offset + 1).copied().unwrap_or(0),
    ])
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes.get(offset).copied().unwrap_or(0),
        bytes.get(offset + 1).copied().unwrap_or(0),
        bytes.get(offset + 2).copied().unwrap_or(0),
        bytes.get(offset + 3).copied().unwrap_or(0),
    ])
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        bytes.get(offset).copied().unwrap_or(0),
        bytes.get(offset + 1).copied().unwrap_or(0),
        bytes.get(offset + 2).copied().unwrap_or(0),
        bytes.get(offset + 3).copied().unwrap_or(0),
        bytes.get(offset + 4).copied().unwrap_or(0),
        bytes.get(offset + 5).copied().unwrap_or(0),
        bytes.get(offset + 6).copied().unwrap_or(0),
        bytes.get(offset + 7).copied().unwrap_or(0),
    ])
}

fn write_u32_slice(bytes: &mut [u8], offset: usize, value: u32) {
    if offset + 4 <= bytes.len() {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
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
    use super::{VirtioGpuDevice, VIRTIO_GPU_MMIO_BASE};
    use crate::vm::{exit_reason, VmExitInfo};

    #[test]
    fn exposes_modern_version_feature() {
        let mut dev = VirtioGpuDevice::default();
        let _ = dev.mmio_action(
            &VmExitInfo {
                reason: exit_reason::EPT_VIOLATION,
                guest_phys_addr: VIRTIO_GPU_MMIO_BASE,
                access_size: 4,
                io_data: 1,
                ..VmExitInfo::default()
            },
            "test",
            |_gpa, _dest| Ok(()),
            |_gpa, _src| Ok(()),
        );
        let value = dev
            .mmio_action(
                &VmExitInfo {
                    reason: exit_reason::EPT_VIOLATION,
                    guest_phys_addr: VIRTIO_GPU_MMIO_BASE + 4,
                    access_size: 4,
                    is_read: 1,
                    ..VmExitInfo::default()
                },
                "test",
                |_gpa, _dest| Ok(()),
                |_gpa, _src| Ok(()),
            )
            .unwrap()
            .unwrap()
            .read_value
            .unwrap();
        assert_eq!(value, 1);
    }
}
