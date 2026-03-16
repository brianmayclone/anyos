use std::path::{Path, PathBuf};
use std::fs;

pub use libcorevm::setup::{GuestOs, GuestArch};
pub use libcorevm::devices::gpu::GpuModel;

#[derive(Clone, Debug, PartialEq)]
pub enum BootOrder { DiskFirst, CdFirst, FloppyFirst }

#[derive(Clone, Debug, PartialEq)]
pub enum BiosType { CoreVm, SeaBios }

#[derive(Clone, Debug, PartialEq)]
pub enum RamAlloc { Preallocate, OnDemand }

#[derive(Clone, Debug, PartialEq)]
pub enum NetMode {
    /// No host networking — packets silently dropped.
    Disconnected,
    /// User-mode NAT (built-in DHCP/DNS, no root required).
    UserMode,
    /// TAP device bridged to host interface (requires root/CAP_NET_ADMIN).
    Bridge,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MacMode { Dynamic, Static }

#[derive(Clone, Debug)]
pub struct VmConfig {
    pub uuid: String,
    pub name: String,
    pub guest_os: GuestOs,
    pub guest_arch: GuestArch,
    pub ram_mb: u32,
    pub cpu_cores: u32,
    pub disk_images: Vec<String>,
    pub iso_image: String,
    pub boot_order: BootOrder,
    pub bios_type: BiosType,
    pub gpu_model: GpuModel,
    pub vram_mb: u32,
    pub net_enabled: bool,
    pub net_mode: NetMode,
    pub net_host_nic: String,
    pub mac_mode: MacMode,
    pub mac_address: String,
    pub audio_enabled: bool,
    pub usb_tablet: bool,
    pub ram_alloc: RamAlloc,
    pub diagnostics: bool,
    /// Disk I/O cache size per disk in MiB (0 = disabled).
    pub disk_cache_mb: u32,
    /// Disk cache mode: "writeback", "writethrough", "none".
    pub disk_cache_mode: DiskCacheMode,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DiskCacheMode {
    WriteBack,
    WriteThrough,
    None,
}

impl VmConfig {
    /// First disk image path (for backwards compat / primary disk).
    pub fn primary_disk(&self) -> &str {
        self.disk_images.first().map(|s| s.as_str()).unwrap_or("")
    }
}

impl Default for VmConfig {
    fn default() -> Self {
        Self {
            uuid: uuid::Uuid::new_v4().to_string().replace("-", ""),
            name: "New VM".into(),
            guest_os: GuestOs::Other,
            guest_arch: GuestArch::X64,
            ram_mb: 256,
            cpu_cores: 1,
            disk_images: Vec::new(),
            iso_image: String::new(),
            boot_order: BootOrder::CdFirst,
            bios_type: BiosType::SeaBios,
            gpu_model: GpuModel::StdVga,
            vram_mb: 16,
            net_enabled: false,
            net_mode: NetMode::UserMode,
            net_host_nic: String::new(),
            mac_mode: MacMode::Dynamic,
            mac_address: String::new(),
            audio_enabled: true,
            usb_tablet: false,
            ram_alloc: RamAlloc::OnDemand,
            diagnostics: false,
            disk_cache_mb: 0,
            disk_cache_mode: DiskCacheMode::None,
        }
    }
}

impl VmConfig {
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        let path = dir.join(format!("{}.conf", self.uuid));
        let boot = match self.boot_order {
            BootOrder::DiskFirst => "disk",
            BootOrder::CdFirst => "cd",
            BootOrder::FloppyFirst => "floppy",
        };
        let bios = match self.bios_type {
            BiosType::CoreVm => "corevm",
            BiosType::SeaBios => "seabios",
        };
        let alloc = match self.ram_alloc {
            RamAlloc::Preallocate => "preallocate",
            RamAlloc::OnDemand => "ondemand",
        };
        let net_mode = match self.net_mode {
            NetMode::Disconnected => "disconnected",
            NetMode::UserMode => "usermode",
            NetMode::Bridge => "bridge",
        };
        let mac_mode = match self.mac_mode {
            MacMode::Dynamic => "dynamic",
            MacMode::Static => "static",
        };
        // Serialize disk_images: first as "disk=" for compat, additional as "disk2=", "disk3=" etc.
        let mut disk_lines = String::new();
        for (i, d) in self.disk_images.iter().enumerate() {
            if i == 0 {
                disk_lines.push_str(&format!("disk={}\n", d));
            } else {
                disk_lines.push_str(&format!("disk{}={}\n", i + 1, d));
            }
        }
        if self.disk_images.is_empty() {
            disk_lines.push_str("disk=\n");
        }
        let arch = match self.guest_arch {
            GuestArch::X86 => "x86",
            GuestArch::X64 => "x64",
        };
        let content = format!(
            "name={}\nguest_os={}\nguest_arch={}\nram={}\ncpu_cores={}\n{}iso={}\nboot={}\nbios={}\n\
             ram_alloc={}\ngpu={}\nvram_mb={}\nnet_enabled={}\nnet_mode={}\nnet_host_nic={}\n\
             mac_mode={}\nmac_address={}\naudio_enabled={}\nusb_tablet={}\ndiagnostics={}\n\
             disk_cache_mb={}\ndisk_cache_mode={}\n",
            self.name, self.guest_os.to_config_str(), arch,
            self.ram_mb, self.cpu_cores, disk_lines, self.iso_image,
            boot, bios,
            alloc,
            match self.gpu_model { GpuModel::StdVga => "stdvga" },
            self.vram_mb,
            if self.net_enabled { "1" } else { "0" },
            net_mode, self.net_host_nic, mac_mode, self.mac_address,
            if self.audio_enabled { "1" } else { "0" },
            if self.usb_tablet { "1" } else { "0" },
            if self.diagnostics { "1" } else { "0" },
            self.disk_cache_mb,
            match self.disk_cache_mode {
                DiskCacheMode::WriteBack => "writeback",
                DiskCacheMode::WriteThrough => "writethrough",
                DiskCacheMode::None => "none",
            },
        );
        fs::write(&path, content)
    }

