use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::errors::AsldError;
use crate::model::{DistroConfig, DISTRO_IMAGES_ROOT};

pub const SMOKE_TEST_KERNEL_PROFILE: &str = "avm-smoke-test";
pub const SEABIOS_KERNEL_PROFILE: &str = "seabios-x86_64";
pub const SEABIOS_FIRMWARE_PATH: &str = "/System/var/asl/firmware/seabios.bin";
pub const LINUX_BOOT_PARAMS_ADDR: usize = 0x7000;
pub const LINUX_CMDLINE_ADDR: usize = 0x20_000;
pub const LINUX_KERNEL_LOAD_ADDR: usize = 0x10_0000;
pub const SEABIOS_RESET_VECTOR: usize = 0x0f_fff0;

const LINUX_BOOT_PARAMS_SIZE: usize = 4096;
const LINUX_SETUP_HEADER_MIN: usize = 0x240;
const LINUX_BOOT_FLAG_OFFSET: usize = 0x1fe;
const LINUX_HEADER_OFFSET: usize = 0x202;
const LINUX_VERSION_OFFSET: usize = 0x206;
const LINUX_TYPE_OF_LOADER_OFFSET: usize = 0x210;
const LINUX_LOADFLAGS_OFFSET: usize = 0x211;
const LINUX_INITRD_ADDR_OFFSET: usize = 0x218;
const LINUX_INITRD_SIZE_OFFSET: usize = 0x21c;
const LINUX_HEAP_END_PTR_OFFSET: usize = 0x224;
const LINUX_CMDLINE_PTR_OFFSET: usize = 0x228;
const LINUX_INITRD_ADDR_MAX_OFFSET: usize = 0x22c;
const LINUX_CMDLINE_SIZE_OFFSET: usize = 0x238;
const LINUX_BOOT_MAGIC: u32 = 0x5372_6448;
const LINUX_BOOT_FLAG: u16 = 0xaa55;
const DEFAULT_SETUP_SECTORS: usize = 4;
const SEABIOS_LOAD_TOP: usize = 0x10_0000;
const SEABIOS_MAX_SIZE: usize = 0x40_000;
const SEABIOS_MIN_ADDR: usize = 0x0c_0000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BootPlan {
    pub mode: String,
    pub rootfs_mode: String,
    pub firmware_path: String,
    pub kernel_path: String,
    pub initrd_path: String,
    pub cmdline: String,
    pub startable: bool,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectLinuxImage {
    pub setup_sectors: usize,
    pub protocol_version: u16,
    pub protected_mode_offset: usize,
    pub protected_mode_size: usize,
    pub entry_addr: usize,
    pub cmdline_limit: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectLinuxLayout {
    pub boot_params_addr: usize,
    pub kernel_load_addr: usize,
    pub kernel_entry_addr: usize,
    pub kernel_size: usize,
    pub cmdline_addr: usize,
    pub cmdline_size: usize,
    pub initrd_addr: usize,
    pub initrd_size: usize,
}

pub fn build_boot_plan(config: &DistroConfig) -> BootPlan {
    if config.kernel_profile == SMOKE_TEST_KERNEL_PROFILE {
        return BootPlan {
            mode: String::from("avm-smoke-test"),
            rootfs_mode: String::from("builtin"),
            firmware_path: String::from("-"),
            kernel_path: String::from("builtin://avm/hlt-bootstrap"),
            initrd_path: String::from("-"),
            cmdline: String::from("-"),
            startable: true,
            message: String::from("built-in AVM bootstrap smoke test"),
        };
    }

    if config.kernel_profile == SEABIOS_KERNEL_PROFILE {
        let firmware_present = file_exists(SEABIOS_FIRMWARE_PATH);
        return BootPlan {
            mode: String::from("firmware-seabios"),
            rootfs_mode: String::from("firmware"),
            firmware_path: String::from(SEABIOS_FIRMWARE_PATH),
            kernel_path: String::from("-"),
            initrd_path: String::from("-"),
            cmdline: String::from("-"),
            startable: firmware_present,
            message: if firmware_present {
                String::from("SeaBIOS firmware artifact is present")
            } else {
                String::from("SeaBIOS firmware artifact is missing")
            },
        };
    }

    let kernel_path = format!("{}/{}/boot/vmlinuz", DISTRO_IMAGES_ROOT, config.name);
    let initrd_path = format!("{}/{}/boot/initrd.img", DISTRO_IMAGES_ROOT, config.name);
    let kernel_present = file_exists(&kernel_path);
    let initrd_present = file_exists(&initrd_path);
    BootPlan {
        mode: String::from("direct-linux"),
        rootfs_mode: String::from("initrd"),
        firmware_path: String::from("-"),
        kernel_path,
        initrd_path,
        cmdline: build_linux_cmdline(config),
        startable: kernel_present && initrd_present,
        message: if kernel_present && initrd_present {
            String::from("direct Linux kernel and ASL initrd are present")
        } else if kernel_present {
            String::from("direct Linux initrd is required until ASL block storage is available")
        } else {
            String::from("direct Linux kernel image is missing")
        },
    }
}

pub fn is_smoke_test(config: &DistroConfig) -> bool {
    config.kernel_profile == SMOKE_TEST_KERNEL_PROFILE
}

pub fn is_seabios(config: &DistroConfig) -> bool {
    config.kernel_profile == SEABIOS_KERNEL_PROFILE
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeaBiosLayout {
    pub firmware_addr: usize,
    pub firmware_size: usize,
    pub reset_vector: usize,
}

pub fn prepare_seabios_boot(
    guest_memory: *mut u8,
    guest_memory_size: usize,
) -> Result<SeaBiosLayout, AsldError> {
    let firmware = read_file_bytes(SEABIOS_FIRMWARE_PATH)?;
    let layout = build_seabios_layout(firmware.len(), guest_memory_size)?;
    write_seabios_guest_memory(guest_memory, guest_memory_size, &firmware, &layout)?;
    Ok(layout)
}

pub fn prepare_direct_linux_boot(
    config: &DistroConfig,
    guest_memory: *mut u8,
    guest_memory_size: usize,
) -> Result<DirectLinuxLayout, AsldError> {
    let plan = build_boot_plan(config);
    if plan.mode != "direct-linux" {
        return Err(AsldError::InvalidState("not a direct Linux boot plan"));
    }

    if !plan.startable {
        return Err(AsldError::InvalidState(
            "direct Linux kernel and ASL initrd are required",
        ));
    }

    let kernel = read_file_bytes(&plan.kernel_path)?;
    let image = parse_direct_linux_image(&kernel)?;
    let initrd = read_file_bytes(&plan.initrd_path)?;
    let layout =
        build_direct_linux_layout(&image, plan.cmdline.len(), initrd.len(), guest_memory_size)?;
    write_direct_linux_guest_memory(
        guest_memory,
        guest_memory_size,
        &kernel,
        &image,
        &initrd,
        &plan.cmdline,
        &layout,
    )?;
    Ok(layout)
}

pub fn parse_direct_linux_image(data: &[u8]) -> Result<DirectLinuxImage, AsldError> {
    if data.len() < LINUX_SETUP_HEADER_MIN {
        return Err(AsldError::InvalidArgument(
            "Linux kernel image is too small",
        ));
    }
    if read_u16(data, LINUX_BOOT_FLAG_OFFSET)? != LINUX_BOOT_FLAG {
        return Err(AsldError::InvalidArgument(
            "Linux kernel boot flag is invalid",
        ));
    }
    if read_u32(data, LINUX_HEADER_OFFSET)? != LINUX_BOOT_MAGIC {
        return Err(AsldError::InvalidArgument("Linux setup header is missing"));
    }

    let setup_sectors = match data[0x1f1] {
        0 => DEFAULT_SETUP_SECTORS,
        value => value as usize,
    };
    let protected_mode_offset = (setup_sectors + 1) * 512;
    if protected_mode_offset >= data.len() {
        return Err(AsldError::InvalidArgument(
            "Linux protected-mode payload is missing",
        ));
    }
    let cmdline_limit = read_u32(data, LINUX_CMDLINE_SIZE_OFFSET).unwrap_or(2048) as usize;
    Ok(DirectLinuxImage {
        setup_sectors,
        protocol_version: read_u16(data, LINUX_VERSION_OFFSET)?,
        protected_mode_offset,
        protected_mode_size: data.len() - protected_mode_offset,
        entry_addr: LINUX_KERNEL_LOAD_ADDR,
        cmdline_limit: cmdline_limit.max(2048),
    })
}

fn build_seabios_layout(
    firmware_len: usize,
    guest_memory_size: usize,
) -> Result<SeaBiosLayout, AsldError> {
    if firmware_len == 0 || firmware_len > SEABIOS_MAX_SIZE {
        return Err(AsldError::InvalidArgument("SeaBIOS firmware size"));
    }
    if guest_memory_size < SEABIOS_LOAD_TOP {
        return Err(AsldError::InvalidState(
            "guest memory too small for SeaBIOS",
        ));
    }
    let firmware_addr = SEABIOS_LOAD_TOP
        .checked_sub(firmware_len)
        .ok_or(AsldError::InvalidState("SeaBIOS layout overflow"))?;
    if firmware_addr < SEABIOS_MIN_ADDR {
        return Err(AsldError::InvalidState("SeaBIOS firmware is too large"));
    }
    Ok(SeaBiosLayout {
        firmware_addr,
        firmware_size: firmware_len,
        reset_vector: SEABIOS_RESET_VECTOR,
    })
}

fn write_seabios_guest_memory(
    guest_memory: *mut u8,
    guest_memory_size: usize,
    firmware: &[u8],
    layout: &SeaBiosLayout,
) -> Result<(), AsldError> {
    let memory = unsafe { core::slice::from_raw_parts_mut(guest_memory, guest_memory_size) };
    memory.fill(0);
    copy_to_guest(memory, layout.firmware_addr, firmware)?;
    Ok(())
}

fn build_linux_cmdline(config: &DistroConfig) -> String {
    format!(
        "console=ttyS0 earlycon=uart,io,0x3f8 panic=-1 rdinit=/init asl.name={} asl.agent={} asl.rootfs=initrd asl.storage={}",
        config.name,
        if config.agent.enabled { "1" } else { "0" },
        config.storage.layout
    )
}

fn build_direct_linux_layout(
    image: &DirectLinuxImage,
    cmdline_len: usize,
    initrd_len: usize,
    guest_memory_size: usize,
) -> Result<DirectLinuxLayout, AsldError> {
    if cmdline_len + 1 > image.cmdline_limit {
        return Err(AsldError::InvalidArgument("Linux command line is too long"));
    }
    let kernel_end = LINUX_KERNEL_LOAD_ADDR
        .checked_add(image.protected_mode_size)
        .ok_or(AsldError::InvalidState("Linux kernel layout overflow"))?;
    if kernel_end > guest_memory_size {
        return Err(AsldError::InvalidState(
            "guest memory too small for Linux kernel",
        ));
    }
    let cmdline_end = LINUX_CMDLINE_ADDR
        .checked_add(cmdline_len + 1)
        .ok_or(AsldError::InvalidState("Linux cmdline layout overflow"))?;
    if cmdline_end > guest_memory_size {
        return Err(AsldError::InvalidState(
            "guest memory too small for Linux command line",
        ));
    }

    let initrd_addr = if initrd_len == 0 {
        0
    } else {
        let top = guest_memory_size.saturating_sub(0x20_0000);
        top.checked_sub(initrd_len)
            .map(|addr| addr & !0xfff)
            .ok_or(AsldError::InvalidState("guest memory too small for initrd"))?
    };
    if initrd_len != 0 && initrd_addr <= kernel_end {
        return Err(AsldError::InvalidState(
            "guest memory too small for kernel and initrd",
        ));
    }

    Ok(DirectLinuxLayout {
        boot_params_addr: LINUX_BOOT_PARAMS_ADDR,
        kernel_load_addr: LINUX_KERNEL_LOAD_ADDR,
        kernel_entry_addr: image.entry_addr,
        kernel_size: image.protected_mode_size,
        cmdline_addr: LINUX_CMDLINE_ADDR,
        cmdline_size: cmdline_len + 1,
        initrd_addr,
        initrd_size: initrd_len,
    })
}

fn write_direct_linux_guest_memory(
    guest_memory: *mut u8,
    guest_memory_size: usize,
    kernel: &[u8],
    image: &DirectLinuxImage,
    initrd: &[u8],
    cmdline: &str,
    layout: &DirectLinuxLayout,
) -> Result<(), AsldError> {
    let memory = unsafe { core::slice::from_raw_parts_mut(guest_memory, guest_memory_size) };
    memory.fill(0);

    let setup_len = image.protected_mode_offset.min(LINUX_BOOT_PARAMS_SIZE);
    copy_to_guest(memory, layout.boot_params_addr, &kernel[..setup_len])?;
    copy_to_guest(
        memory,
        layout.kernel_load_addr,
        &kernel[image.protected_mode_offset..],
    )?;
    copy_to_guest(memory, layout.cmdline_addr, cmdline.as_bytes())?;
    memory[layout.cmdline_addr + cmdline.len()] = 0;
    if !initrd.is_empty() {
        copy_to_guest(memory, layout.initrd_addr, initrd)?;
    }

    let params = slice_mut(memory, layout.boot_params_addr, LINUX_BOOT_PARAMS_SIZE)?;
    params[LINUX_TYPE_OF_LOADER_OFFSET] = 0xff;
    params[LINUX_LOADFLAGS_OFFSET] |= 0x80;
    write_u16(params, LINUX_HEAP_END_PTR_OFFSET, 0xe000)?;
    write_u32(params, LINUX_CMDLINE_PTR_OFFSET, layout.cmdline_addr as u32)?;
    write_u32(params, LINUX_INITRD_ADDR_MAX_OFFSET, 0xffff_ffff)?;
    write_u32(params, LINUX_INITRD_ADDR_OFFSET, layout.initrd_addr as u32)?;
    write_u32(params, LINUX_INITRD_SIZE_OFFSET, layout.initrd_size as u32)?;
    Ok(())
}

fn file_exists(path: &str) -> bool {
    let mut stat_buf = [0u32; 7];
    anyos_std::fs::stat(path, &mut stat_buf) == 0 && stat_buf[0] == 0
}

#[cfg(target_os = "linux")]
fn read_file_bytes(path: &str) -> Result<Vec<u8>, AsldError> {
    anyos_std::fs::read_to_vec(path).ok_or(AsldError::InvalidState(
        "Linux boot artifact is not readable",
    ))
}

#[cfg(not(target_os = "linux"))]
fn read_file_bytes(path: &str) -> Result<Vec<u8>, AsldError> {
    anyos_std::fs::read_to_vec(path)
        .map_err(|_| AsldError::InvalidState("Linux boot artifact is not readable"))
}

fn copy_to_guest(memory: &mut [u8], addr: usize, data: &[u8]) -> Result<(), AsldError> {
    slice_mut(memory, addr, data.len())?.copy_from_slice(data);
    Ok(())
}

fn slice_mut(memory: &mut [u8], addr: usize, len: usize) -> Result<&mut [u8], AsldError> {
    let end = addr
        .checked_add(len)
        .ok_or(AsldError::InvalidState("guest memory layout overflow"))?;
    memory
        .get_mut(addr..end)
        .ok_or(AsldError::InvalidState("guest memory layout out of bounds"))
}

fn read_u16(data: &[u8], offset: usize) -> Result<u16, AsldError> {
    let bytes = data
        .get(offset..offset + 2)
        .ok_or(AsldError::InvalidArgument("short Linux setup header"))?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(data: &[u8], offset: usize) -> Result<u32, AsldError> {
    let bytes = data
        .get(offset..offset + 4)
        .ok_or(AsldError::InvalidArgument("short Linux setup header"))?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn write_u16(data: &mut [u8], offset: usize, value: u16) -> Result<(), AsldError> {
    let bytes = data
        .get_mut(offset..offset + 2)
        .ok_or(AsldError::InvalidState("short Linux boot params"))?;
    bytes.copy_from_slice(&value.to_le_bytes());
    Ok(())
}

fn write_u32(data: &mut [u8], offset: usize, value: u32) -> Result<(), AsldError> {
    let bytes = data
        .get_mut(offset..offset + 4)
        .ok_or(AsldError::InvalidState("short Linux boot params"))?;
    bytes.copy_from_slice(&value.to_le_bytes());
    Ok(())
}

#[cfg(test)]
mod tests {
    use alloc::string::String;
    use alloc::vec::Vec;

    use super::{
        build_boot_plan, parse_direct_linux_image, LINUX_BOOT_PARAMS_ADDR, LINUX_CMDLINE_ADDR,
        LINUX_KERNEL_LOAD_ADDR, SEABIOS_KERNEL_PROFILE, SEABIOS_RESET_VECTOR,
        SMOKE_TEST_KERNEL_PROFILE,
    };
    use crate::distro::build_distro_config;

    #[test]
    fn default_distro_requires_direct_linux_loader() {
        let cfg = build_distro_config("ubuntu-dev", "ubuntu-24.04-x86_64-v1", "strati").unwrap();
        let plan = build_boot_plan(&cfg);
        assert_eq!(plan.mode, "direct-linux");
        assert!(!plan.startable);
        assert!(plan.kernel_path.ends_with("/ubuntu-dev/boot/vmlinuz"));
        assert!(plan.cmdline.contains("console=ttyS0"));
        assert!(plan.cmdline.contains("rdinit=/init"));
        assert!(!plan.cmdline.contains("/dev/vda"));
        assert_eq!(plan.rootfs_mode, "initrd");
        assert_eq!(plan.firmware_path, "-");
    }

    #[test]
    fn smoke_test_profile_uses_builtin_bootstrap() {
        let mut cfg = build_distro_config("smoke", "ubuntu-24.04-x86_64-v1", "strati").unwrap();
        cfg.kernel_profile = SMOKE_TEST_KERNEL_PROFILE.into();
        let plan = build_boot_plan(&cfg);
        assert_eq!(plan.mode, "avm-smoke-test");
        assert!(plan.startable);
        assert_eq!(plan.rootfs_mode, "builtin");
        assert_eq!(plan.firmware_path, "-");
        assert_eq!(plan.kernel_path, "builtin://avm/hlt-bootstrap");
    }

    #[test]
    fn seabios_profile_uses_userspace_firmware_artifact() {
        let mut cfg = build_distro_config("pcboot", "ubuntu-24.04-x86_64-v1", "strati").unwrap();
        cfg.kernel_profile = SEABIOS_KERNEL_PROFILE.into();
        let plan = build_boot_plan(&cfg);
        assert_eq!(plan.mode, "firmware-seabios");
        assert_eq!(plan.rootfs_mode, "firmware");
        assert_eq!(plan.kernel_path, "-");
        assert_eq!(plan.firmware_path, super::SEABIOS_FIRMWARE_PATH);
    }

    #[test]
    fn parses_bzimage_setup_header() {
        let image = fake_bzimage();
        let parsed = parse_direct_linux_image(&image).unwrap();
        assert_eq!(parsed.setup_sectors, 4);
        assert_eq!(parsed.protected_mode_offset, 5 * 512);
        assert_eq!(parsed.protected_mode_size, 64);
        assert_eq!(parsed.entry_addr, LINUX_KERNEL_LOAD_ADDR);
    }

    #[test]
    fn rejects_non_linux_kernel_image() {
        assert!(parse_direct_linux_image(&[0u8; 512]).is_err());
    }

    #[test]
    fn writes_direct_linux_guest_memory() {
        let mut cfg =
            build_distro_config("ubuntu-dev", "ubuntu-24.04-x86_64-v1", "strati").unwrap();
        cfg.kernel_profile = String::from("linux-x86_64-generic");
        let kernel = fake_bzimage();
        let mut memory = alloc::vec![0u8; 32 * 1024 * 1024];
        let image = parse_direct_linux_image(&kernel).unwrap();
        let layout = super::build_direct_linux_layout(
            &image,
            super::build_linux_cmdline(&cfg).len(),
            0,
            memory.len(),
        )
        .unwrap();
        super::write_direct_linux_guest_memory(
            memory.as_mut_ptr(),
            memory.len(),
            &kernel,
            &image,
            &[],
            &super::build_linux_cmdline(&cfg),
            &layout,
        )
        .unwrap();
        assert_eq!(
            &memory[LINUX_KERNEL_LOAD_ADDR..LINUX_KERNEL_LOAD_ADDR + 4],
            &[0x7f; 4]
        );
        assert_eq!(memory[LINUX_CMDLINE_ADDR], b'c');
        assert_eq!(memory[LINUX_BOOT_PARAMS_ADDR + 0x210], 0xff);
    }

    #[test]
    fn writes_seabios_at_top_of_low_memory() {
        let firmware = alloc::vec![0x5au8; 128 * 1024];
        let mut memory = alloc::vec![0xccu8; 2 * 1024 * 1024];
        let layout = super::build_seabios_layout(firmware.len(), memory.len()).unwrap();
        super::write_seabios_guest_memory(memory.as_mut_ptr(), memory.len(), &firmware, &layout)
            .unwrap();
        assert_eq!(layout.firmware_addr, 0xe0000);
        assert_eq!(layout.reset_vector, SEABIOS_RESET_VECTOR);
        assert_eq!(memory[0], 0);
        assert_eq!(memory[0xe0000], 0x5a);
        assert_eq!(memory[0xfffff], 0x5a);
    }

    fn fake_bzimage() -> Vec<u8> {
        let mut image = alloc::vec![0u8; 5 * 512 + 64];
        image[0x1f1] = 4;
        image[0x1fe..0x200].copy_from_slice(&0xaa55u16.to_le_bytes());
        image[0x202..0x206].copy_from_slice(&0x5372_6448u32.to_le_bytes());
        image[0x206..0x208].copy_from_slice(&0x020bu16.to_le_bytes());
        image[0x238..0x23c].copy_from_slice(&4096u32.to_le_bytes());
        for byte in &mut image[5 * 512..] {
            *byte = 0x7f;
        }
        image
    }
}
