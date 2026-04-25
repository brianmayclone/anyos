use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use libgit::ignore::GitIgnore;
use libgit::tree::parse_tree;
use libgit::{Commit, Index, Object, Oid, Repository};

pub fn print_porcelain(repo: &Repository) {
    let index = Index::read(repo).unwrap_or_else(|_| Index::new());
    let head_tree = head_tree_flat(repo);
    let gitignore = GitIgnore::load(repo).with_defaults();

    for entry in &index.entries {
        match head_tree.iter().find(|(name, _)| name == &entry.name) {
            Some((_, oid)) if *oid != entry.oid => {
                anyos_std::println!("M  {}", entry.name);
            }
            None => {
                anyos_std::println!("A  {}", entry.name);
            }
            _ => {}
        }
    }

    for (name, _) in &head_tree {
        if index.find(name).is_none() {
            anyos_std::println!("D  {}", name);
        }
    }

    for entry in &index.entries {
        let full_path = repo.workdir_path(&entry.name);
        if let Ok(data) = std::fs::read(&full_path) {
            let obj = Object::blob(data);
            if obj.id() != entry.oid {
                anyos_std::println!(" M {}", entry.name);
            }
        } else {
            anyos_std::println!(" D {}", entry.name);
        }
    }

    let mut untracked: Vec<String> = Vec::new();
    collect_untracked(repo, &index, ".", &mut untracked, &gitignore);
    for path in &untracked {
        anyos_std::println!("?? {}", path);
    }
}

pub fn collect_untracked(
    repo: &Repository,
    index: &Index,
    dir: &str,
    out: &mut Vec<String>,
    gitignore: &GitIgnore,
) {
    let full_path = repo.workdir_path(dir);
    if let Ok(entries) = std::fs::read_dir(&full_path) {
        for entry in entries {
            if let Ok(entry) = entry {
                let name = match entry.file_name().to_str() {
                    Some(n) => String::from(n),
                    None => continue,
                };
                let rel_path = if dir == "." {
                    name
                } else {
                    format!("{}/{}", dir, name)
                };
                if let Ok(ft) = entry.file_type() {
                    if gitignore.is_ignored(&rel_path, ft.is_dir()) {
                        continue;
                    }
                    if ft.is_dir() {
                        if index_has_prefix(index, &rel_path) {
                            collect_untracked(repo, index, &rel_path, out, gitignore);
                        } else {
                            out.push(format!("{}/", rel_path));
                        }
                    } else if ft.is_file() && index.find(&rel_path).is_none() {
                        out.push(rel_path);
                    }
                }
            }
        }
    }
}

fn index_has_prefix(index: &Index, dir: &str) -> bool {
    let prefix = format!("{}/", dir);
    index
        .entries
        .iter()
        .any(|entry| entry.name.starts_with(&prefix))
}

pub fn head_tree_flat(repo: &Repository) -> Vec<(String, Oid)> {
    let mut out = Vec::new();
    let head_oid = match repo.head() {
        Ok(o) => o,
        Err(_) => return out,
    };
    let commit_obj = match repo.read_object(&head_oid) {
        Ok(o) => o,
        Err(_) => return out,
    };
    let commit = match Commit::parse(&commit_obj.data) {
        Some(c) => c,
        None => return out,
    };
    collect_tree_entries(repo, &commit.tree, "", &mut out);
    out
}

fn collect_tree_entries(
    repo: &Repository,
    tree_oid: &Oid,
    prefix: &str,
    out: &mut Vec<(String, Oid)>,
) {
    let tree_obj = match repo.read_object(tree_oid) {
        Ok(o) => o,
        Err(_) => return,
    };
    for entry in parse_tree(&tree_obj.data) {
        let path = if prefix.is_empty() {
            entry.name.clone()
        } else {
            format!("{}/{}", prefix, entry.name)
        };
        if entry.is_tree() {
            collect_tree_entries(repo, &entry.oid, &path, out);
        } else {
            out.push((path, entry.oid));
        }
    }
}
