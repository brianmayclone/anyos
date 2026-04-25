#![cfg_attr(not(feature = "host"), no_std)]
#![cfg_attr(not(feature = "host"), no_main)]

extern crate alloc;
extern crate std;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use libgit::ignore::GitIgnore;
use libgit::index::IndexEntry;
use libgit::tree::parse_tree;
use libgit::{Commit, Index, Object, ObjectType, Oid, Repository};

mod cli;
mod status;

#[cfg(not(feature = "host"))]
anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let raw_args = cli::normalize_raw_args(raw);
    let tokens = anyos_std::args::tokenize(&raw_args);
    let args = anyos_std::args::parse(&raw_args, b"mnub");

    // -v flag enables verbose output
    if args.has(b'v') {
        libgit::pack::set_verbose(true);
    }

    if args.pos_count == 0 {
        print_usage();
        return;
    }

    let cmd = args.positional[0];

    match cmd {
        "init" => cmd_init(&args),
        "clone" => cmd_clone(&args),
        "add" => cmd_add(&args),
        "status" => cmd_status(&tokens),
        "commit" => cmd_commit(&args),
        "log" => cmd_log(&args),
        "diff" => cmd_diff(),
        "branch" => cmd_branch(&args, &tokens),
        "checkout" => cmd_checkout(&args),
        "remote" => cmd_remote(&args),
        "fetch" => cmd_fetch(&args),
        "pull" => cmd_pull(&args),
        "push" => cmd_push(&args),
        "config" => cmd_config(&args),
        "tag" => cmd_tag(&args),
        "rm" => cmd_rm(&args),
        "reset" => cmd_reset(&args),
        "show" => cmd_show(&args),
        "rev-parse" => cmd_rev_parse(&args),
        "hash-object" => cmd_hash_object(&args),
        "cat-file" => cmd_cat_file(&args),
        _ => anyos_std::println!("git: '{}' is not a git command. See 'git --help'.", cmd),
    }
}

fn print_usage() {
    anyos_std::println!("usage: git <command> [<args>]");
    anyos_std::println!();
    anyos_std::println!("Repository commands:");
    anyos_std::println!("   init          Create an empty Git repository");
    anyos_std::println!("   clone         Clone a repository");
    anyos_std::println!();
    anyos_std::println!("Working tree commands:");
    anyos_std::println!("   add           Add file contents to the index");
    anyos_std::println!("   rm            Remove files from the working tree and index");
    anyos_std::println!("   status        Show the working tree status");
    anyos_std::println!("   diff          Show changes between commits, index, and working tree");
    anyos_std::println!("   checkout      Switch branches or restore working tree files");
    anyos_std::println!("   reset         Reset current HEAD to the specified state");
    anyos_std::println!();
    anyos_std::println!("History commands:");
    anyos_std::println!("   commit        Record changes to the repository");
    anyos_std::println!("   log           Show commit logs");
    anyos_std::println!("   show          Show a commit");
    anyos_std::println!("   tag           Create, list, or delete tags");
    anyos_std::println!();
    anyos_std::println!("Branch commands:");
    anyos_std::println!("   branch        List, create, or delete branches");
    anyos_std::println!();
    anyos_std::println!("Remote commands:");
    anyos_std::println!("   remote        Manage remote repositories");
    anyos_std::println!("   fetch         Download objects and refs from a remote");
    anyos_std::println!("   pull          Fetch and integrate with local branch");
    anyos_std::println!("   push          Update remote refs and objects");
    anyos_std::println!();
    anyos_std::println!("Low-level commands:");
    anyos_std::println!("   config        Get and set repository or global options");
    anyos_std::println!("   rev-parse     Parse revision identifiers");
    anyos_std::println!("   hash-object   Compute object ID from a file");
    anyos_std::println!("   cat-file      Show object content, type, or size");
}

// ── git init ────────────────────────────────────────────────────────────────

fn cmd_init(args: &anyos_std::args::ParsedArgs) {
    let path = args.pos(1).unwrap_or(".");
    match Repository::init(path) {
        Ok(repo) => {
            anyos_std::println!(
                "Initialized empty Git repository in {}",
                repo.gitdir.display()
            );
        }
        Err(e) => anyos_std::println!("fatal: {}", e),
    }
}

// ── git clone ───────────────────────────────────────────────────────────────

