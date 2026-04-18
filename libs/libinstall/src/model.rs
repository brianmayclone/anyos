use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

#[derive(Clone)]
pub struct ExistingInstallation {
    pub version: String,
    pub has_users: bool,
}

#[derive(Clone)]
pub struct UpgradeList {
    pub from_version: String,
    pub to_version: String,
    pub operations: Vec<UpgradeOperation>,
}

#[derive(Clone)]
pub enum UpgradeOperation {
    EnsureDir {
        path: String,
    },
    SyncTree {
        source: String,
        target: String,
        merge_configs: bool,
        preserve_existing: bool,
    },
}

#[derive(Default, Clone, Copy)]
pub struct ApplyStats {
    pub files_copied: u32,
    pub files_preserved: u32,
    pub configs_merged: u32,
    pub dirs_created: u32,
    pub errors: u32,
    pub files_rolled_back: u32,
}

pub struct PackageInstallResult {
    pub files: Vec<String>,
}

pub fn default_upgrade_list() -> UpgradeList {
    UpgradeList {
        from_version: String::from("0.0.0"),
        to_version: String::from("0.4.0"),
        operations: vec![
            UpgradeOperation::EnsureDir {
                path: String::from("/System/etc/apkg"),
            },
            UpgradeOperation::SyncTree {
                source: String::from("/System"),
                target: String::from("/System"),
                merge_configs: true,
                preserve_existing: false,
            },
            UpgradeOperation::SyncTree {
                source: String::from("/Applications"),
                target: String::from("/Applications"),
                merge_configs: true,
                preserve_existing: false,
            },
            UpgradeOperation::SyncTree {
                source: String::from("/Libraries"),
                target: String::from("/Libraries"),
                merge_configs: false,
                preserve_existing: false,
            },
            UpgradeOperation::SyncTree {
                source: String::from("/boot"),
                target: String::from("/boot"),
                merge_configs: true,
                preserve_existing: false,
            },
            UpgradeOperation::SyncTree {
                source: String::from("/media"),
                target: String::from("/media"),
                merge_configs: false,
                preserve_existing: false,
            },
        ],
    }
}
