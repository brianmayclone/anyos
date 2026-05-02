use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::logic::language::{self, LanguageId};
use crate::logic::node_packages;
use crate::logic::project::Project;
use crate::logic::symbol_index::{IndexedSymbol, SymbolIndex};
use crate::logic::symbols::SymbolKind;
use crate::util::path;

const MAX_COMPLETIONS: usize = 64;

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
    project: Option<&Project>,
) -> CompletionSet {
    let lang = language::language_for_filename(path::basename(file_path)).id;
    let prefix = prefix_at(text, row as usize, col as usize);
    let mut items = Vec::new();

    if matches!(lang, LanguageId::JavaScript | LanguageId::TypeScript) {
        if let Some(module_prefix) = module_string_prefix_at(text, row as usize, col as usize) {
            push_node_module_completions(&mut items, &module_prefix, project);
            return CompletionSet {
                prefix: module_prefix,
                items,
            };
        }

        if let Some((receiver, member_prefix)) = member_access_at(text, row as usize, col as usize)
        {
            if is_anyui_alias(text, &receiver) {
                let mut member_items = Vec::new();
                push_anyui_members(&mut member_items, &member_prefix);
                return CompletionSet {
                    prefix: member_prefix,
                    items: member_items,
                };
            }
            if let Some(kind) = js_receiver_kind(text, &receiver) {
                let mut member_items = Vec::new();
                push_js_receiver_members(&mut member_items, &member_prefix, kind);
                return CompletionSet {
                    prefix: member_prefix,
                    items: member_items,
                };
            }
        }
        push_js_node_globals(&mut items, &prefix, project);
        push_js_anyos_completions(&mut items, &prefix);
    }

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

fn push_js_anyos_completions(items: &mut Vec<CompletionItem>, prefix: &str) {
    for &(label, insert, detail) in JS_ANYOS_COMPLETIONS {
        push_completion(items, prefix, label, insert, detail);
    }
}

fn push_anyui_members(items: &mut Vec<CompletionItem>, prefix: &str) {
    for &(label, insert, detail) in ANYUI_MEMBER_COMPLETIONS {
        push_completion(items, prefix, label, insert, detail);
    }
}

const JS_ANYOS_COMPLETIONS: &[(&str, &str, &str)] = &[
    ("@anyos/anyui", "@anyos/anyui", "native anyOS UI module"),
    (
        "requireAnyUI",
        "const ui = require('@anyos/anyui');",
        "import native anyOS UI",
    ),
    (
        "createWindow",
        "const win = new ui.Window('Main', 120, 80, 800, 520);",
        "anyOS UI window",
    ),
    (
        "anyuiButton",
        "const button = new ui.Button('OK');",
        "anyOS UI control",
    ),
    (
        "anyuiLabel",
        "const label = new ui.Label('Label');",
        "anyOS UI control",
    ),
];

const ANYUI_MEMBER_COMPLETIONS: &[(&str, &str, &str)] = &[
    ("Window", "Window", "@anyos/anyui class"),
    ("View", "View", "@anyos/anyui class"),
    ("Button", "Button", "@anyos/anyui class"),
    ("PlainButton", "PlainButton", "@anyos/anyui class"),
    ("IconButton", "IconButton", "@anyos/anyui class"),
    ("ImageButton", "ImageButton", "@anyos/anyui class"),
    ("Label", "Label", "@anyos/anyui class"),
    ("LinkLabel", "LinkLabel", "@anyos/anyui class"),
    ("TextField", "TextField", "@anyos/anyui class"),
    ("TextArea", "TextArea", "@anyos/anyui class"),
    (
        "AutoCompleteTextField",
        "AutoCompleteTextField",
        "@anyos/anyui class",
    ),
    ("SearchField", "SearchField", "@anyos/anyui class"),
    ("CheckBox", "CheckBox", "@anyos/anyui class"),
    ("RadioButton", "RadioButton", "@anyos/anyui class"),
    ("Toggle", "Toggle", "@anyos/anyui class"),
    ("DropDown", "DropDown", "@anyos/anyui class"),
    ("ComboBox", "ComboBox", "@anyos/anyui class"),
    ("ListBox", "ListBox", "@anyos/anyui class"),
    ("TreeView", "TreeView", "@anyos/anyui class"),
    ("DataGrid", "DataGrid", "@anyos/anyui class"),
    ("TableView", "TableView", "@anyos/anyui class"),
    ("TabBar", "TabBar", "@anyos/anyui class"),
    ("SegmentedControl", "SegmentedControl", "@anyos/anyui class"),
    ("Toolbar", "Toolbar", "@anyos/anyui class"),
    ("NavigationBar", "NavigationBar", "@anyos/anyui class"),
    ("GroupBox", "GroupBox", "@anyos/anyui class"),
    ("Panel", "Panel", "@anyos/anyui class"),
    ("FlowPanel", "FlowPanel", "@anyos/anyui class"),
    ("StackPanel", "StackPanel", "@anyos/anyui class"),
    ("SplitView", "SplitView", "@anyos/anyui class"),
    ("ScrollView", "ScrollView", "@anyos/anyui class"),
    ("Canvas", "Canvas", "@anyos/anyui class"),
    ("ImageView", "ImageView", "@anyos/anyui class"),
    ("ColorWell", "ColorWell", "@anyos/anyui class"),
    ("DatePicker", "DatePicker", "@anyos/anyui class"),
    ("TimePicker", "TimePicker", "@anyos/anyui class"),
    ("ProgressBar", "ProgressBar", "@anyos/anyui class"),
    ("Slider", "Slider", "@anyos/anyui class"),
    ("Stepper", "Stepper", "@anyos/anyui class"),
    ("Spinner", "Spinner", "@anyos/anyui class"),
    ("StatusIndicator", "StatusIndicator", "@anyos/anyui class"),
    ("Alert", "Alert", "@anyos/anyui class"),
    ("Badge", "Badge", "@anyos/anyui class"),
    ("Tooltip", "Tooltip", "@anyos/anyui class"),
];

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

fn member_access_at(text: &str, row: usize, col: usize) -> Option<(String, String)> {
    let line = nth_line(text, row);
    let bytes = line.as_bytes();
    let col = col.min(bytes.len());
    let mut dot = col;
    while dot > 0 && is_ident_byte(bytes[dot - 1]) {
        dot -= 1;
    }
    if dot == 0 || bytes[dot - 1] != b'.' {
        return None;
    }
    let member = core::str::from_utf8(&bytes[dot..col]).unwrap_or("");
    let mut receiver_end = dot - 1;
    while receiver_end > 0 && bytes[receiver_end - 1].is_ascii_whitespace() {
        receiver_end -= 1;
    }
    let mut receiver_start = receiver_end;
    while receiver_start > 0 && is_ident_byte(bytes[receiver_start - 1]) {
        receiver_start -= 1;
    }
    if receiver_start == receiver_end {
        return None;
    }
    Some((
        String::from(core::str::from_utf8(&bytes[receiver_start..receiver_end]).unwrap_or("")),
        String::from(member),
    ))
}

fn is_anyui_alias(text: &str, receiver: &str) -> bool {
    if receiver == "ui" || receiver == "anyui" {
        return true;
    }
    for line in text.split('\n') {
        if !line.contains("@anyos/anyui") || !line.contains(receiver) {
            continue;
        }
        let compact = without_spaces(line);
        if compact.contains(&format!("{}=require('@anyos/anyui')", receiver))
            || compact.contains(&format!("{}=require(\"@anyos/anyui\")", receiver))
            || compact.contains(&format!("*as{}from'@anyos/anyui'", receiver))
            || compact.contains(&format!("*as{}from\"@anyos/anyui\"", receiver))
        {
            return true;
        }
    }
    false
}

fn without_spaces(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if !ch.is_whitespace() {
            out.push(ch);
        }
    }
    out
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