fn cmd_clone(args: &anyos_std::args::ParsedArgs) {
    if args.pos_count < 2 {
        anyos_std::println!("usage: git clone <url> [<directory>]");
        return;
    }

    let url_str = args.positional[1];

    // Determine target directory from URL
    let target_dir = if args.pos_count >= 3 {
        String::from(args.positional[2])
    } else {
        // Extract repo name from URL
        let name = url_str.trim_end_matches('/').trim_end_matches(".git");
        let name = name.rsplit('/').next().unwrap_or("repo");
        String::from(name)
    };

    anyos_std::println!("Cloning into '{}'...", target_dir);

    let url = match libgit::remote::GitUrl::parse(url_str) {
        Some(u) => u,
        None => {
            anyos_std::println!("fatal: invalid URL: {}", url_str);
            return;
        }
    };

    // Step 1: Discover refs
    let (remote_refs, caps) = match libgit::transport::discover_refs(&url, "git-upload-pack") {
        Ok(r) => r,
        Err(e) => {
            anyos_std::println!("fatal: {}", e);
            return;
        }
    };

    if remote_refs.is_empty() {
        anyos_std::println!("warning: remote has no refs");
    }

    // Step 2: Initialize local repository
    let repo = match Repository::init(&target_dir) {
        Ok(r) => r,
        Err(e) => {
            anyos_std::println!("fatal: {}", e);
            return;
        }
    };

    // Step 3: Add remote
    if let Err(e) = libgit::remote::add_remote(&repo, "origin", url_str) {
        anyos_std::println!("warning: failed to add remote: {}", e);
    }

    // Step 4: Determine which refs to fetch (all heads + HEAD)
    let wants: Vec<Oid> = remote_refs
        .iter()
        .filter(|r| r.name.starts_with("refs/heads/") || r.name == "HEAD")
        .map(|r| r.oid)
        .collect();

    if wants.is_empty() {
        anyos_std::println!("warning: no branches to fetch");
        return;
    }

    // Deduplicate wants
    let mut unique_wants = Vec::new();
    for w in &wants {
        if !unique_wants.contains(w) {
            unique_wants.push(*w);
        }
    }

    // Step 5+6: Stream pack data directly into repository (no buffering)
    anyos_std::println!("remote: Counting objects...");
    match libgit::transport::fetch_pack_streamed(&url, &unique_wants, &[], &repo) {
        Ok(_n) => {}
        Err(e) => {
            anyos_std::println!("fatal: {}", e);
            return;
        }
    }

    // Step 7: Update remote refs
    for rref in &remote_refs {
        if rref.name.starts_with("refs/heads/") {
            let local_ref = rref.name.replace("refs/heads/", "refs/remotes/origin/");
            let _ = libgit::refs::update_ref(&repo, &local_ref, &rref.oid);
        }
    }

    // Step 8: Determine HEAD branch
    let head_ref = caps.symref_head.as_deref().unwrap_or("refs/heads/main");
    let head_branch = head_ref.strip_prefix("refs/heads/").unwrap_or("main");

    // Find HEAD commit
    let head_oid = remote_refs
        .iter()
        .find(|r| r.name == head_ref || r.name == "HEAD")
        .map(|r| r.oid);

    if let Some(oid) = head_oid {
        // Update local branch
        let local_branch_ref = format!("refs/heads/{}", head_branch);
        let _ = libgit::refs::update_ref(&repo, &local_branch_ref, &oid);

        // Set HEAD to point to the branch
        let _ = libgit::refs::update_head(&repo, head_branch);

        // Step 9: Checkout working tree
        match libgit::checkout::checkout_head(&repo) {
            Ok(n) => anyos_std::println!("Resolving deltas, checking out files ({})...", n),
            Err(e) => anyos_std::println!("warning: checkout failed: {}", e),
        }
    }

    anyos_std::println!("done.");
}

// ── git add ─────────────────────────────────────────────────────────────────

fn cmd_add(args: &anyos_std::args::ParsedArgs) {
    if args.pos_count < 2 {
        anyos_std::println!("usage: git add <file>...");
        return;
    }

    let repo = match Repository::open(".") {
        Ok(r) => r,
        Err(e) => {
            anyos_std::println!("fatal: {}", e);
            return;
        }
    };

    let gitignore = GitIgnore::load(&repo).with_defaults();
    let mut index = Index::read(&repo).unwrap_or_else(|_| Index::new());

    for i in 1..args.pos_count {
        let path = args.positional[i];
        if path == "." || path == "-A" {
            add_directory(&repo, &mut index, ".", &gitignore);
        } else {
            add_file(&repo, &mut index, path);
        }
    }

    if let Err(e) = index.write(&repo) {
        anyos_std::println!("fatal: failed to write index: {}", e);
    }
}

fn add_file(repo: &Repository, index: &mut Index, path: &str) {
    let full_path = repo.workdir_path(path);
    let data = match std::fs::read(&full_path) {
        Ok(d) => d,
        Err(_) => {
            // File was deleted — remove from index
            index.remove(path);
            return;
        }
    };

    let obj = Object::blob(data.clone());
    match repo.write_object(&obj) {
        Ok(oid) => {
            let mode = if is_executable(&full_path) {
                0o100755
            } else {
                0o100644
            };
            let entry = IndexEntry::new(path, oid, mode, data.len() as u32);
            index.add(entry);
        }
        Err(e) => anyos_std::println!("error: {}", e),
    }
}

fn add_directory(repo: &Repository, index: &mut Index, dir: &str, gitignore: &GitIgnore) {
    let full_path = repo.workdir_path(dir);
    if let Ok(entries) = std::fs::read_dir(&full_path) {
        for entry in entries {
            if let Ok(entry) = entry {
                let name = match entry.file_name().to_str() {
                    Some(n) => String::from(n),
                    None => continue,
                };

                let rel_path = if dir == "." {
                    name.clone()
                } else {
                    format!("{}/{}", dir, name)
                };

                let ft = match entry.file_type() {
                    Ok(ft) => ft,
                    Err(_) => continue,
                };

                if gitignore.is_ignored(&rel_path, ft.is_dir()) {
                    continue;
                }

                if ft.is_dir() {
                    add_directory(repo, index, &rel_path, gitignore);
                } else if ft.is_file() {
                    add_file(repo, index, &rel_path);
                }
            }
        }
    }
}

