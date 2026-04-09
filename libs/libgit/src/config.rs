//! Git config file parsing (.git/config, ~/.gitconfig).
//!
//! Simple INI-style parser for git configuration.

use alloc::string::String;
use alloc::vec::Vec;
use crate::repo::{Repository, Result, Error};

/// A config key-value pair.
#[derive(Debug, Clone)]
pub struct ConfigEntry {
    pub section: String,
    pub subsection: Option<String>,
    pub key: String,
    pub value: String,
}

/// Parsed git config.
#[derive(Debug)]
pub struct Config {
    pub entries: Vec<ConfigEntry>,
}

impl Config {
    /// Parse a git config string.
    pub fn parse(text: &str) -> Self {
        let mut entries = Vec::new();
        let mut section = String::new();
        let mut subsection: Option<String> = None;

        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
                continue;
            }

            if trimmed.starts_with('[') {
                if let Some(end) = trimmed.find(']') {
                    let header = &trimmed[1..end];
                    if let Some(space) = header.find(' ') {
                        section = String::from(header[..space].trim());
                        let sub = header[space + 1..].trim().trim_matches('"');
                        subsection = Some(String::from(sub));
                    } else {
                        section = String::from(header.trim());
                        subsection = None;
                    }
                }
                continue;
            }

            if let Some(eq) = trimmed.find('=') {
                let key = trimmed[..eq].trim();
                let value = trimmed[eq + 1..].trim();
                entries.push(ConfigEntry {
                    section: section.clone(),
                    subsection: subsection.clone(),
                    key: String::from(key),
                    value: String::from(value),
                });
            }
        }

        Config { entries }
    }

    /// Get a config value by section and key.
    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.entries.iter()
            .rev()
            .find(|e| e.section == section && e.key == key)
            .map(|e| e.value.as_str())
    }

    /// Get a config value with subsection.
    pub fn get_subsection(&self, section: &str, subsection: &str, key: &str) -> Option<&str> {
        self.entries.iter()
            .rev()
            .find(|e| {
                e.section == section
                    && e.subsection.as_deref() == Some(subsection)
                    && e.key == key
            })
            .map(|e| e.value.as_str())
    }

    /// Get user name from config.
    pub fn user_name(&self) -> Option<&str> {
        self.get("user", "name")
    }

    /// Get user email from config.
    pub fn user_email(&self) -> Option<&str> {
        self.get("user", "email")
    }
}

/// Read the repository config.
pub fn read_config(repo: &Repository) -> Result<Config> {
    let path = repo.gitdir.join("config");
    let text = std::fs::read_to_string(&path).map_err(|_| Error::NotFound)?;
    Ok(Config::parse(&text))
}

/// Read the global git config (~/.gitconfig).
pub fn read_global_config() -> Option<Config> {
    // Try $HOME/.gitconfig
    let mut home_buf = [0u8; 256];
    let len = anyos_std::env::get("HOME", &mut home_buf);
    if len == 0 || len == u32::MAX {
        return None;
    }
    let home = core::str::from_utf8(&home_buf[..len as usize]).ok()?;
    let path = alloc::format!("{}/.gitconfig", home);
    let text = std::fs::read_to_string(std::path::Path::new(&path)).ok()?;
    Some(Config::parse(&text))
}
