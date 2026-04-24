use alloc::string::String;
use alloc::vec::Vec;

use crate::logic::language;
use crate::logic::symbols::{self, SymbolKind};
use crate::util::path;

#[derive(Clone)]
pub struct IndexedSymbol {
    pub name: String,
    pub kind: SymbolKind,
    pub file_path: String,
    pub line: u32,
    pub detail: String,
}

pub struct SymbolIndex {
    pub symbols: Vec<IndexedSymbol>,
    pub root: String,
}

impl SymbolIndex {
    pub fn new() -> Self {
        Self {
            symbols: Vec::new(),
            root: String::new(),
        }
    }

    pub fn clear(&mut self) {
        self.symbols.clear();
        self.root.clear();
    }

    pub fn rebuild(&mut self, root: &str) {
        self.clear();
        self.root = String::from(root);
        self.scan_dir(root, 0);
        self.symbols.sort_by(|a, b| {
            a.name
                .cmp(&b.name)
                .then(a.file_path.cmp(&b.file_path))
                .then(a.line.cmp(&b.line))
        });
    }

    pub fn count(&self) -> usize {
        self.symbols.len()
    }

    pub fn find_by_name(&self, name: &str) -> Vec<&IndexedSymbol> {
        self.symbols
            .iter()
            .filter(|symbol| symbol.name == name)
            .collect()
    }

    fn scan_dir(&mut self, dir: &str, depth: u32) {
        if depth > 10 || self.symbols.len() > 8000 {
            return;
        }

        let entries = match anyos_std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(_) => return,
        };

        for entry in entries {
            if entry.name == "." || entry.name == ".." {
                continue;
            }
            if entry.is_dir() {
                if should_skip_dir(&entry.name) {
                    continue;
                }
                self.scan_dir(&path::join(dir, &entry.name), depth + 1);
                continue;
            }
            if should_index_file(&entry.name) {
                let file_path = path::join(dir, &entry.name);
                self.scan_file(&file_path);
            }
        }
    }

    fn scan_file(&mut self, file_path: &str) {
        let text = match anyos_std::fs::read_to_string(file_path) {
            Ok(text) => text,
            Err(_) => return,
        };
        let filename = path::basename(file_path);
        let lang = language::language_for_filename(filename);
        let extracted = symbols::extract_symbols(&text, lang.id);
        for symbol in extracted {
            self.symbols.push(IndexedSymbol {
                name: symbol.name,
                kind: symbol.kind,
                file_path: String::from(file_path),
                line: symbol.line,
                detail: symbol.detail,
            });
        }
    }
}

fn should_index_file(name: &str) -> bool {
    matches!(
        path::extension(name),
        "rs" | "c" | "h" | "cpp" | "cc" | "hpp" | "py" | "js" | "ts" | "sh" | "mk"
    ) || name == "Makefile"
}

fn should_skip_dir(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | ".svn"
            | ".hg"
            | ".vscode"
            | ".idea"
            | "target"
            | "build"
            | "node_modules"
            | "__pycache__"
            | ".venv"
            | "dist"
    )
}