fn is_executable(_path: &std::path::Path) -> bool {
    // anyOS doesn't have Unix-style executable bits in the same way
    false
}

// ── git status ──────────────────────────────────────────────────────────────

fn cmd_status(tokens: &[String]) {
    let repo = match Repository::open(".") {
        Ok(r) => r,
        Err(e) => {
            anyos_std::println!("fatal: {}", e);
            return;
        }
    };

    if cli::has_long_option(tokens, "--porcelain") || cli::has_long_option(tokens, "--porcelain=v1")
    {
        status::print_porcelain(&repo);
        return;
    }

    match repo.current_branch() {
        Ok(Some(branch)) => anyos_std::println!("On branch {}", branch),
        Ok(None) => anyos_std::println!("HEAD detached"),
        Err(_) => anyos_std::println!("On branch (unknown)"),
    }

    let index = Index::read(&repo).unwrap_or_else(|_| Index::new());
    let head_tree = status::head_tree_flat(&repo);
    let gitignore = GitIgnore::load(&repo).with_defaults();

    let mut staged_new: Vec<String> = Vec::new();
    let mut staged_modified: Vec<String> = Vec::new();
    let mut staged_deleted: Vec<String> = Vec::new();

    for entry in &index.entries {
        match head_tree.iter().find(|(n, _)| n == &entry.name) {
            Some((_, oid)) if *oid != entry.oid => staged_modified.push(entry.name.clone()),
            None => staged_new.push(entry.name.clone()),
            _ => {}
        }
    }
    for (name, _) in &head_tree {
        if index.find(name).is_none() {
            staged_deleted.push(name.clone());
        }
    }

    if !staged_new.is_empty() || !staged_modified.is_empty() || !staged_deleted.is_empty() {
        anyos_std::println!("\nChanges to be committed:");
        anyos_std::println!("  (use \"git reset HEAD <file>...\" to unstage)");
        for f in &staged_new {
            anyos_std::println!("\tnew file:   {}", f);
        }
        for f in &staged_modified {
            anyos_std::println!("\tmodified:   {}", f);
        }
        for f in &staged_deleted {
            anyos_std::println!("\tdeleted:    {}", f);
        }
    }

    let mut modified: Vec<String> = Vec::new();
    let mut deleted: Vec<String> = Vec::new();
    for entry in &index.entries {
        let full_path = repo.workdir_path(&entry.name);
        if let Ok(data) = std::fs::read(&full_path) {
            let obj = Object::blob(data);
            if obj.id() != entry.oid {
                modified.push(entry.name.clone());
            }
        } else {
            deleted.push(entry.name.clone());
        }
    }

    if !modified.is_empty() || !deleted.is_empty() {
        anyos_std::println!("\nChanges not staged for commit:");
        anyos_std::println!("  (use \"git add <file>...\" to update what will be committed)");
        for f in &modified {
            anyos_std::println!("\tmodified:   {}", f);
        }
        for f in &deleted {
            anyos_std::println!("\tdeleted:    {}", f);
        }
    }

    let mut untracked: Vec<String> = Vec::new();
    status::collect_untracked(&repo, &index, ".", &mut untracked, &gitignore);
    if !untracked.is_empty() {
        anyos_std::println!("\nUntracked files:");
        anyos_std::println!("  (use \"git add <file>...\" to include in what will be committed)");
        for f in &untracked {
            anyos_std::println!("\t{}", f);
        }
    }

    if staged_new.is_empty()
        && staged_modified.is_empty()
        && staged_deleted.is_empty()
        && modified.is_empty()
        && deleted.is_empty()
        && untracked.is_empty()
    {
        anyos_std::println!("\nnothing to commit, working tree clean");
    }
}

// ── git commit ──────────────────────────────────────────────────────────────

fn cmd_commit(args: &anyos_std::args::ParsedArgs) {
    let repo = match Repository::open(".") {
        Ok(r) => r,
        Err(e) => {
            anyos_std::println!("fatal: {}", e);
            return;
        }
    };

    let message = match args.opt(b'm') {
        Some(m) => String::from(m),
        None => {
            anyos_std::println!("error: switch 'm' requires a value");
            return;
        }
    };

    let index = match Index::read(&repo) {
        Ok(idx) => idx,
        Err(e) => {
            anyos_std::println!("fatal: {}", e);
            return;
        }
    };

    if index.entries.is_empty() {
        anyos_std::println!("nothing to commit");
        return;
    }

    // Build recursive tree from index
    let tree_oid = match repo.write_tree_recursive(&index) {
        Ok(oid) => oid,
        Err(e) => {
            anyos_std::println!("fatal: {}", e);
            return;
        }
    };

    let parents = match repo.head() {
        Ok(oid) => alloc::vec![oid],
        Err(_) => Vec::new(),
    };

    let (name, email) = get_user_info(&repo);
    let timestamp = get_timestamp();
    let author_line = format!("{} <{}> {} +0000", name, email, timestamp);

    let commit = Commit {
        tree: tree_oid,
        parents,
        author: author_line.clone(),
        committer: author_line,
        message: message.clone(),
    };

    let obj = Object::commit(commit.serialize());
    let commit_oid = match repo.write_object(&obj) {
        Ok(oid) => oid,
        Err(e) => {
            anyos_std::println!("fatal: {}", e);
            return;
        }
    };

    // Update ref
    match repo.current_branch() {
        Ok(Some(branch)) => {
            let _ = libgit::refs::update_ref(&repo, &format!("refs/heads/{}", branch), &commit_oid);
        }
        _ => {
            let _ = libgit::refs::detach_head(&repo, &commit_oid);
        }
    }

    let is_root = commit.parents.is_empty();
    let branch = repo
        .current_branch()
        .ok()
        .flatten()
        .unwrap_or_else(|| String::from("HEAD"));
    let file_count = index.entries.len();

    anyos_std::println!(
        "[{} {}] {}",
        if is_root {
            format!("{} (root-commit)", branch)
        } else {
            branch
        },
        commit_oid.short(),
        message,
    );
    anyos_std::println!(
        " {} file{} changed",
        file_count,
        if file_count != 1 { "s" } else { "" }
    );
}

