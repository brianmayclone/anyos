use alloc::string::String;
use alloc::vec::Vec;
use alloc::{format, vec};

use crate::errors::AsldError;
use crate::model::DistroConfig;

const SECTOR_SIZE: usize = 512;
const TOTAL_SECTORS: usize = 16_384;
const RESERVED_SECTORS: usize = 1;
const FAT_COUNT: usize = 2;
const ROOT_ENTRIES: usize = 512;
const ROOT_DIR_SECTORS: usize = (ROOT_ENTRIES * 32) / SECTOR_SIZE;
const SECTORS_PER_FAT: usize = 64;
const SECTORS_PER_CLUSTER: usize = 1;
const FIRST_DATA_SECTOR: usize = RESERVED_SECTORS + FAT_COUNT * SECTORS_PER_FAT + ROOT_DIR_SECTORS;

#[cfg(target_os = "linux")]
pub fn ensure_seed_image(_config: &DistroConfig) -> Result<(), AsldError> {
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn ensure_seed_image(config: &DistroConfig) -> Result<(), AsldError> {
    ensure_parent_dirs(&config.name)?;
    let image = build_seed_image(config)?;
    write_seed_image(&config.storage.seed_image_path, &image)
}

pub fn build_seed_image(config: &DistroConfig) -> Result<Vec<u8>, AsldError> {
    let files = seed_files(config);
    let mut image = vec![0u8; TOTAL_SECTORS * SECTOR_SIZE];
    write_boot_sector(&mut image);
    let mut next_cluster = 2u16;
    let mut dir_offset = RESERVED_SECTORS * SECTOR_SIZE + FAT_COUNT * SECTORS_PER_FAT * SECTOR_SIZE;
    write_volume_label(&mut image, &mut dir_offset);

    for (long_name, short_name, data) in files {
        let start_cluster = next_cluster;
        let clusters = cluster_count(data.len()).max(1);
        next_cluster = next_cluster
            .checked_add(clusters as u16)
            .ok_or(AsldError::InvalidState("seed image cluster overflow"))?;
        write_fat_chain(&mut image, start_cluster, clusters);
        write_file_data(&mut image, start_cluster, &data)?;
        write_dir_entries(
            &mut image,
            &mut dir_offset,
            long_name,
            *short_name,
            start_cluster,
            data.len() as u32,
        )?;
    }

    Ok(image)
}

fn seed_files(config: &DistroConfig) -> Vec<(&'static str, &'static [u8; 11], Vec<u8>)> {
    vec![
        ("user-data", b"USERDA~1   ", user_data(config).into_bytes()),
        ("meta-data", b"METADA~1   ", meta_data(config).into_bytes()),
        (
            "network-config",
            b"NETWOR~1   ",
            network_config().into_bytes(),
        ),
    ]
}

fn user_data(config: &DistroConfig) -> String {
    format!(
        "#cloud-config\n\
users:\n\
  - name: anyos\n\
    gecos: anyOS ASL User\n\
    groups: [sudo]\n\
    shell: /bin/bash\n\
    lock_passwd: false\n\
    sudo: ['ALL=(ALL) NOPASSWD:ALL']\n\
chpasswd:\n\
  expire: false\n\
  users:\n\
    - name: anyos\n\
      password: anyos\n\
      type: text\n\
ssh_pwauth: true\n\
disable_root: false\n\
manage_etc_hosts: true\n\
write_files:\n\
  - path: /etc/profile.d/anyos-asl.sh\n\
    permissions: '0644'\n\
    content: |\n\
      export ASL_DISTRO={}\n\
      export ASL_INTEROP=1\n\
      cd /home/anyos 2>/dev/null || true\n\
  - path: /etc/asl-release\n\
    permissions: '0644'\n\
    content: |\n\
      NAME=anyOS-ASL\n\
      DISTRO={}\n\
      INTEGRATION=cloud-init\n\
runcmd:\n\
  - [ sh, -lc, \"systemctl enable serial-getty@ttyS0.service || true\" ]\n\
  - [ sh, -lc, \"systemctl start serial-getty@ttyS0.service || true\" ]\n\
  - [ sh, -lc, \"printf 'anyOS ASL Debian is ready. Login: anyos / anyos\\n' > /etc/motd\" ]\n\
final_message: 'anyOS ASL Debian ready after $UPTIME seconds'\n",
        config.name, config.name
    )
}

fn meta_data(config: &DistroConfig) -> String {
    format!(
        "instance-id: anyos-asl-{}\nlocal-hostname: {}\n",
        config.name, config.name
    )
}

fn network_config() -> String {
    String::from(
        "version: 2\n\
ethernets:\n\
  anyos0:\n\
    match:\n\
      name: \"en*\"\n\
    dhcp4: true\n\
    dhcp6: false\n",
    )
}

fn write_boot_sector(image: &mut [u8]) {
    image[0] = 0xeb;
    image[1] = 0x3c;
    image[2] = 0x90;
    image[3..11].copy_from_slice(b"ANYOSASL");
    write_u16(image, 11, SECTOR_SIZE as u16);
    image[13] = SECTORS_PER_CLUSTER as u8;
    write_u16(image, 14, RESERVED_SECTORS as u16);
    image[16] = FAT_COUNT as u8;
    write_u16(image, 17, ROOT_ENTRIES as u16);
    write_u16(image, 19, TOTAL_SECTORS as u16);
    image[21] = 0xf8;
    write_u16(image, 22, SECTORS_PER_FAT as u16);
    write_u16(image, 24, 63);
    write_u16(image, 26, 255);
    image[36] = 0x80;
    image[38] = 0x29;
    write_u32(image, 39, 0x4153_4c31);
    image[43..54].copy_from_slice(b"CIDATA     ");
    image[54..62].copy_from_slice(b"FAT16   ");
    image[510] = 0x55;
    image[511] = 0xaa;

    write_fat_entry(image, 0, 0xfff8);
    write_fat_entry(image, 1, 0xffff);
}

fn write_volume_label(image: &mut [u8], offset: &mut usize) {
    image[*offset..*offset + 11].copy_from_slice(b"CIDATA     ");
    image[*offset + 11] = 0x08;
    *offset += 32;
}

fn write_dir_entries(
    image: &mut [u8],
    offset: &mut usize,
    long_name: &str,
    short_name: [u8; 11],
    first_cluster: u16,
    size: u32,
) -> Result<(), AsldError> {
    let checksum = lfn_checksum(&short_name);
    let chars = utf16_name(long_name);
    let entry_count = (chars.len() + 12) / 13;
    for entry in (1..=entry_count).rev() {
        let seq = if entry == entry_count {
            0x40 | entry as u8
        } else {
            entry as u8
        };
        write_lfn_entry(
            image,
            *offset,
            seq,
            checksum,
            &chars[(entry - 1) * 13..chars.len().min(entry * 13)],
        )?;
        *offset += 32;
    }

    image[*offset..*offset + 11].copy_from_slice(&short_name);
    image[*offset + 11] = 0x20;
    write_u16(image, *offset + 26, first_cluster);
    write_u32(image, *offset + 28, size);
    *offset += 32;
    Ok(())
}

fn write_lfn_entry(
    image: &mut [u8],
    offset: usize,
    seq: u8,
    checksum: u8,
    chars: &[u16],
) -> Result<(), AsldError> {
    if offset + 32 > image.len() {
        return Err(AsldError::InvalidState("seed root directory overflow"));
    }
    image[offset] = seq;
    image[offset + 11] = 0x0f;
    image[offset + 13] = checksum;
    let positions = [1usize, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30];
    for (index, pos) in positions.iter().copied().enumerate() {
        let value = if index < chars.len() {
            chars[index]
        } else if index == chars.len() {
            0
        } else {
            0xffff
        };
        write_u16(image, offset + pos, value);
    }
    Ok(())
}

fn utf16_name(name: &str) -> Vec<u16> {
    name.bytes().map(|b| b as u16).collect()
}

fn lfn_checksum(short_name: &[u8; 11]) -> u8 {
    let mut sum = 0u8;
    for byte in short_name {
        sum = ((sum & 1) << 7).wrapping_add(sum >> 1).wrapping_add(*byte);
    }
    sum
}

fn cluster_count(len: usize) -> usize {
    (len + SECTOR_SIZE - 1) / SECTOR_SIZE
}

fn write_fat_chain(image: &mut [u8], start_cluster: u16, clusters: usize) {
    for index in 0..clusters {
        let cluster = start_cluster + index as u16;
        let next = if index + 1 == clusters {
            0xffff
        } else {
            cluster + 1
        };
        write_fat_entry(image, cluster, next);
    }
}

fn write_fat_entry(image: &mut [u8], cluster: u16, value: u16) {
    for fat in 0..FAT_COUNT {
        let base = (RESERVED_SECTORS + fat * SECTORS_PER_FAT) * SECTOR_SIZE;
        write_u16(image, base + cluster as usize * 2, value);
    }
}

fn write_file_data(image: &mut [u8], start_cluster: u16, data: &[u8]) -> Result<(), AsldError> {
    let offset = cluster_offset(start_cluster)?;
    let end = offset
        .checked_add(data.len())
        .ok_or(AsldError::InvalidState("seed data overflow"))?;
    if end > image.len() {
        return Err(AsldError::InvalidState("seed data out of bounds"));
    }
    image[offset..end].copy_from_slice(data);
    Ok(())
}

fn cluster_offset(cluster: u16) -> Result<usize, AsldError> {
    let cluster_index = cluster
        .checked_sub(2)
        .ok_or(AsldError::InvalidState("invalid seed cluster"))? as usize;
    Ok((FIRST_DATA_SECTOR + cluster_index * SECTORS_PER_CLUSTER) * SECTOR_SIZE)
}

fn write_u16(buf: &mut [u8], offset: usize, value: u16) {
    buf[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn write_u32(buf: &mut [u8], offset: usize, value: u32) {
    buf[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

#[cfg(not(target_os = "linux"))]
fn ensure_parent_dirs(name: &str) -> Result<(), AsldError> {
    for dir in [
        String::from("/System/var"),
        String::from("/System/var/asl"),
        String::from("/System/var/asl/distros"),
        format!("/System/var/asl/distros/{name}"),
        format!("/System/var/asl/distros/{name}/images"),
    ] {
        let _ = anyos_std::fs::mkdir(&dir);
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn write_seed_image(path: &str, image: &[u8]) -> Result<(), AsldError> {
    let fd = anyos_std::fs::open(
        path,
        anyos_std::fs::O_WRITE | anyos_std::fs::O_CREATE | anyos_std::fs::O_TRUNC,
    );
    if fd == 0 || fd == u32::MAX {
        return Err(AsldError::InvalidState("could not create cloud-init seed"));
    }
    let written = anyos_std::fs::write(fd, image);
    let _ = anyos_std::fs::close(fd);
    if written == image.len() as u32 {
        Ok(())
    } else {
        Err(AsldError::InvalidState("cloud-init seed write failed"))
    }
}

#[cfg(test)]
mod tests {
    use super::build_seed_image;
    use crate::distro::build_distro_config_with_kernel_profile;

    #[test]
    fn builds_cidata_fat_seed_image() {
        let cfg = build_distro_config_with_kernel_profile(
            "debian",
            "debian-13-nocloud-amd64-raw",
            "root",
            "seabios-x86_64",
        )
        .unwrap();
        let image = build_seed_image(&cfg).unwrap();
        assert_eq!(image.len(), 16_384 * 512);
        assert_eq!(&image[43..54], b"CIDATA     ");
        assert_eq!(image[510], 0x55);
        assert_eq!(image[511], 0xaa);
        assert!(image.windows("USERDA~1".len()).any(|w| w == b"USERDA~1"));
        assert!(image
            .windows("anyOS ASL Debian is ready".len())
            .any(|w| w == b"anyOS ASL Debian is ready"));
    }
}
