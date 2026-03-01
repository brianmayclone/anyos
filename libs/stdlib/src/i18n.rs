//! Internationalization (i18n) support for anyOS.
//!
//! Provides a simple translation system based on JSON files stored in
//! `/System/translations/{lang}.json`. English strings are used as keys,
//! so if no translation is found the original English text is returned.
//!
//! # Usage
//! ```ignore
//! use anyos_std::i18n;
//!
//! // Call once at startup (after anyui::init() for GUI apps)
//! i18n::init();
//!
//! // Translate strings — returns original if no translation found
//! let label = i18n::t("Save");   // → "Speichern" if lang=de
//! let lang  = i18n::lang();      // → "de"
//! ```
//!
//! # Language detection order
//! 1. `LANG` environment variable (e.g. `"de"`)
//! 2. `/System/settings/language.conf` file contents
//! 3. Fallback: `"en"`

use alloc::string::String;
use crate::hashmap::HashMap;
use crate::{env, fs, json};

/// Path to the system-wide language preference file.
const LANG_CONF_PATH: &str = "/System/settings/language.conf";

/// Base directory for translation JSON files.
const TRANSLATIONS_DIR: &str = "/System/translations";

// ── Global state ─────────────────────────────────────────────────────────

static mut TRANSLATIONS: Option<HashMap<String, String>> = None;
static mut CURRENT_LANG: Option<String> = None;

// ── Public API ───────────────────────────────────────────────────────────

/// Detect the current language and load the corresponding translation file.
///
/// Call this once at program startup. For GUI apps, call after `anyui::init()`.
/// Safe to call multiple times (reloads translations).
pub fn init() {
    let lang = detect_language();

    // English is the default — no translation file needed
    if lang == "en" {
        unsafe {
            CURRENT_LANG = Some(String::from("en"));
            TRANSLATIONS = None;
        }
        return;
    }

    // Load translation file
    let path = alloc::format!("{}/{}.json", TRANSLATIONS_DIR, lang);
    let map = load_translations(&path);

    unsafe {
        CURRENT_LANG = Some(String::from(lang.as_str()));
        TRANSLATIONS = map;
    }
}

/// Translate a string. Returns the translated text if available, otherwise
/// returns the original English key unchanged.
///
/// If `init()` has not been called yet, always returns the key as-is.
#[inline]
pub fn t(key: &str) -> &str {
    unsafe {
        if let Some(ref map) = TRANSLATIONS {
            if let Some(val) = map.get(&String::from(key)) {
                return val.as_str();
            }
        }
    }
    key
}

/// Returns the current language code (e.g. `"en"`, `"de"`, `"fr"`, `"it"`, `"gsw"`).
///
/// Returns `"en"` if `init()` has not been called.
pub fn lang() -> &'static str {
    unsafe {
        if let Some(ref l) = CURRENT_LANG {
            return l.as_str();
        }
    }
    "en"
}

// ── Internal helpers ─────────────────────────────────────────────────────

/// Detect language from environment variable or config file.
fn detect_language() -> String {
    // 1. Check LANG environment variable
    let mut buf = [0u8; 32];
    let len = env::get("LANG", &mut buf);
    if len != u32::MAX && len > 0 && len < 32 {
        if let Ok(s) = core::str::from_utf8(&buf[..len as usize]) {
            let s = s.trim();
            if !s.is_empty() {
                return String::from(s);
            }
        }
    }

    // 2. Check language.conf file
    if let Ok(content) = fs::read_to_string(LANG_CONF_PATH) {
        let s = content.trim();
        if !s.is_empty() {
            return String::from(s);
        }
    }

    // 3. Fallback
    String::from("en")
}

/// Load translations from a JSON file. Returns None if the file can't be
/// read or parsed, or if it contains no entries.
fn load_translations(path: &str) -> Option<HashMap<String, String>> {
    let content = fs::read_to_string(path).ok()?;
    let val = json::Value::parse(&content).ok()?;

    let obj = match &val {
        json::Value::Object(o) => o,
        _ => return None,
    };

    let mut map = HashMap::new();
    for (key, value) in obj.iter() {
        if let json::Value::String(s) = value {
            map.insert(String::from(key), String::from(s.as_str()));
        }
    }

    if map.len() == 0 {
        None
    } else {
        Some(map)
    }
}