// ── git log ─────────────────────────────────────────────────────────────────

fn cmd_log(args: &anyos_std::args::ParsedArgs) {
    let repo = match Repository::open(".") {
        Ok(r) => r,
        Err(e) => {
            anyos_std::println!("fatal: {}", e);
            return;
        }
    };

    let max_count = args.opt_u32(b'n', 20) as usize;

    let mut oid = match repo.head() {
        Ok(oid) => oid,
        Err(_) => {
            anyos_std::println!("fatal: your current branch does not have any commits yet");
            return;
        }
    };

    for _ in 0..max_count {
        let obj = match repo.read_object(&oid) {
            Ok(o) => o,
            Err(_) => break,
        };
        if obj.obj_type != ObjectType::Commit {
            break;
        }
        let commit = match Commit::parse(&obj.data) {
            Some(c) => c,
            None => break,
        };

        anyos_std::println!("\x1b[33mcommit {}\x1b[0m", oid.to_hex());
        anyos_std::println!("Author: {}", commit.author);
        anyos_std::println!();
        for line in commit.message.lines() {
            anyos_std::println!("    {}", line);
        }
        anyos_std::println!();

        if let Some(parent) = commit.parents.first() {
            oid = *parent;
        } else {
            break;
        }
    }
}

// ── git diff ────────────────────────────────────────────────────────────────

fn cmd_diff() {
    let repo = match Repository::open(".") {
        Ok(r) => r,
        Err(e) => {
            anyos_std::println!("fatal: {}", e);
            return;
        }
    };

    let index = Index::read(&repo).unwrap_or_else(|_| Index::new());

    for entry in &index.entries {
        let full_path = repo.workdir_path(&entry.name);
        let workdir_data = match std::fs::read(&full_path) {
            Ok(d) => d,
            Err(_) => continue,
        };
        let workdir_obj = Object::blob(workdir_data.clone());
        if workdir_obj.id() == entry.oid {
            continue;
        }

        let index_data = match repo.read_object(&entry.oid) {
            Ok(o) => o.data,
            Err(_) => continue,
        };
        let old_text = core::str::from_utf8(&index_data).unwrap_or("");
        let new_text = core::str::from_utf8(&workdir_data).unwrap_or("");
        let hunks = libgit::diff::unified_diff(old_text, new_text, 3);
        if !hunks.is_empty() {
            anyos_std::print!("{}", libgit::diff::format_diff(&entry.name, &hunks));
        }
    }
}

// ── git branch ──────────────────────────────────────────────────────────────

fn cmd_branch(args: &anyos_std::args::ParsedArgs, tokens: &[String]) {
    let repo = match Repository::open(".") {
        Ok(r) => r,
        Err(e) => {
            anyos_std::println!("fatal: {}", e);
            return;
        }
    };

    if cli::has_long_option(tokens, "--show-current") {
        if let Ok(Some(branch)) = repo.current_branch() {
            anyos_std::println!("{}", branch);
        }
        return;
    }

    if args.pos_count < 2 {
        let current = repo.current_branch().ok().flatten();
        if let Ok(branches) = repo.branches() {
            for branch in branches {
                let prefix = if current.as_deref() == Some(&branch) {
                    "\x1b[32m* "
                } else {
                    "  "
                };
                let suffix = if current.as_deref() == Some(&branch) {
                    "\x1b[0m"
                } else {
                    ""
                };
                anyos_std::println!("{}{}{}", prefix, branch, suffix);
            }
        }
    } else {
        let name = args.positional[1];
        let head_oid = match repo.head() {
            Ok(oid) => oid,
            Err(_) => {
                anyos_std::println!("fatal: not a valid object name: 'HEAD'");
                return;
            }
        };
        let _ = libgit::refs::update_ref(&repo, &format!("refs/heads/{}", name), &head_oid);
    }
}

// ── git checkout ────────────────────────────────────────────────────────────