    pub fn load(path: &Path) -> std::io::Result<Self> {
        let content = fs::read_to_string(path)?;
        let uuid = path.file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let mut cfg = VmConfig { uuid, ..Default::default() };

        for line in content.lines() {
            let Some((key, val)) = line.split_once('=') else { continue };
            match key.trim() {
                "name" => cfg.name = val.to_string(),
                "guest_os" => cfg.guest_os = GuestOs::from_config_str(val),
                "guest_arch" => cfg.guest_arch = match val {
                    "x86" => GuestArch::X86,
                    _ => GuestArch::X64,
                },
                "ram" => cfg.ram_mb = val.parse().unwrap_or(256),
                "cpu_cores" => cfg.cpu_cores = val.parse().unwrap_or(1),
                "disk" => {
                    // Primary disk — backwards compatible
                    if !val.is_empty() {
                        if cfg.disk_images.is_empty() {
                            cfg.disk_images.push(val.to_string());
                        } else {
                            cfg.disk_images[0] = val.to_string();
                        }
                    }
                }
                k if k.starts_with("disk") && k.len() > 4 => {
                    // disk2, disk3, ... — additional disks
                    if let Ok(idx) = k[4..].parse::<usize>() {
                        let pos = idx - 1; // disk2 -> index 1
                        if !val.is_empty() {
                            while cfg.disk_images.len() <= pos {
                                cfg.disk_images.push(String::new());
                            }
                            cfg.disk_images[pos] = val.to_string();
                        }
                    }
                }
                "iso" => cfg.iso_image = val.to_string(),
                "boot" => cfg.boot_order = match val {
                    "disk" => BootOrder::DiskFirst,
                    "floppy" => BootOrder::FloppyFirst,
                    _ => BootOrder::CdFirst,
                },
                "bios" => cfg.bios_type = match val {
                    "corevm" => BiosType::CoreVm,
                    _ => BiosType::SeaBios,
                },
                "jit" => { /* ignored, legacy field */ },
                "ram_alloc" => cfg.ram_alloc = match val {
                    "preallocate" => RamAlloc::Preallocate,
                    _ => RamAlloc::OnDemand,
                },
                "gpu" => cfg.gpu_model = match val {
                    _ => GpuModel::StdVga, // currently only one model
                },
                "vram_mb" => cfg.vram_mb = val.parse().unwrap_or(16),
                "net_enabled" => cfg.net_enabled = val == "1",
                "net_mode" => cfg.net_mode = match val {
                    "bridge" => NetMode::Bridge,
                    "disconnected" => NetMode::Disconnected,
                    "nat" | "usermode" | _ => NetMode::UserMode,
                },
                "net_host_nic" => cfg.net_host_nic = val.to_string(),
                "mac_mode" => cfg.mac_mode = match val {
                    "static" => MacMode::Static,
                    _ => MacMode::Dynamic,
                },
                "mac_address" => cfg.mac_address = val.to_string(),
                "audio_enabled" => cfg.audio_enabled = val == "1",
                "usb_tablet" => cfg.usb_tablet = val == "1",
                "diagnostics" => cfg.diagnostics = val == "1",
                "disk_cache_mb" => cfg.disk_cache_mb = val.parse().unwrap_or(32),
                "disk_cache_mode" => cfg.disk_cache_mode = match val {
                    "writethrough" => DiskCacheMode::WriteThrough,
                    "none" => DiskCacheMode::None,
                    _ => DiskCacheMode::WriteBack,
                },
                _ => {}
            }
        }
        // Remove trailing empty entries
        while cfg.disk_images.last().map_or(false, |s| s.is_empty()) {
            cfg.disk_images.pop();
        }
        Ok(cfg)
    }

    pub fn config_path(&self, dir: &Path) -> PathBuf {
        dir.join(format!("{}.conf", self.uuid))
    }
}
