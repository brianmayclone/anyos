use alloc::string::String;
use alloc::vec::Vec;

use crate::errors::AsldError;
use crate::model::{MountSpec, MountValidation};

pub fn validate_mount(spec: &MountSpec) -> Result<(), AsldError> {
    if spec.host_path.is_empty() || !spec.host_path.starts_with('/') {
        return Err(AsldError::InvalidPath);
    }
    if spec.guest_path.is_empty() || !spec.guest_path.starts_with('/') {
        return Err(AsldError::InvalidPath);
    }
    if spec.mode != "readonly" && spec.mode != "readwrite" {
        return Err(AsldError::InvalidArgument("mount mode"));
    }
    Ok(())
}

pub fn validate_mount_set(mounts: &[MountSpec]) -> Vec<MountValidation> {
    let mut out = Vec::new();
    for mount in mounts {
        let (valid, message) = match validate_mount(mount) {
            Ok(()) if host_path_reachable(&mount.host_path) => {
                (true, String::from("mount export reachable"))
            }
            Ok(()) => (false, String::from("host path is not reachable")),
            Err(err) => (false, err.message()),
        };
        out.push(MountValidation {
            id: mount.id.clone(),
            guest_path: mount.guest_path.clone(),
            valid,
            message,
        });
    }
    out
}

#[cfg(target_os = "linux")]
fn host_path_reachable(path: &str) -> bool {
    std::path::Path::new(path).exists()
}

#[cfg(not(target_os = "linux"))]
fn host_path_reachable(_path: &str) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use alloc::string::String;

    use crate::model::MountSpec;

    use super::{validate_mount, validate_mount_set};

    fn mount() -> MountSpec {
        MountSpec {
            id: String::from("workspace"),
            host_path: String::from("/Users/test/projects"),
            guest_path: String::from("/mnt/projects"),
            mode: String::from("readwrite"),
            metadata_mode: String::from("relaxed"),
            case_mode: String::from("host-native"),
            exec_policy: String::from("inherit"),
            watch_policy: String::from("best-effort"),
            description: String::new(),
        }
    }

    #[test]
    fn validates_good_mount() {
        assert!(validate_mount(&mount()).is_ok());
    }

    #[test]
    fn rejects_non_absolute_guest_path() {
        let mut spec = mount();
        spec.guest_path = String::from("mnt/projects");
        assert!(validate_mount(&spec).is_err());
    }

    #[test]
    fn validation_report_marks_unreachable_host_path() {
        let mut spec = mount();
        spec.host_path = String::from("/definitely/not/an/asl/test/path");
        let report = validate_mount_set(&[spec]);
        assert_eq!(report.len(), 1);
        assert!(!report[0].valid);
    }
}
