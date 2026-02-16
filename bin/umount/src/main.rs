#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let args = anyos_std::process::args(&mut args_buf);

    if args.is_empty() {
        anyos_std::println!("Usage: umount mountpoint");
        anyos_std::println!("Example: umount /mnt/cdrom0");
        return;
    }

    let mount_point = args.trim();
    let result = anyos_std::fs::umount(mount_point);
    if result == u32::MAX {
        anyos_std::println!("umount: failed to unmount {}", mount_point);
    }
}