fn cmd_checkout(args: &anyos_std::args::ParsedArgs) {
    if args.pos_count < 2 {
        anyos_std::println!("usage: git checkout <branch>");
        return;
    }

    let repo = match Repository::open(".") {
        Ok(r) => r,
        Err(e) => {
            anyos_std::println!("fatal: {}", e);
            return;
        }
    };

    let target = args.positional[1];
    let create = args.has(b'b');

    if create {
        // Create and switch
        let head_oid = match repo.head() {
            Ok(oid) => oid,
            Err(_) => {
                anyos_std::println!("fatal: not a valid object name: 'HEAD'");
                return;
            }
        };
        let _ = libgit::refs::update_ref(&repo, &format!("refs/heads/{}", target), &head_oid);
        let _ = libgit::refs::update_head(&repo, target);
        anyos_std::println!("Switched to a new branch '{}'", target);
        return;
    }

    // Check if target is a branch
    let branch_ref = format!("refs/heads/{}", target);
    if let Ok(oid) = libgit::refs::resolve_ref(&repo, &branch_ref) {
        let _ = libgit::refs::update_head(&repo, target);
        match libgit::checkout::checkout_head(&repo) {
            Ok(n) => anyos_std::println!("Switched to branch '{}' ({} files)", target, n),
            Err(e) => anyos_std::println!("warning: checkout failed: {}", e),
        }
    } else if let Some(oid) = Oid::from_hex(target) {
        // Detached HEAD
        let _ = libgit::refs::detach_head(&repo, &oid);
        anyos_std::println!("Note: switching to '{}'.", &target[..7.min(target.len())]);
        anyos_std::println!("You are in 'detached HEAD' state.");
    } else {
        anyos_std::println!(
            "error: pathspec '{}' did not match any branch or commit",
            target
        );
    }
}

// ── git remote ──────────────────────────────────────────────────────────────

fn cmd_remote(args: &anyos_std::args::ParsedArgs) {
    let repo = match Repository::open(".") {
        Ok(r) => r,
        Err(e) => {
            anyos_std::println!("fatal: {}", e);
            return;
        }
    };

    let subcmd = args.pos(1).unwrap_or("");

    match subcmd {
        "" => {
            // List remotes
            let verbose = args.has(b'v');
            if let Ok(remotes) = libgit::remote::list_remotes(&repo) {
                for r in &remotes {
                    if verbose {
                        anyos_std::println!("{}\t{} (fetch)", r.name, r.url);
                        anyos_std::println!("{}\t{} (push)", r.name, r.url);
                    } else {
                        anyos_std::println!("{}", r.name);
                    }
                }
            }
        }
        "add" => {
            if args.pos_count < 4 {
                anyos_std::println!("usage: git remote add <name> <url>");
                return;
            }
            let name = args.positional[2];
            let url = args.positional[3];
            if let Err(e) = libgit::remote::add_remote(&repo, name, url) {
                anyos_std::println!("fatal: {}", e);
            }
        }
        "remove" | "rm" => {
            if args.pos_count < 3 {
                anyos_std::println!("usage: git remote remove <name>");
                return;
            }
            if let Err(e) = libgit::remote::remove_remote(&repo, args.positional[2]) {
                anyos_std::println!("fatal: {}", e);
            }
        }
        "set-url" => {
            if args.pos_count < 4 {
                anyos_std::println!("usage: git remote set-url <name> <url>");
                return;
            }
            if let Err(e) = libgit::remote::set_url(&repo, args.positional[2], args.positional[3]) {
                anyos_std::println!("fatal: {}", e);
            }
        }
        _ => anyos_std::println!("error: unknown subcommand: {}", subcmd),
    }
}

// ── git fetch ───────────────────────────────────────────────────────────────

fn cmd_fetch(args: &anyos_std::args::ParsedArgs) {
    let repo = match Repository::open(".") {
        Ok(r) => r,
        Err(e) => {
            anyos_std::println!("fatal: {}", e);
            return;
        }
    };

    let remote_name = args.pos(1).unwrap_or("origin");
    let remote = match libgit::remote::get_remote(&repo, remote_name) {
        Ok(r) => r,
        Err(_) => {
            anyos_std::println!(
                "fatal: '{}' does not appear to be a git repository",
                remote_name
            );
            return;
        }
    };

    let url = match libgit::remote::GitUrl::parse(&remote.url) {
        Some(u) => u,
        None => {
            anyos_std::println!("fatal: invalid URL: {}", remote.url);
            return;
        }
    };

    anyos_std::println!("From {}", remote.url);

    // Discover remote refs
    let (remote_refs, _) = match libgit::transport::discover_refs(&url, "git-upload-pack") {
        Ok(r) => r,
        Err(e) => {
            anyos_std::println!("fatal: {}", e);
            return;
        }
    };

    // Determine what we need (refs we don't have or that changed)
    let mut wants = Vec::new();
    let mut haves = Vec::new();

    for rref in &remote_refs {
        if rref.name.starts_with("refs/heads/") {
            let local_ref = rref
                .name
                .replace("refs/heads/", &format!("refs/remotes/{}/", remote_name));
            let local_oid = libgit::refs::resolve_ref(&repo, &local_ref).ok();

            if local_oid.as_ref() != Some(&rref.oid) {
                wants.push(rref.oid);
                if let Some(oid) = local_oid {
                    haves.push(oid);
                }
            }
        }
    }

    if wants.is_empty() {
        anyos_std::println!("Already up to date.");
        return;
    }

    // Stream pack directly into repository
    match libgit::transport::fetch_pack_streamed(&url, &wants, &haves, &repo) {
        Ok(_n) => {}
        Err(e) => {
            anyos_std::println!("fatal: {}", e);
            return;
        }
    }

    // Update remote-tracking refs
    for rref in &remote_refs {
        if rref.name.starts_with("refs/heads/") {
            let branch = rref.name.strip_prefix("refs/heads/").unwrap();
            let local_ref = format!("refs/remotes/{}/{}", remote_name, branch);
            let _ = libgit::refs::update_ref(&repo, &local_ref, &rref.oid);
            anyos_std::println!(" * [updated]   {} -> {}/{}", branch, remote_name, branch);
        }
    }
}

