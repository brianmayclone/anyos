//! `apkg info` — show detailed package information.

use anyos_std::println;
use crate::{db, index};

/// Execute `apkg info <name>`.
pub fn run(name: &str) {
    if name.is_empty() {
        println!("Usage: apkg info <package>");
        return;
    }

    // Check installed database first
    let database = db::Database::load();
    let installed = database.get(name);

    // Check remote index
    let idx = index::Index::load();
    let available = idx.as_ref().and_then(|i| i.find(name));

    if installed.is_none() && available.is_none() {
        println!("apkg: package '{}' not found", name);
        return;
    }

    if let Some(avail) = available {
        println!("Name:          {}", avail.name);
        println!("Version:       {}", avail.version_str);
        println!("Description:   {}", avail.description);
        println!("Category:      {}", avail.category);
        println!("Type:          {}", avail.pkg_type);
        println!("Architecture:  {}", avail.arch);
        println!("Download size: {} bytes", avail.size);
        println!("Install size:  {} bytes", avail.size_installed);
        if !avail.depends.is_empty() {
            println!("Dependencies:  {}", join(&avail.depends, ", "));
        }
        if !avail.provides.is_empty() {
            println!("Provides:      {}", join(&avail.provides, ", "));
        }
        println!("Filename:      {}", avail.filename);
        if !avail.md5.is_empty() {
            println!("MD5:           {}", avail.md5);
        }
    } else if let Some(inst) = installed {
        println!("Name:          {}", inst.name);
        println!("Version:       {}", inst.version);
        println!("Type:          {}", inst.pkg_type);
        println!("Auto:          {}", if inst.auto { "yes" } else { "no" });
        if !inst.depends.is_empty() {
            println!("Dependencies:  {}", join(&inst.depends, ", "));
        }
        println!("Files:         {} file(s)", inst.files.len());
    }

    if let Some(inst) = installed {
        println!("Status:        installed ({})", inst.version);
    } else {
        println!("Status:        not installed");
    }
}

/// Join a slice of strings with a separator.
fn join(items: &[alloc::string::String], sep: &str) -> alloc::string::String {
    let mut result = alloc::string::String::new();
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            result.push_str(sep);
        }
        result.push_str(item);
    }
    result
}
