//! Host-mode i18n — identity translation (returns key as-is).

pub fn init() {}

pub fn t(key: &str) -> &str {
    key
}

pub fn lang() -> &'static str {
    "en"
}