// ── git pull ────────────────────────────────────────────────────────────────

fn cmd_pull(args: &anyos_std::args::ParsedArgs) {
    let repo = match Repository::open(".") {
        Ok(r) => r,
        Err(e) => {
            anyos_std::println!("fatal: {}", e);
            return;
        }
    };

    // Fetch first
    cmd_fetch(args);

    // Determine upstream branch
    let remote_name = args.pos(1).unwrap_or("origin");
    let branch = repo
        .current_branch()
        .ok()
        .flatten()
        .unwrap_or_else(|| String::from("main"));
    let remote_ref = format!("refs/remotes/{}/{}", remote_name, branch);

    let remote_oid = match libgit::refs::resolve_ref(&repo, &remote_ref) {
        Ok(oid) => oid,
        Err(_) => {
            anyos_std::println!("fatal: no tracking information for current branch");
            return;
        }
    };

    // Fast-forward merge
    match libgit::merge::fast_forward(&repo, &remote_oid) {
        Ok(libgit::merge::MergeResult::FastForward(oid)) => {
            anyos_std::println!("Updating..{}", oid.short());
            anyos_std::println!("Fast-forward");
            match libgit::checkout::checkout_head(&repo) {
                Ok(n) => {
                    anyos_std::println!(" {} file{} changed", n, if n != 1 { "s" } else { "" })
                }
                Err(e) => anyos_std::println!("warning: checkout failed: {}", e),
            }
        }
        Ok(libgit::merge::MergeResult::UpToDate) => {
            anyos_std::println!("Already up to date.");
        }
        Ok(libgit::merge::MergeResult::Conflict(_)) | Err(_) => {
            anyos_std::println!("error: merge not possible (fast-forward only)");
        }
        _ => {}
    }
}

// ── git push ────────────────────────────────────────────────────────────────

fn cmd_push(args: &anyos_std::args::ParsedArgs) {
    let repo = match Repository::open(".") {
        Ok(r) => r,
        Err(e) => {
            anyos_std::println!("fatal: {}", e);
            return;
        }
    };

    let remote_name = args.pos(1).unwrap_or("origin");
    let remote = match libgit::remote::get_remote(&repo, remote_name) {
        Ok(r) => r,
        Err(_) => {
            anyos_std::println!(
                "fatal: '{}' does not appear to be a git repository",
                remote_name
            );
            return;
        }
    };

    let url = match libgit::remote::GitUrl::parse(&remote.url) {
        Some(u) => u,
        None => {
            anyos_std::println!("fatal: invalid URL: {}", remote.url);
            return;
        }
    };

    let branch = if args.pos_count >= 3 {
        String::from(args.positional[2])
    } else {
        repo.current_branch()
            .ok()
            .flatten()
            .unwrap_or_else(|| String::from("main"))
    };

    let local_oid = match repo.head() {
        Ok(oid) => oid,
        Err(_) => {
            anyos_std::println!("fatal: no commits to push");
            return;
        }
    };

    // Get remote's current ref (if any)
    let remote_ref = format!("refs/heads/{}", branch);
    let (remote_refs, _) = match libgit::transport::discover_refs(&url, "git-receive-pack") {
        Ok(r) => r,
        Err(e) => {
            anyos_std::println!("fatal: {}", e);
            return;
        }
    };

    let old_oid = remote_refs
        .iter()
        .find(|r| r.name == remote_ref)
        .map(|r| r.oid)
        .unwrap_or(Oid::ZERO);

    if old_oid == local_oid {
        anyos_std::println!("Everything up-to-date");
        return;
    }

    anyos_std::println!("Enumerating objects...");

    // Collect objects to send
    let exclude: Vec<Oid> = remote_refs.iter().map(|r| r.oid).collect();
    let objects = match repo.collect_objects(&local_oid, &exclude) {
        Ok(o) => o,
        Err(e) => {
            anyos_std::println!("fatal: {}", e);
            return;
        }
    };

    anyos_std::println!("Counting objects: {}, done.", objects.len());

    // Build pack
    let pack_data = libgit::pack::build_pack(&objects);

    // Push
    let updates = alloc::vec![(old_oid, local_oid, format!("refs/heads/{}", branch))];
    match libgit::transport::push_pack(&url, &updates, &pack_data) {
        Ok(response) => {
            if response.contains("unpack ok") || response.contains("000eunpack ok") {
                anyos_std::println!("To {}", remote.url);
                anyos_std::println!(
                    "   {}..{}  {} -> {}",
                    old_oid.short(),
                    local_oid.short(),
                    branch,
                    branch
                );
            } else {
                anyos_std::println!("remote: {}", response.trim());
            }
        }
        Err(e) => anyos_std::println!("fatal: {}", e),
    }
}

// ── git config ──────────────────────────────────────────────────────────────

