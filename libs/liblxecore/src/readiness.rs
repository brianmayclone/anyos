use alloc::string::String;
use anyos_std::fs;

use crate::config::LxeConfig;
use crate::rootfs::{find_linux_bash, path_exists};

#[derive(Clone)]
pub enum LxeShellAvailability {
    Ready { rootfs: String, bash: String },
    MissingRootfs { rootfs: String },
    MissingBash { rootfs: String },
}

impl LxeShellAvailability {
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }
}

pub fn shell_availability(config: &LxeConfig) -> LxeShellAvailability {
    let mut stat_buf = [0u32; 7];
    if fs::stat(&config.rootfs, &mut stat_buf) != 0 || !path_exists(&config.rootfs) {
        return LxeShellAvailability::MissingRootfs {
            rootfs: config.rootfs.clone(),
        };
    }

    match find_linux_bash(&config.rootfs) {
        Some(bash) => LxeShellAvailability::Ready {
            rootfs: config.rootfs.clone(),
            bash,
        },
        None => LxeShellAvailability::MissingBash {
            rootfs: config.rootfs.clone(),
        },
    }
}
