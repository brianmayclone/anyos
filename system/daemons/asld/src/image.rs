use crate::errors::AsldError;

pub fn is_valid_image_ref(image_ref: &str) -> bool {
    !image_ref.is_empty()
        && image_ref.bytes().all(|b| matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.'))
}

pub fn validate_image_ref(image_ref: &str) -> Result<(), AsldError> {
    if is_valid_image_ref(image_ref) {
        Ok(())
    } else {
        Err(AsldError::InvalidArgument("base image ref"))
    }
}

#[cfg(test)]
mod tests {
    use super::is_valid_image_ref;

    #[test]
    fn accepts_safe_image_refs() {
        assert!(is_valid_image_ref("ubuntu-24.04-x86_64-v1"));
        assert!(is_valid_image_ref("debian_13.dev"));
    }

    #[test]
    fn rejects_unsafe_image_refs() {
        assert!(!is_valid_image_ref(""));
        assert!(!is_valid_image_ref("../ubuntu"));
        assert!(!is_valid_image_ref("ubuntu/dev"));
    }
}