fn cmd_config(args: &anyos_std::args::ParsedArgs) {
    let repo = match Repository::open(".") {
        Ok(r) => r,
        Err(e) => {
            anyos_std::println!("fatal: {}", e);
            return;
        }
    };

    if args.pos_count < 2 || args.has(b'l') {
        // --list
        if let Ok(config) = libgit::config::read_config(&repo) {
            for entry in &config.entries {
                let section = if let Some(ref sub) = entry.subsection {
                    format!("{}.{}", entry.section, sub)
                } else {
                    entry.section.clone()
                };
                anyos_std::println!("{}.{}={}", section, entry.key, entry.value);
            }
        }
    } else if args.pos_count == 2 {
        let key = args.positional[1];
        if let Some(dot) = key.rfind('.') {
            if let Ok(config) = libgit::config::read_config(&repo) {
                if let Some(val) = config.get(&key[..dot], &key[dot + 1..]) {
                    anyos_std::println!("{}", val);
                }
            }
        }
    } else if args.pos_count >= 3 {
        let key = args.positional[1];
        let value = args.positional[2];
        if let Some(dot) = key.rfind('.') {
            let config_path = repo.gitdir.join("config");
            let mut content = std::fs::read_to_string(&config_path).unwrap_or_default();
            content.push_str(&format!(
                "[{}]\n\t{} = {}\n",
                &key[..dot],
                &key[dot + 1..],
                value
            ));
            let _ = std::fs::write(&config_path, content.as_bytes());
        }
    }
}

// ── git tag ─────────────────────────────────────────────────────────────────

fn cmd_tag(args: &anyos_std::args::ParsedArgs) {
    let repo = match Repository::open(".") {
        Ok(r) => r,
        Err(e) => {
            anyos_std::println!("fatal: {}", e);
            return;
        }
    };

    if args.pos_count < 2 {
        // List tags
        let tags_dir = repo.gitdir.join("refs/tags");
        if let Ok(entries) = std::fs::read_dir(&tags_dir) {
            for entry in entries {
                if let Ok(entry) = entry {
                    if let Some(name) = entry.file_name().to_str() {
                        anyos_std::println!("{}", name);
                    }
                }
            }
        }
    } else {
        // Create lightweight tag
        let tag_name = args.positional[1];
        let oid = match repo.head() {
            Ok(o) => o,
            Err(_) => {
                anyos_std::println!("fatal: no commits");
                return;
            }
        };
        let tag_ref = format!("refs/tags/{}", tag_name);
        if let Err(e) = libgit::refs::update_ref(&repo, &tag_ref, &oid) {
            anyos_std::println!("fatal: {}", e);
        }
    }
}

// ── git rm ──────────────────────────────────────────────────────────────────

fn cmd_rm(args: &anyos_std::args::ParsedArgs) {
    if args.pos_count < 2 {
        anyos_std::println!("usage: git rm <file>...");
        return;
    }

    let repo = match Repository::open(".") {
        Ok(r) => r,
        Err(e) => {
            anyos_std::println!("fatal: {}", e);
            return;
        }
    };

    let mut index = Index::read(&repo).unwrap_or_else(|_| Index::new());

    for i in 1..args.pos_count {
        let path = args.positional[i];
        index.remove(path);
        let full_path = repo.workdir_path(path);
        let _ = std::fs::remove_file(&full_path);
        anyos_std::println!("rm '{}'", path);
    }

    let _ = index.write(&repo);
}

// ── git reset ───────────────────────────────────────────────────────────────

fn cmd_reset(args: &anyos_std::args::ParsedArgs) {
    let repo = match Repository::open(".") {
        Ok(r) => r,
        Err(e) => {
            anyos_std::println!("fatal: {}", e);
            return;
        }
    };

    // Simple reset: rebuild index from HEAD tree
    let head_oid = match repo.head() {
        Ok(o) => o,
        Err(_) => {
            anyos_std::println!("fatal: no HEAD");
            return;
        }
    };

    let commit_obj = match repo.read_object(&head_oid) {
        Ok(o) => o,
        Err(e) => {
            anyos_std::println!("fatal: {}", e);
            return;
        }
    };

    let commit = match Commit::parse(&commit_obj.data) {
        Some(c) => c,
        None => {
            anyos_std::println!("fatal: invalid commit");
            return;
        }
    };

    match libgit::checkout::build_index_from_tree(&repo, &commit.tree) {
        Ok(index) => {
            if let Err(e) = index.write(&repo) {
                anyos_std::println!("fatal: {}", e);
            } else {
                anyos_std::println!("Unstaged changes after reset:");
            }
        }
        Err(e) => anyos_std::println!("fatal: {}", e),
    }
}

// ── git show ────────────────────────────────────────────────────────────────

