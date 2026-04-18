use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::format;

use crate::model::PackageInstallResult;
use crate::util::{ensure_dir, ensure_parent_dirs, join_root};

pub fn install_package_archive(
    archive_path: &str,
    root: &str,
) -> Result<PackageInstallResult, String> {
    let reader = libzip_client::TarReader::open(archive_path)
        .ok_or_else(|| format!("could not open archive {}", archive_path))?;
    let count = reader.entry_count();
    let mut prefix: Option<String> = None;

    for i in 0..count {
        let name = reader.entry_name(i);
        if name.ends_with("/pkg.json") {
            if let Some(slash) = name.rfind('/') {
                prefix = Some(format!("{}/files/", &name[..slash]));
            }
            break;
        }
    }

    let prefix = prefix.ok_or_else(|| format!("archive {} has no pkg.json", archive_path))?;
    let mut files = Vec::new();

    for i in 0..count {
        let name = reader.entry_name(i);
        if !name.starts_with(&prefix) {
            continue;
        }
        let rel = &name[prefix.len()..];
        if rel.is_empty() {
            continue;
        }

        let target = join_root(root, &format!("/{}", rel));
        if reader.entry_is_dir(i) {
            ensure_dir(&target);
            continue;
        }

        ensure_parent_dirs(&target);
        if !reader.extract_to_file(i, &target) {
            return Err(format!("failed to extract {}", target));
        }
        files.push(target);
    }

    Ok(PackageInstallResult { files })
}
