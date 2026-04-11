//! ARM64 boot flow orchestration.

use crate::arch;
use crate::boot::{self, GPU_ACCEL, GPU_HW_CURSOR, NOGUI, SETUP_MODE};
use crate::drivers;
use crate::fs;
use crate::graphics;
use crate::ipc;
use crate::memory;
use crate::net;
use crate::serial_println;
use crate::task;
use core::sync::atomic::Ordering;

pub(super) fn kernel_main(dtb_addr: u64) -> ! {
    crate::drivers::serial::init();
    crate::drivers::serial::set_verbose(true);
    arch::arm64::boot::save_dtb_addr(dtb_addr);
    boot::set_boot_mode(0);

    serial_println!("  .anyOS Kernel (arm64) v{}", env!("CARGO_PKG_VERSION"));
    serial_println!("  DTB @ {:#018x}", dtb_addr);

    init_cpu();
    init_memory();
    init_platform();
    init_devices();
    init_scheduler();
    enable_interrupts();
    init_storage();
    init_userspace();
}

fn init_cpu() {
    arch::arm64::cpu_features::detect();
    arch::arm64::power::init();
}

fn init_memory() {
    let (ram_base, ram_size) = arch::arm64::boot::detect_memory();
    memory::physical::init_arm64(ram_base, ram_size);
    memory::heap::init();
    serial_println!("[OK] Heap allocator initialized");
    graphics::cc_font::init();
}

fn init_platform() {
    arch::arm64::exceptions::init();
    arch::arm64::gic::init_distributor();
    arch::arm64::smp::init_bsp();
    arch::arm64::gic::init_cpu(arch::hal::cpu_id());
    arch::arm64::exceptions::register_irq(0, halt_ipi_handler);
    arch::arm64::exceptions::register_irq(1, resched_ipi_handler);
    arch::arm64::exceptions::register_irq(30, arch::arm64::generic_timer::irq_handler_with_schedule);
    arch::arm64::generic_timer::init();
    arch::arm64::syscall::init_bsp();
    serial_println!("[OK] ARM64 platform initialized");
}

fn init_devices() {
    drivers::hal::init();
    drivers::hal::register_legacy_devices();

    for dev in drivers::arm::probe_all() {
        match dev.device_id() {
            1 => drivers::network::init_mmio(&dev),
            2 => drivers::arm::blk::init(&dev),
            16 => drivers::arm::gpu::init(&dev),
            18 => drivers::arm::input::init(&dev),
            other => serial_println!("  [ARM64] Unhandled VirtIO MMIO device id={}", other),
        }
    }

    drivers::boot_console::init();
}

fn init_scheduler() {
    task::scheduler::init();
    serial_println!("[OK] Scheduler initialized");
}

fn enable_interrupts() {
    arch::hal::enable_interrupts();
    serial_println!("[OK] Interrupts enabled");
}

fn init_storage() {
    use drivers::storage::blockdev;
    use fs::partition::PartitionType;

    let capacity = drivers::arm::blk::capacity();
    if capacity == 0 {
        serial_println!("  [ARM64] No block device detected");
        fs::vfs::init();
        fs::vfs::mount_devfs();
        return;
    }

    blockdev::register_device(blockdev::BlockDevice {
        id: 0,
        disk_id: 0,
        partition: None,
        part_type: PartitionType::Empty,
        start_lba: 0,
        size_sectors: capacity,
        label: [0u8; 40],
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
            }
        }
    }

    fs::blockcache::init();
    fs::vfs::init();
    fs::vfs::mount("/", fs::vfs::FsType::Fat, 0);
    fs::vfs::mount_devfs();
}

fn init_userspace() -> ! {
    task::users::init();

    if drivers::network::is_available() {
        net::init();
        net::load_config_files();
    }

    ipc::event_bus::system_emit(ipc::event_bus::EventData::new(
        ipc::event_bus::EVT_BOOT_COMPLETE,
        0,
        0,
        0,
        0,
    ));

    let cpu_count = arch::arm64::boot::detect_cpu_count().min(arch::hal::MAX_CPUS);
    if cpu_count > 1 {
        arch::arm64::smp::start_aps(cpu_count);
    }

    if !NOGUI.load(Ordering::Relaxed) {
        load_boot_dlls();
    }

    drivers::boot_console::stop_spinner();

    if drivers::framebuffer::is_available() && !NOGUI.load(Ordering::Relaxed) {
        GPU_ACCEL.store(false, Ordering::Relaxed);
        GPU_HW_CURSOR.store(false, Ordering::Relaxed);
        spawn_compositor(SETUP_MODE.load(Ordering::Relaxed));
    } else {
        spawn_init();
    }

    task::scheduler::run();
}

fn spawn_compositor(setup_mode: bool) {
    let result = if setup_mode {
        task::loader::load_and_run_with_args(
            "/System/compositor/compositor",
            "compositor",
            "compositor setupmode",
        )
    } else {
        task::loader::load_and_run("/System/compositor/compositor", "compositor")
    };

    match result {
        Ok(tid) => serial_println!("[OK] Userspace compositor spawned (TID={})", tid),
        Err(err) => {
            serial_println!("  WARN: Failed to load compositor: {}", err);
            spawn_init();
        }
    }
}

fn spawn_init() {
    match task::loader::load_and_run("/System/bin/init", "init") {
        Ok(tid) => serial_println!("[OK] Init spawned (TID={})", tid),
        Err(err) => serial_println!("  WARN: Failed to load init: {}", err),
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

fn halt_ipi_handler() {
    arch::hal::disable_interrupts();
    loop {
        arch::hal::halt();
    }
}

fn resched_ipi_handler() {
    task::scheduler::schedule_tick();
}
