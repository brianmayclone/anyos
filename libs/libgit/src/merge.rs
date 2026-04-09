//! Git merge operations (fast-forward and basic three-way).

use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;
use crate::oid::Oid;
use crate::object::{Object, ObjectType, Commit};
use crate::repo::{Repository, Result, Error};

/// Merge result.
#[derive(Debug)]
pub enum MergeResult {
    /// Fast-forward: HEAD was updated to target.
    FastForward(Oid),
    /// Already up to date.
    UpToDate,
    /// Merge commit created.
    MergeCommit(Oid),
    /// Conflict detected.
    Conflict(Vec<String>),
}

/// Check if `ancestor` is an ancestor of `descendant`.
pub fn is_ancestor(repo: &Repository, ancestor: &Oid, descendant: &Oid) -> Result<bool> {
    if ancestor == descendant {
        return Ok(true);
    }

    // BFS through commit parents
    let mut queue = Vec::new();
    let mut visited = Vec::new();
    queue.push(*descendant);

    while let Some(current) = queue.pop() {
        if current == *ancestor {
            return Ok(true);
        }
        if visited.contains(&current) {
            continue;
        }
        visited.push(current);

        let obj = match repo.read_object(&current) {
            Ok(o) => o,
            Err(_) => continue,
        };
        if obj.obj_type != ObjectType::Commit {
            continue;
        }
        let commit = match Commit::parse(&obj.data) {
            Some(c) => c,
            None => continue,
        };
        for parent in &commit.parents {
            if !visited.contains(parent) {
                queue.push(*parent);
            }
        }
    }

    Ok(false)
}

/// Find the merge base (common ancestor) of two commits.
pub fn merge_base(repo: &Repository, a: &Oid, b: &Oid) -> Result<Option<Oid>> {
    if a == b {
        return Ok(Some(*a));
    }

    // Collect ancestors of A
    let ancestors_a = collect_ancestors(repo, a, 1000)?;

    // Walk ancestors of B and find first common
    let mut queue = alloc::vec![*b];
    let mut visited = Vec::new();

    while let Some(current) = queue.pop() {
        if ancestors_a.contains(&current) {
            return Ok(Some(current));
        }
        if visited.contains(&current) {
            continue;
        }
        visited.push(current);

        let obj = match repo.read_object(&current) {
            Ok(o) => o,
            Err(_) => continue,
        };
        if let Some(commit) = Commit::parse(&obj.data) {
            for parent in &commit.parents {
                if !visited.contains(parent) {
                    queue.push(*parent);
                }
            }
        }
    }

    Ok(None)
}

fn collect_ancestors(repo: &Repository, start: &Oid, max: usize) -> Result<Vec<Oid>> {
    let mut ancestors = Vec::new();
    let mut queue = alloc::vec![*start];

    while let Some(current) = queue.pop() {
        if ancestors.len() >= max || ancestors.contains(&current) {
            continue;
        }
        ancestors.push(current);

        let obj = match repo.read_object(&current) {
            Ok(o) => o,
            Err(_) => continue,
        };
        if let Some(commit) = Commit::parse(&obj.data) {
            for parent in &commit.parents {
                queue.push(*parent);
            }
        }
    }

    Ok(ancestors)
}

/// Perform a fast-forward merge.
/// Updates the current branch to point to `target_oid`.
pub fn fast_forward(repo: &Repository, target_oid: &Oid) -> Result<MergeResult> {
    let head_oid = repo.head().ok();

    // If HEAD doesn't exist, just set it
    if head_oid.is_none() {
        if let Ok(Some(branch)) = repo.current_branch() {
            crate::refs::update_ref(repo, &format!("refs/heads/{}", branch), target_oid)?;
        }
        return Ok(MergeResult::FastForward(*target_oid));
    }

    let head_oid = head_oid.unwrap();

    // Already up to date?
    if head_oid == *target_oid {
        return Ok(MergeResult::UpToDate);
    }

    // Check if HEAD is an ancestor of target (fast-forward possible)
    if is_ancestor(repo, &head_oid, target_oid)? {
        if let Ok(Some(branch)) = repo.current_branch() {
            crate::refs::update_ref(repo, &format!("refs/heads/{}", branch), target_oid)?;
        }
        return Ok(MergeResult::FastForward(*target_oid));
    }

    // Check reverse — target is ancestor of HEAD
    if is_ancestor(repo, target_oid, &head_oid)? {
        return Ok(MergeResult::UpToDate);
    }

    // Non-fast-forward merge needed — not yet implemented
    Err(Error::MergeConflict)
}
