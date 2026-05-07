use alloc::format;
use alloc::string::String;
use anyos_std::fs;

use crate::constants::*;

#[derive(Clone)]
pub(crate) struct ManagerConfig {
    pub(crate) asl_root: String,
    pub(crate) distros_root: String,
    pub(crate) distro_root: String,
    pub(crate) images_dir: String,
    pub(crate) base_image_path: String,
    pub(crate) base_image_tmp: String,
    pub(crate) debian_raw_url: String,
}

impl ManagerConfig {
    fn defaults() -> Self {
        let asl_root = String::from(DEFAULT_ASL_ROOT);
        let distros_root = join_path(&asl_root, DEFAULT_DISTROS_DIR);
        Self::from_roots(asl_root, distros_root, DEBIAN_RAW_URL)
    }

    fn from_roots(asl_root: String, distros_root: String, debian_raw_url: &str) -> Self {
        let distro_root = join_path(&distros_root, DISTRO_NAME);
        let images_dir = join_path(&distro_root, DEFAULT_IMAGES_DIR);
        Self {
            asl_root,
            distros_root,
            distro_root,
            base_image_path: join_path(&images_dir, "base.img"),
            base_image_tmp: join_path(&images_dir, "base.img.part"),
            images_dir,
            debian_raw_url: String::from(debian_raw_url),
        }
    }
}

pub(crate) fn load() -> ManagerConfig {
    let mut cfg = ManagerConfig::defaults();
    let Ok(text) = fs::read_to_string(CONFIG_PATH) else {
        return cfg;
    };

    let mut asl_root = cfg.asl_root.clone();
    let mut distros_root = cfg.distros_root.clone();
    let mut raw_url = cfg.debian_raw_url.clone();

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = split_key_value(line) else {
            continue;
        };
        match key {
            "asl_root" if is_safe_absolute_dir(value) => asl_root = String::from(value),
            "distros_root" if is_safe_absolute_dir(value) => distros_root = String::from(value),
            "debian_raw_url" if is_allowed_debian_url(value) => raw_url = String::from(value),
            _ => {}
        }
    }

    if distros_root == ManagerConfig::defaults().distros_root {
        distros_root = join_path(&asl_root, DEFAULT_DISTROS_DIR);
    }
    cfg = ManagerConfig::from_roots(asl_root, distros_root, &raw_url);
    cfg
}

pub(crate) use aslmanager_core::{is_allowed_debian_url, is_safe_absolute_dir};
use aslmanager_core::{join_path, split_key_value};