fn cmd_show(args: &anyos_std::args::ParsedArgs) {
    let repo = match Repository::open(".") {
        Ok(r) => r,
        Err(e) => {
            anyos_std::println!("fatal: {}", e);
            return;
        }
    };

    let rev = args.pos(1).unwrap_or("HEAD");
    let oid = resolve_rev(&repo, rev);

    let oid = match oid {
        Some(o) => o,
        None => {
            anyos_std::println!("fatal: bad object {}", rev);
            return;
        }
    };

    let obj = match repo.read_object(&oid) {
        Ok(o) => o,
        Err(e) => {
            anyos_std::println!("fatal: {}", e);
            return;
        }
    };

    match obj.obj_type {
        ObjectType::Commit => {
            let commit = match Commit::parse(&obj.data) {
                Some(c) => c,
                None => return,
            };
            anyos_std::println!("\x1b[33mcommit {}\x1b[0m", oid.to_hex());
            anyos_std::println!("Author: {}", commit.author);
            anyos_std::println!();
            for line in commit.message.lines() {
                anyos_std::println!("    {}", line);
            }
        }
        ObjectType::Blob => {
            if let Ok(text) = core::str::from_utf8(&obj.data) {
                anyos_std::print!("{}", text);
            } else {
                anyos_std::println!("(binary data, {} bytes)", obj.data.len());
            }
        }
        ObjectType::Tree => {
            let entries = parse_tree(&obj.data);
            for entry in &entries {
                anyos_std::println!(
                    "{:06o} {} {}\t{}",
                    entry.mode,
                    if entry.is_tree() { "tree" } else { "blob" },
                    entry.oid.short(),
                    entry.name
                );
            }
        }
        ObjectType::Tag => {
            if let Ok(text) = core::str::from_utf8(&obj.data) {
                anyos_std::print!("{}", text);
            }
        }
    }
}

// ── git rev-parse ───────────────────────────────────────────────────────────

fn cmd_rev_parse(args: &anyos_std::args::ParsedArgs) {
    let repo = match Repository::open(".") {
        Ok(r) => r,
        Err(e) => {
            anyos_std::println!("fatal: {}", e);
            return;
        }
    };

    for i in 1..args.pos_count {
        let rev = args.positional[i];
        match resolve_rev(&repo, rev) {
            Some(oid) => anyos_std::println!("{}", oid.to_hex()),
            None => anyos_std::println!("fatal: bad revision '{}'", rev),
        }
    }
}

// ── git hash-object ─────────────────────────────────────────────────────────

fn cmd_hash_object(args: &anyos_std::args::ParsedArgs) {
    for i in 1..args.pos_count {
        let path = args.positional[i];
        if let Ok(data) = std::fs::read(std::path::Path::new(path)) {
            let obj = Object::blob(data);
            anyos_std::println!("{}", obj.id().to_hex());
        } else {
            anyos_std::println!("fatal: Unable to read file '{}'", path);
        }
    }
}

// ── git cat-file ────────────────────────────────────────────────────────────

fn cmd_cat_file(args: &anyos_std::args::ParsedArgs) {
    if args.pos_count < 3 {
        anyos_std::println!("usage: git cat-file (-t | -s | -p) <object>");
        return;
    }

    let repo = match Repository::open(".") {
        Ok(r) => r,
        Err(e) => {
            anyos_std::println!("fatal: {}", e);
            return;
        }
    };

    let flag = args.positional[1];
    let rev = args.positional[2];

    let oid = match resolve_rev(&repo, rev) {
        Some(o) => o,
        None => {
            anyos_std::println!("fatal: bad object {}", rev);
            return;
        }
    };

    let obj = match repo.read_object(&oid) {
        Ok(o) => o,
        Err(e) => {
            anyos_std::println!("fatal: {}", e);
            return;
        }
    };

    match flag {
        "-t" => anyos_std::println!("{}", obj.obj_type.as_str()),
        "-s" => anyos_std::println!("{}", obj.data.len()),
        "-p" => {
            if let Ok(text) = core::str::from_utf8(&obj.data) {
                anyos_std::print!("{}", text);
            } else {
                anyos_std::println!("(binary data, {} bytes)", obj.data.len());
            }
        }
        _ => anyos_std::println!("error: unknown flag '{}'", flag),
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn resolve_rev(repo: &Repository, rev: &str) -> Option<Oid> {
    // HEAD
    if rev == "HEAD" {
        return repo.head().ok();
    }
    // Branch name
    if let Ok(oid) = libgit::refs::resolve_ref(repo, &format!("refs/heads/{}", rev)) {
        return Some(oid);
    }
    // Tag name
    if let Ok(oid) = libgit::refs::resolve_ref(repo, &format!("refs/tags/{}", rev)) {
        return Some(oid);
    }
    // Full hex SHA
    Oid::from_hex(rev)
}

fn get_user_info(repo: &Repository) -> (String, String) {
    let config = libgit::config::read_config(repo).ok();
    let global_config = libgit::config::read_global_config();
    let name = config
        .as_ref()
        .and_then(|c| c.user_name())
        .or_else(|| global_config.as_ref().and_then(|c| c.user_name()))
        .unwrap_or("Unknown");
    let email = config
        .as_ref()
        .and_then(|c| c.user_email())
        .or_else(|| global_config.as_ref().and_then(|c| c.user_email()))
        .unwrap_or("unknown@unknown");
    (String::from(name), String::from(email))
}

fn get_timestamp() -> u64 {
    let mut buf = [0u8; 8];
    anyos_std::sys::time(&mut buf);
    let year = buf[0] as u64 | ((buf[1] as u64) << 8);
    let month = buf[2] as u64;
    let day = buf[3] as u64;
    let hour = buf[4] as u64;
    let min = buf[5] as u64;
    let sec = buf[6] as u64;
    let days = (year - 1970) * 365 + (year - 1969) / 4 + month_days(month) + day - 1;
    days * 86400 + hour * 3600 + min * 60 + sec
}

fn month_days(month: u64) -> u64 {
    const C: [u64; 13] = [0, 0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    if month < 13 {
        C[month as usize]
    } else {
        0
    }
}
