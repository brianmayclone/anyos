//! Working tree checkout — materialize a tree into the filesystem.

use crate::object::ObjectType;
use crate::oid::Oid;
use crate::repo::{Error, Repository, Result};
use crate::tree;
use alloc::format;

/// Checkout a tree object into the working directory.
pub fn checkout_tree(repo: &Repository, tree_oid: &Oid) -> Result<u32> {
    checkout_tree_recursive(repo, tree_oid, "")
}

fn checkout_tree_recursive(repo: &Repository, tree_oid: &Oid, prefix: &str) -> Result<u32> {
    let obj = repo.read_object_any(tree_oid).map_err(|_| {
        Error::Other(format!(
            "missing tree object {} at {}",
            tree_oid.to_hex(),
            if prefix.is_empty() { "." } else { prefix }
        ))
    })?;
    if obj.obj_type != ObjectType::Tree {
        return Err(Error::InvalidObject);
    }

    let entries = tree::parse_tree(&obj.data);
    let mut count = 0u32;

    for entry in &entries {
        let path = if prefix.is_empty() {
            entry.name.clone()
        } else {
            format!("{}/{}", prefix, entry.name)
        };

        if entry.is_tree() {
            // Create directory and recurse
            let dir_path = repo.workdir_path(&path);
            let _ = std::fs::create_dir_all(&dir_path);
            count += checkout_tree_recursive(repo, &entry.oid, &path)?;
        } else if entry.is_gitlink() {
            // Submodule gitlinks point at a commit object that is not part of
            // this repository's object database. Materialize an empty directory
            // like a fresh Git checkout with uninitialized submodules.
            let dir_path = repo.workdir_path(&path);
            let _ = std::fs::create_dir_all(&dir_path);
            count += 1;
        } else {
            // Write file
            let blob = repo.read_object_any(&entry.oid).map_err(|_| {
                Error::Other(format!(
                    "missing blob object {} for {}",
                    entry.oid.to_hex(),
                    path
                ))
            })?;
            let file_path = repo.workdir_path(&path);

            // Ensure parent directory exists
            if let Some(parent) = file_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }

            std::fs::write(&file_path, &blob.data).map_err(|_| Error::IoError)?;

            // Set executable permission if needed
            if entry.is_executable() {
                #[cfg(feature = "host")]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(
                        &file_path,
                        std::fs::Permissions::from_mode(0o755),
                    );
                }
                #[cfg(not(feature = "host"))]
                let _ = std::fs::set_permissions(&file_path, std::fs::Permissions { mode: 0o755 });
            }

            count += 1;
        }
    }

    Ok(count)
}

/// Build index from a tree (used after clone/checkout).
pub fn build_index_from_tree(repo: &Repository, tree_oid: &Oid) -> Result<crate::index::Index> {
    let mut index = crate::index::Index::new();
    build_index_recursive(repo, tree_oid, "", &mut index)?;
    Ok(index)
}

fn build_index_recursive(
    repo: &Repository,
    tree_oid: &Oid,
    prefix: &str,
    index: &mut crate::index::Index,
) -> Result<()> {
    let obj = repo.read_object_any(tree_oid).map_err(|_| {
        Error::Other(format!(
            "missing tree object {} while indexing {}",
            tree_oid.to_hex(),
            if prefix.is_empty() { "." } else { prefix }
        ))
    })?;
    if obj.obj_type != ObjectType::Tree {
        return Err(Error::InvalidObject);
    }

    let entries = tree::parse_tree(&obj.data);

    for entry in &entries {
        let path = if prefix.is_empty() {
            entry.name.clone()
        } else {
            format!("{}/{}", prefix, entry.name)
        };

        if entry.is_tree() {
            build_index_recursive(repo, &entry.oid, &path, index)?;
        } else if entry.is_gitlink() {
            let index_entry = crate::index::IndexEntry::new(&path, entry.oid, entry.mode, 0);
            index.add(index_entry);
        } else {
            // Read blob to get its size
            let blob = repo.read_object_any(&entry.oid).map_err(|_| {
                Error::Other(format!(
                    "missing blob object {} while indexing {}",
                    entry.oid.to_hex(),
                    path
                ))
            })?;
            let index_entry =
                crate::index::IndexEntry::new(&path, entry.oid, entry.mode, blob.data.len() as u32);
            index.add(index_entry);
        }
    }

    Ok(())
}

/// Reset working tree to match HEAD.
pub fn checkout_head(repo: &Repository) -> Result<u32> {
    let head_oid = repo.head()?;
    let commit_obj = repo
        .read_object_any(&head_oid)
        .map_err(|_| Error::Other(format!("missing HEAD commit {}", head_oid.to_hex())))?;
    let commit = crate::object::Commit::parse(&commit_obj.data).ok_or(Error::InvalidObject)?;
    let old_index = crate::index::Index::read(repo).unwrap_or_else(|_| crate::index::Index::new());

    let count = checkout_tree(repo, &commit.tree)?;

    // Update index to match
    let index = build_index_from_tree(repo, &commit.tree)?;
    remove_tracked_paths_not_in_target(repo, &old_index, &index);
    let entry_count = index.entries.len();
    index.write(repo)?;
    let reread_index = crate::index::Index::read(repo)?;
    if reread_index.entries.len() != entry_count {
        return Err(Error::Other(format!(
            "index verification failed after checkout: wrote {} entries, read {}",
            entry_count,
            reread_index.entries.len()
        )));
    }

    Ok(count)
}

fn remove_tracked_paths_not_in_target(
    repo: &Repository,
    old_index: &crate::index::Index,
    new_index: &crate::index::Index,
) {
    for entry in &old_index.entries {
        if new_index.find(&entry.name).is_some() || !is_safe_relative_path(&entry.name) {
            continue;
        }

        let path = repo.workdir_path(&entry.name);
        let should_remove = match std::fs::read(&path) {
            Ok(data) => crate::object::Object::blob(data).id() == entry.oid,
            Err(_) => false,
        };
        if should_remove {
            let _ = std::fs::remove_file(&path);
            remove_empty_parent_dirs(repo, &entry.name);
        }
    }
}

fn remove_empty_parent_dirs(repo: &Repository, path: &str) {
    let mut current = repo.workdir_path(path);
    while current.pop() && current != repo.workdir {
        match std::fs::remove_dir(&current) {
            Ok(_) => {}
            Err(_) => break,
        }
    }
}

fn is_safe_relative_path(path: &str) -> bool {
    let p = std::path::Path::new(path);
    !p.is_absolute()
        && p.components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}
