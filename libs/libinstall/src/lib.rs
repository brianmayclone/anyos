#![no_std]

extern crate alloc;

mod detect;
mod merge;
mod model;
mod package;
mod preflight;
mod sync;
mod transaction;
mod util;

pub use detect::{detect_existing_installation, read_system_version};
pub use model::{
    default_upgrade_list, ApplyStats, ExistingInstallation, PackageInstallResult, UpgradeList,
    UpgradeOperation,
};
pub use package::install_package_archive;
pub use preflight::{
    preflight_upgrade, PreflightCheck, PreflightCode, PreflightIssue, PreflightReport,
};
pub use sync::{apply_upgrade_list, load_upgrade_list};
pub use transaction::{has_pending_upgrade, recover_pending_upgrade};
