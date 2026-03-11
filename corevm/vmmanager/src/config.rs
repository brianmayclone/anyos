use std::path::{Path, PathBuf};
use std::fs;

#[derive(Clone, Debug, PartialEq)]
pub enum BootOrder { DiskFirst, CdFirst, FloppyFirst }

#[derive(Clone, Debug, PartialEq)]
pub enum BiosType { CoreVm, SeaBios }

#[derive(Clone, Debug, PartialEq)]
pub enum RamAlloc { Preallocate, OnDemand }

#[derive(Clone, Debug, PartialEq)]
pub enum NetMode { Nat, Bridge }

#[derive(Clone, Debug, PartialEq)]
pub enum MacMode { Dynamic, Static }

#[derive(Clone, Debug)]
pub struct VmConfig {
    pub uuid: String,
    pub name: String,
    pub ram_mb: u32,
    pub cpu_cores: u32,
    pub disk_image: String,
    pub iso_image: String,
    pub boot_order: BootOrder,
    pub bios_type: BiosType,
    pub gpu_type: String,
    pub net_enabled: bool,
    pub net_mode: NetMode,
    pub net_host_nic: String,
    pub mac_mode: MacMode,
    pub mac_address: String,
    pub ram_alloc: RamAlloc,
}

impl Default for VmConfig {
    fn default() -> Self {
        Self {
            uuid: uuid::Uuid::new_v4().to_string().replace("-", ""),
            name: "New VM".into(),
            ram_mb: 256,
            cpu_cores: 1,
            disk_image: String::new(),
            iso_image: String::new(),
            boot_order: BootOrder::CdFirst,
            bios_type: BiosType::SeaBios,
            gpu_type: "svga".into(),
            net_enabled: false,
            net_mode: NetMode::Nat,
            net_host_nic: String::new(),
            mac_mode: MacMode::Dynamic,
            mac_address: String::new(),
            ram_alloc: RamAlloc::OnDemand,
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
            NetMode::Nat => "nat",
            NetMode::Bridge => "bridge",
        };
        let mac_mode = match self.mac_mode {
            MacMode::Dynamic => "dynamic",
            MacMode::Static => "static",
        };
        let content = format!(
            "name={}\nram={}\ncpu_cores={}\ndisk={}\niso={}\nboot={}\nbios={}\n\
             ram_alloc={}\ngpu={}\nnet_enabled={}\nnet_mode={}\nnet_host_nic={}\n\
             mac_mode={}\nmac_address={}\n",
            self.name, self.ram_mb, self.cpu_cores, self.disk_image, self.iso_image,
            boot, bios, alloc, self.gpu_type,
            if self.net_enabled { "1" } else { "0" },
            net_mode, self.net_host_nic, mac_mode, self.mac_address,
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
                "ram" => cfg.ram_mb = val.parse().unwrap_or(256),
                "cpu_cores" => cfg.cpu_cores = val.parse().unwrap_or(1),
                "disk" => cfg.disk_image = val.to_string(),
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
                "jit" => { /* ignored — hardware virtualization */ },
                "ram_alloc" => cfg.ram_alloc = match val {
                    "preallocate" => RamAlloc::Preallocate,
                    _ => RamAlloc::OnDemand,
                },
                "gpu" => cfg.gpu_type = val.to_string(),
                "net_enabled" => cfg.net_enabled = val == "1",
                "net_mode" => cfg.net_mode = match val {
                    "bridge" => NetMode::Bridge,
                    _ => NetMode::Nat,
                },
                "net_host_nic" => cfg.net_host_nic = val.to_string(),
                "mac_mode" => cfg.mac_mode = match val {
                    "static" => MacMode::Static,
                    _ => MacMode::Dynamic,
                },
                "mac_address" => cfg.mac_address = val.to_string(),
                _ => {}
            }
        }
        Ok(cfg)
    }

    pub fn config_path(&self, dir: &Path) -> PathBuf {
        dir.join(format!("{}.conf", self.uuid))
    }
}
