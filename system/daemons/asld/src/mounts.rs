use crate::errors::AsldError;
use crate::model::MountSpec;

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

#[cfg(test)]
mod tests {
    use alloc::string::String;

    use crate::model::MountSpec;

    use super::validate_mount;

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
}
