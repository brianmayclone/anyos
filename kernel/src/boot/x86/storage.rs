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
    // Dual-partition layouts add `/boot` as a second sub-mount so that
    // Stage 2 and the early kernel can continue to read the (small) boot
    // filesystem while user-facing paths (/System, /Applications, /Users)
    // all live on the larger root partition.
    fs::vfs::mount_boot_if_present();
    fs::vfs::mount_devfs();
    maybe_mount_cdrom_root();
    try_mount_corefs_partitions();

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

    // Collect all data-carrying partitions on disk 0 in MBR order.
    //
    // Criteria:
    //  - Partition slot assigned (skips the whole-disk pseudo-entry).
    //  - Not a GPT EFI System Partition (ESP is bootloader-only).
    //  - Not CoreFS — CoreFS partitions are mounted separately under
    //    /mnt/corefs* (or /System with the opt-in flag from Phase 1),
    //    not used as the classic Root-FS.
    //
    // The sort key is the partition-table index so we always see the
    // partitions in the same order the MBR lists them, regardless of
    // registration order.
    let mut data_parts: alloc::vec::Vec<&blockdev::BlockDevice> = devices
        .iter()
        .filter(|d| d.disk_id == 0 && d.partition.is_some())
        .filter(|d| {
            d.part_type != PartitionType::GptEsp && d.part_type != PartitionType::CoreFs
        })
        .collect();
    data_parts.sort_by_key(|d| d.partition.unwrap_or(0));

    for dev in &devices {
        if dev.disk_id == 0 && dev.partition.is_some() {
            serial_println!(
                "  Partition sda{}: start_lba={} type={}",
                dev.partition.unwrap() + 1,
                dev.start_lba,
                dev.part_type.label()
            );
        }
    }

    // Policy for picking `/` vs `/boot`:
    //
    //  - 0 data partitions: no MBR, fall back to DEFAULT_PARTITION_LBA
    //    (setup mode / legacy CD-booted images).
    //  - 1 data partition: classic single-partition layout — this is
    //    `/`, there is no separate `/boot` mount.
    //  - 2+ data partitions (dual-partition layout, Phase 5): the first
    //    partition is the boot filesystem (kernel assets, boot.cfg,
    //    logo/font) and gets mounted under `/boot`; the second is the
    //    root filesystem (/System, /Applications, /Users, …).  Any
    //    extra data partitions beyond the first two are ignored here
    //    and can still be mounted manually.
    match data_parts.len() {
        0 => {
            serial_println!("  No partition table found, using default LBA 8192");
        }
        1 => {
            let root = data_parts[0];
            fs::vfs::set_root_partition_lba(root.start_lba as u32);
            fs::vfs::set_root_blockdev_id(root.id);
            serial_println!(
                "  Root partition: sda{} (LBA {}, blockdev id={}) — single-partition layout",
                root.partition.unwrap() + 1,
                root.start_lba,
                root.id
            );
        }
        _ => {
            let boot = data_parts[0];
            let root = data_parts[1];
            fs::vfs::set_root_partition_lba(root.start_lba as u32);
            fs::vfs::set_root_blockdev_id(root.id);
            fs::vfs::set_boot_blockdev_id(boot.id);
            serial_println!(
                "  Root partition: sda{} (LBA {}, blockdev id={})",
                root.partition.unwrap() + 1,
                root.start_lba,
                root.id
            );
            serial_println!(
                "  Boot partition: sda{} (LBA {}, blockdev id={}) — dual-partition layout",
                boot.partition.unwrap() + 1,
                boot.start_lba,
                boot.id
            );
        }
    }
}

fn try_mount_corefs_partitions() {
    use drivers::storage::blockdev;

    let devices = blockdev::list_devices();
    let root_disk_id: u8 = 0; // Root-FS lebt immer auf disk 0 (erste BIOS-Disk).
    let mut mount_index: u32 = 0;
    let mut system_mounted = false;
    let want_system_mount = fs::corefs::mount_as_system_enabled();

    for dev in &devices {
        // Skip the "whole-disk" pseudo-entry (partition == None) and any
        // entry that obviously can't host a filesystem (size 0).
        if dev.partition.is_none() || dev.size_sectors == 0 {
            continue;
        }
        // Ersten CoreFS-Hit auf der Root-Disk optional nach /System mounten,
        // damit `/System/bin/*`, `/System/lib/*` usw. aus CoreFS bedient
        // werden können. Alle weiteren CoreFS-Partitionen landen wie gehabt
        // unter /mnt/corefs*. Nach VFS-Dispatch verhält sich `/System` wie
        // jeder andere Sub-Mount — siehe `fs::vfs::path::find_submount`.
        let mount_path = if want_system_mount
            && !system_mounted
            && dev.disk_id == root_disk_id
        {
            alloc::string::String::from("/System")
        } else if mount_index == 0 {
            alloc::string::String::from("/mnt/corefs")
        } else {
            alloc::format!("/mnt/corefs{}", mount_index)
        };
        let did_mount = fs::corefs::try_auto_mount_corefs(
            &mount_path,
            dev.disk_id,
            dev.start_lba as u32,
            dev.size_sectors,
            dev.id as u32,
        );
        if did_mount {
            if mount_path == "/System" {
                system_mounted = true;
            } else {
                mount_index += 1;
            }
        }
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
