//! Git reference management (HEAD, branches, tags).
//!
//! References are stored as plain text files containing SHA-1 hex strings,
//! or as symbolic refs ("ref: refs/heads/main").

use crate::oid::Oid;
use crate::repo::{Error, Repository, Result};
use alloc::string::String;
use alloc::vec::Vec;
use std::path::PathBuf;

/// Resolve a reference name to an OID (follows symbolic refs).
pub fn resolve_ref(repo: &Repository, refname: &str) -> Result<Oid> {
    resolve_ref_depth(repo, refname, 0)
}

fn resolve_ref_depth(repo: &Repository, refname: &str, depth: u32) -> Result<Oid> {
    if depth > 10 {
        return Err(Error::InvalidRef);
    }

    let path = if refname == "HEAD" || refname.starts_with("refs/") {
        repo.gitdir.join(refname)
    } else {
        // Try as branch name
        repo.gitdir.join("refs/heads").join(refname)
    };

    let content = std::fs::read_to_string(&path).map_err(|_| Error::InvalidRef)?;
    let trimmed = content.trim();

    if let Some(target) = trimmed.strip_prefix("ref: ") {
        resolve_ref_depth(repo, target, depth + 1)
    } else {
        Oid::from_hex(trimmed).ok_or(Error::InvalidRef)
    }
}

/// Update a reference to point to a new OID.
pub fn update_ref(repo: &Repository, refname: &str, oid: &Oid) -> Result<()> {
    let path = if refname.starts_with("refs/") {
        repo.gitdir.join(refname)
    } else {
        repo.gitdir.join("refs/heads").join(refname)
    };

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let content = alloc::format!("{}\n", oid.to_hex());
    std::fs::write(&path, content.as_bytes()).map_err(|_| Error::IoError)
}

/// Update HEAD to point to a branch or OID.
pub fn update_head(repo: &Repository, target: &str) -> Result<()> {
    let content = if target.starts_with("refs/") {
        alloc::format!("ref: {}\n", target)
    } else {
        // Assume it's a branch name
        alloc::format!("ref: refs/heads/{}\n", target)
    };
    std::fs::write(&repo.gitdir.join("HEAD"), content.as_bytes()).map_err(|_| Error::IoError)
}

/// Update HEAD to a detached state pointing to an OID.
pub fn detach_head(repo: &Repository, oid: &Oid) -> Result<()> {
    let content = alloc::format!("{}\n", oid.to_hex());
    std::fs::write(&repo.gitdir.join("HEAD"), content.as_bytes()).map_err(|_| Error::IoError)
}

/// List all references with their OIDs.
pub fn list_refs(repo: &Repository) -> Result<Vec<(String, Oid)>> {
    let mut result = Vec::new();
    list_refs_recursive(repo, &repo.gitdir.join("refs"), "refs", &mut result)?;
    Ok(result)
}

fn list_refs_recursive(
    repo: &Repository,
    dir: &std::path::Path,
    prefix: &str,
    result: &mut Vec<(String, Oid)>,
) -> Result<()> {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries {
            if let Ok(entry) = entry {
                let name = match entry.file_name().to_str() {
                    Some(n) => String::from(n),
                    None => continue,
                };
                let refname = alloc::format!("{}/{}", prefix, name);
                let path = dir.join(&*name);

                if path.is_dir() {
                    list_refs_recursive(repo, &path, &refname, result)?;
                } else if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Some(oid) = Oid::from_hex(content.trim()) {
                        result.push((refname, oid));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Delete a reference.
pub fn delete_ref(repo: &Repository, refname: &str) -> Result<()> {
    let path = repo.gitdir.join(refname);
    std::fs::remove_file(&path).map_err(|_| Error::NotFound)
}
