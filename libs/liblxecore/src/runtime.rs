use crate::config::LxeConfig;

pub fn prepare_rootfs(config: &LxeConfig) {
    crate::rootfs::ensure_rootfs_layout(config);
    crate::rootfs::repair_rootfs_runtime(&config.rootfs);
    anyos_std::fs::sync();
}
