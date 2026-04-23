use alloc::format;
use alloc::string::String;

use libconf::ConfError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AsldError {
    InvalidArgument(&'static str),
    InvalidName,
    InvalidPath,
    MissingField(&'static str),
    NotFound,
    InvalidState(&'static str),
    PolicyDenied,
    BackendUnavailable(&'static str),
    NotImplemented(&'static str),
    Config(ConfError),
}

impl AsldError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidArgument(_) => "INVALID_ARGUMENT",
            Self::InvalidName => "INVALID_NAME",
            Self::InvalidPath => "INVALID_PATH",
            Self::MissingField(_) => "MISSING_FIELD",
            Self::NotFound => "DISTRO_NOT_FOUND",
            Self::InvalidState(_) => "INVALID_STATE",
            Self::PolicyDenied => "POLICY_DENIED",
            Self::BackendUnavailable(_) => "BACKEND_UNAVAILABLE",
            Self::NotImplemented(_) => "NOT_IMPLEMENTED",
            Self::Config(_) => "CONFIG_BACKEND_ERROR",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::InvalidArgument(msg) => format!("invalid argument: {msg}"),
            Self::InvalidName => String::from("invalid distro name"),
            Self::InvalidPath => String::from("invalid path"),
            Self::MissingField(field) => format!("missing required field: {field}"),
            Self::NotFound => String::from("distribution not found"),
            Self::InvalidState(msg) => format!("invalid state: {msg}"),
            Self::PolicyDenied => String::from("operation denied by policy"),
            Self::BackendUnavailable(msg) => format!("backend unavailable: {msg}"),
            Self::NotImplemented(msg) => format!("not implemented: {msg}"),
            Self::Config(err) => format!("configuration backend error: {:?}", err),
        }
    }
}

impl From<ConfError> for AsldError {
    fn from(value: ConfError) -> Self {
        Self::Config(value)
    }
}

#[cfg(test)]
mod tests {
    use super::AsldError;

    #[test]
    fn maps_error_codes_stably() {
        assert_eq!(AsldError::InvalidName.code(), "INVALID_NAME");
        assert_eq!(AsldError::NotFound.code(), "DISTRO_NOT_FOUND");
        assert_eq!(
            AsldError::NotImplemented("vm backend").code(),
            "NOT_IMPLEMENTED"
        );
    }

    #[test]
    fn renders_human_message() {
        let msg = AsldError::InvalidState("already running").message();
        assert!(msg.contains("already running"));
    }
}
