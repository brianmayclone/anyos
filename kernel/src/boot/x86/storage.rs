//! Storage discovery, mounts, and late boot handoff into userspace.

use crate::arch;
use crate::boot::{NOGUI, SETUP_MODE};
use crate::boot_info::BootInfo;
use crate::drivers;
use crate::fs;
use crate::ipc;
use crate::net;
use crate::serial_println;
use crate::task;
use core::sync::atomic::Ordering;

pub(super) fn init_storage_and_userspace(
    boot_info: &BootInfo,
    acpi_info: Option<arch::x86::acpi::AcpiInfo>,
) -> ! {
    detect_and_register_root_partition();

    fs::blockcache::init();
    serial_println!("  Block cache initialized (8 MiB, 16384 sectors)");
    fs::vfs::init();
    fs::vfs::mount("/", fs::vfs::FsType::Fat, 0);
    fs::vfs::mount_devfs();
    maybe_mount_cdrom_root();

    drivers::kdrv::probe_external_drivers();
    task::users::init();
    net::load_config_files();
    drivers::input::mouse::init();
    drivers::input::vmmouse::init();

    ipc::event_bus::system_emit(ipc::event_bus::EventData::new(
        ipc::event_bus::EVT_BOOT_COMPLETE,
        0,
        0,
        0,
        0,
    ));

    start_application_processors(acpi_info.as_ref());

    if !NOGUI.load(Ordering::Relaxed) {
        load_boot_dlls();
    }

    super::userspace::start_userspace(boot_info, NOGUI.load(Ordering::Relaxed))
}

fn detect_and_register_root_partition() {
    use drivers::storage::blockdev;
    use fs::partition::PartitionType;

    let (disk_sectors, disk_label) = {
        let ahci_sectors = drivers::storage::ahci::disk_total_sectors();
        if ahci_sectors > 0 {
            (ahci_sectors, drivers::storage::ahci::disk_model())
        } else {
            (drivers::storage::ata::disk_total_sectors(), drivers::storage::ata::disk_model())
        }
    };

    blockdev::register_device(blockdev::BlockDevice {
        id: 0,
        disk_id: 0,
        partition: None,
        part_type: PartitionType::Empty,
        start_lba: 0,
        size_sectors: disk_sectors,
        label: disk_label,
    });
    blockdev::scan_and_register_partitions(0);

    let devices = blockdev::list_devices();
    let mut found_root_lba = false;
    for dev in &devices {
        if dev.disk_id == 0 && dev.partition.is_some() {
            serial_println!(
                "  Partition hd0p{}: start_lba={} type={}",
                dev.partition.unwrap() + 1,
                dev.start_lba,
                dev.part_type.label()
            );
            if !found_root_lba && dev.part_type != PartitionType::GptEsp {
                fs::vfs::set_root_partition_lba(dev.start_lba as u32);
                found_root_lba = true;
                serial_println!(
                    "  Root partition: hd0p{} (LBA {})",
                    dev.partition.unwrap() + 1,
                    dev.start_lba
                );
            }
        }
    }

    if !found_root_lba {
        serial_println!("  No partition table found, using default LBA 8192");
    }
}

fn maybe_mount_cdrom_root() {
    let ide_cd =
        drivers::storage::atapi::is_present() && drivers::storage::atapi::capacity_lba() > 0;
    let ahci_cd = drivers::storage::ahci::atapi_is_present();
    serial_println!("  CD-ROM: IDE={} AHCI={}", ide_cd, ahci_cd);

    if !(ide_cd || ahci_cd) {
        return;
    }

    if fs::vfs::has_root_fs() {
        fs::vfs::mount("/mnt/cdrom0", fs::vfs::FsType::Iso9660, 0);
        return;
    }

    serial_println!("  No disk filesystem detected, using ISO 9660 as root filesystem");
    fs::vfs::remove_mount("/");
    fs::vfs::mount("/", fs::vfs::FsType::Iso9660, 0);
    fs::vfs::enable_overlay();
    SETUP_MODE.store(true, Ordering::Relaxed);
    crate::drivers::serial::set_verbose(true);
    serial_println!("  CD-ROM boot detected - entering setup mode (verbose enabled)");
}

fn start_application_processors(acpi_info: Option<&arch::x86::acpi::AcpiInfo>) {
    if let Some(info) = acpi_info {
        if info.processors.len() > 1 {
            arch::x86::smp::start_aps(&info.processors);
        }
    }
}

fn load_boot_dlls() {
    const DLLS: [(&str, u64); 4] = [
        ("/Libraries/uisys.dlib", 0x0400_0000u64),
        ("/Libraries/libimage.dlib", 0x0410_0000u64),
        ("/Libraries/librender.dlib", 0x0430_0000u64),
        ("/Libraries/libcompositor.dlib", 0x0438_0000u64),
    ];

    for (path, base) in DLLS {
        let name = path.rsplit('/').next().unwrap_or(path);
        match task::dll::load_dll(path, base) {
            Ok(pages) => serial_println!("[OK] {}: {} pages", name, pages),
            Err(err) => serial_println!("[WARN] {} not loaded: {}", name, err),
        }
    }
}
