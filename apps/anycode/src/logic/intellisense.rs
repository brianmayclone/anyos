use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::logic::language::{self, LanguageId};
use crate::logic::symbol_index::{IndexedSymbol, SymbolIndex};
use crate::logic::symbols::SymbolKind;
use crate::util::path;

const MAX_COMPLETIONS: usize = 24;

#[derive(Clone)]
pub struct CompletionItem {
    pub label: String,
    pub insert_text: String,
    pub detail: String,
}

pub struct CompletionSet {
    pub prefix: String,
    pub items: Vec<CompletionItem>,
}

pub fn completions_for_cursor(
    file_path: &str,
    text: &str,
    row: u32,
    col: u32,
    index: &SymbolIndex,
) -> CompletionSet {
    let lang = language::language_for_filename(path::basename(file_path)).id;
    let prefix = prefix_at(text, row as usize, col as usize);
    let mut items = Vec::new();

    if let Some(info) = language::info_for_id(lang) {
        for &(trigger, body) in info.snippets {
            push_completion(&mut items, &prefix, trigger, body, "snippet");
        }
        for keyword in info.keywords {
            push_completion(&mut items, &prefix, keyword, keyword, lang.display_name());
        }
    }

    for symbol in &index.symbols {
        if !symbol_matches_file(lang, symbol) {
            continue;
        }
        push_completion(
            &mut items,
            &prefix,
            &symbol.name,
            &symbol.name,
            &format!(
                "{} · {}",
                symbol.kind.label(),
                path::basename(&symbol.file_path)
            ),
        );
        if items.len() >= MAX_COMPLETIONS {
            break;
        }
    }

    CompletionSet { prefix, items }
}

pub fn hover_for_cursor(
    file_path: &str,
    text: &str,
    row: u32,
    col: u32,
    index: &SymbolIndex,
) -> String {
    let word = word_at(text, row as usize, col as usize);
    if word.is_empty() {
        return String::new();
    }

    if let Some(symbol) = best_symbol_for_word(file_path, &word, index) {
        return format!(
            "{} {}\n{}\n{}:{}",
            symbol.kind.label(),
            symbol.name,
            symbol.detail,
            path::basename(&symbol.file_path),
            symbol.line + 1
        );
    }

    let lang = language::language_for_filename(path::basename(file_path)).id;
    if let Some(info) = language::info_for_id(lang) {
        if info.keywords.iter().any(|kw| *kw == word) {
            return format!("{} keyword: {}", lang.display_name(), word);
        }
    }

    String::new()
}

pub fn word_at_cursor(text: &str, row: u32, col: u32) -> String {
    word_at(text, row as usize, col as usize)
}

pub fn best_symbol_for_word<'a>(
    file_path: &str,
    word: &str,
    index: &'a SymbolIndex,
) -> Option<&'a IndexedSymbol> {
    let mut fallback = None;
    for symbol in &index.symbols {
        if symbol.name != word {
            continue;
        }
        if symbol.file_path == file_path {
            return Some(symbol);
        }
        if fallback.is_none() {
            fallback = Some(symbol);
        }
    }
    fallback
}

pub fn completion_list_text(items: &[CompletionItem]) -> String {
    let mut out = String::new();
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push('|');
        }
        out.push_str(&item.label);
        if !item.detail.is_empty() {
            out.push_str("    ");
            out.push_str(&item.detail);
        }
    }
    out
}

fn push_completion(
    items: &mut Vec<CompletionItem>,
    prefix: &str,
    label: &str,
    insert_text: &str,
    detail: &str,
) {
    if items.len() >= MAX_COMPLETIONS || label.is_empty() {
        return;
    }
    if !prefix.is_empty() && !starts_with_ascii_ci(label, prefix) {
        return;
    }
    if items.iter().any(|item| item.label == label) {
        return;
    }
    items.push(CompletionItem {
        label: String::from(label),
        insert_text: strip_placeholders(insert_text),
        detail: String::from(detail),
    });
}

fn symbol_matches_file(lang: LanguageId, symbol: &IndexedSymbol) -> bool {
    match lang {
        LanguageId::Rust => matches!(
            symbol.kind,
            SymbolKind::Function
                | SymbolKind::Method
                | SymbolKind::Struct
                | SymbolKind::Enum
                | SymbolKind::Trait
                | SymbolKind::Module
                | SymbolKind::Macro
                | SymbolKind::TypeAlias
                | SymbolKind::Constant
        ),
        _ => true,
    }
}

fn prefix_at(text: &str, row: usize, col: usize) -> String {
    let line = nth_line(text, row);
    let bytes = line.as_bytes();
    let mut end = col.min(bytes.len());
    while end > 0 && !is_ident_byte(bytes[end - 1]) {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && is_ident_byte(bytes[start - 1]) {
        start -= 1;
    }
    String::from(core::str::from_utf8(&bytes[start..end]).unwrap_or(""))
}

fn word_at(text: &str, row: usize, col: usize) -> String {
    let line = nth_line(text, row);
    let bytes = line.as_bytes();
    if bytes.is_empty() {
        return String::new();
    }
    let mut pos = col.min(bytes.len().saturating_sub(1));
    if !is_ident_byte(bytes[pos]) && pos > 0 {
        pos -= 1;
    }
    if !is_ident_byte(bytes[pos]) {
        return String::new();
    }
    let mut start = pos;
    while start > 0 && is_ident_byte(bytes[start - 1]) {
        start -= 1;
    }
    let mut end = pos + 1;
    while end < bytes.len() && is_ident_byte(bytes[end]) {
        end += 1;
    }
    String::from(core::str::from_utf8(&bytes[start..end]).unwrap_or(""))
}

fn nth_line(text: &str, row: usize) -> &str {
    text.split('\n').nth(row).unwrap_or("")
}

fn is_ident_byte(b: u8) -> bool {
    b == b'_' || b.is_ascii_alphanumeric()
}

fn starts_with_ascii_ci(value: &str, prefix: &str) -> bool {
    value.len() >= prefix.len() && value[..prefix.len()].eq_ignore_ascii_case(prefix)
}

fn strip_placeholders(template: &str) -> String {
    let mut out = String::new();
    let mut chars = template.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '$' && chars.peek() == Some(&'{') {
            chars.next();
            while let Some(inner) = chars.next() {
                if inner == ':' {
                    break;
                }
                if inner == '}' {
                    break;
                }
            }
            while let Some(inner) = chars.next() {
                if inner == '}' {
                    break;
                }
                out.push(inner);
            }
        } else {
            out.push(ch);
        }
    }
    out
}
