//! Git remote management.
//!
//! Remotes are stored in .git/config as [remote "name"] sections.

use crate::config;
use crate::repo::{Error, Repository, Result};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// A configured remote.
#[derive(Debug, Clone)]
pub struct Remote {
    pub name: String,
    pub url: String,
    pub fetch_refspec: String,
}

/// List all configured remotes.
pub fn list_remotes(repo: &Repository) -> Result<Vec<Remote>> {
    let cfg = config::read_config(repo)?;
    let mut remotes = Vec::new();
    let mut seen = Vec::new();

    for entry in &cfg.entries {
        if entry.section == "remote" {
            if let Some(ref name) = entry.subsection {
                if !seen.contains(name) {
                    seen.push(name.clone());
                    let url = cfg
                        .get_subsection("remote", name, "url")
                        .unwrap_or("")
                        .into();
                    let fetch = cfg
                        .get_subsection("remote", name, "fetch")
                        .unwrap_or(&format!("+refs/heads/*:refs/remotes/{}/*", name))
                        .into();
                    remotes.push(Remote {
                        name: name.clone(),
                        url,
                        fetch_refspec: fetch,
                    });
                }
            }
        }
    }

    Ok(remotes)
}

/// Get a remote by name.
pub fn get_remote(repo: &Repository, name: &str) -> Result<Remote> {
    let remotes = list_remotes(repo)?;
    remotes
        .into_iter()
        .find(|r| r.name == name)
        .ok_or(Error::NotFound)
}

/// Add a remote.
pub fn add_remote(repo: &Repository, name: &str, url: &str) -> Result<()> {
    let config_path = repo.gitdir.join("config");
    let mut content = std::fs::read_to_string(&config_path).unwrap_or_default();

    content.push_str(&format!(
        "[remote \"{}\"]\n\turl = {}\n\tfetch = +refs/heads/*:refs/remotes/{}/*\n",
        name, url, name
    ));

    std::fs::write(&config_path, content.as_bytes()).map_err(|_| Error::IoError)
}

/// Remove a remote.
pub fn remove_remote(repo: &Repository, name: &str) -> Result<()> {
    let config_path = repo.gitdir.join("config");
    let content = std::fs::read_to_string(&config_path).map_err(|_| Error::NotFound)?;

    let section_header = format!("[remote \"{}\"]", name);
    let mut result = String::new();
    let mut skip = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == section_header {
            skip = true;
            continue;
        }
        if skip && trimmed.starts_with('[') {
            skip = false;
        }
        if !skip {
            result.push_str(line);
            result.push('\n');
        }
    }

    std::fs::write(&config_path, result.as_bytes()).map_err(|_| Error::IoError)
}

/// Set the URL of a remote.
pub fn set_url(repo: &Repository, name: &str, url: &str) -> Result<()> {
    remove_remote(repo, name)?;
    add_remote(repo, name, url)
}

/// Parse a git URL into components.
#[derive(Debug, Clone)]
pub struct GitUrl {
    pub scheme: String,
    pub host: String,
    pub port: u16,
    pub path: String,
    pub user: Option<String>,
    pub password: Option<String>,
}

impl GitUrl {
    pub fn parse(url: &str) -> Option<Self> {
        let (scheme, rest) = if url.starts_with("https://") {
            ("https", &url[8..])
        } else if url.starts_with("http://") {
            ("http", &url[7..])
        } else if url.starts_with("git://") {
            ("git", &url[6..])
        } else {
            return None;
        };

        // Split user:pass@host:port/path
        let (auth, hostpath) = if let Some(at) = rest.find('@') {
            (Some(&rest[..at]), &rest[at + 1..])
        } else {
            (None, rest)
        };

        let (user, password) = if let Some(auth) = auth {
            if let Some(colon) = auth.find(':') {
                (
                    Some(String::from(&auth[..colon])),
                    Some(String::from(&auth[colon + 1..])),
                )
            } else {
                (Some(String::from(auth)), None)
            }
        } else {
            (None, None)
        };

        let (hostport, path) = if let Some(slash) = hostpath.find('/') {
            (&hostpath[..slash], &hostpath[slash..])
        } else {
            (hostpath, "/")
        };

        let (host, port) = if let Some(colon) = hostport.find(':') {
            let port_str = &hostport[colon + 1..];
            let port = port_str.parse::<u16>().unwrap_or(match scheme {
                "https" => 443,
                "git" => 9418,
                _ => 80,
            });
            (String::from(&hostport[..colon]), port)
        } else {
            (
                String::from(hostport),
                match scheme {
                    "https" => 443,
                    "git" => 9418,
                    _ => 80,
                },
            )
        };

        // Normalize host: strip www. prefix for known git hosts
        // to avoid 301 redirects that break POST requests
        let host = if host.starts_with("www.") {
            String::from(&host[4..])
        } else {
            host
        };

        // Ensure path ends with .git for GitHub-style hosting
        let path = String::from(path);
        let path = if !path.ends_with(".git") && !path.ends_with(".git/") {
            let trimmed = path.trim_end_matches('/');
            format!("{}.git", trimmed)
        } else {
            path
        };

        Some(GitUrl {
            scheme: String::from(scheme),
            host,
            port,
            path,
            user,
            password,
        })
    }

    /// Get the full URL for HTTP requests.
    pub fn to_http_url(&self) -> String {
        format!("{}://{}{}", self.scheme, self.host, self.path)
    }

    /// Get the info/refs URL for smart HTTP.
    pub fn info_refs_url(&self, service: &str) -> String {
        let base = self.to_http_url();
        let base = base.trim_end_matches('/');
        format!("{}/info/refs?service={}", base, service)
    }

    /// Get the service URL for smart HTTP.
    pub fn service_url(&self, service: &str) -> String {
        let base = self.to_http_url();
        let base = base.trim_end_matches('/');
        format!("{}/{}", base, service)
    }
}
