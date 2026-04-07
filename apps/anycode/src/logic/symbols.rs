use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;

use crate::logic::language::LanguageId;

// ════════════════════════════════════════════════════════════════
//  Symbol extraction — outline view and quick navigation
// ════════════════════════════════════════════════════════════════

/// Kind of symbol extracted from source code.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SymbolKind {
    Function,
    Method,
    Struct,
    Enum,
    Trait,
    Impl,
    Class,
    Interface,
    Constant,
    Variable,
    Module,
    Import,
    Macro,
    TypeAlias,
    Field,
}

impl SymbolKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Function => "fn",
            Self::Method => "method",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::Trait => "trait",
            Self::Impl => "impl",
            Self::Class => "class",
            Self::Interface => "interface",
            Self::Constant => "const",
            Self::Variable => "var",
            Self::Module => "mod",
            Self::Import => "use",
            Self::Macro => "macro",
            Self::TypeAlias => "type",
            Self::Field => "field",
        }
    }

    pub fn icon_color(&self) -> u32 {
        match self {
            Self::Function | Self::Method => 0xFFDCDCAA,    // Yellow
            Self::Struct | Self::Class => 0xFF4EC9B0,       // Teal
            Self::Enum => 0xFF4EC9B0,                       // Teal
            Self::Trait | Self::Interface => 0xFF4EC9B0,    // Teal
            Self::Impl => 0xFF9CDCFE,                       // Light blue
            Self::Constant => 0xFF569CD6,                   // Blue
            Self::Variable => 0xFF9CDCFE,                   // Light blue
            Self::Module | Self::Import => 0xFFCE9178,      // Orange
            Self::Macro => 0xFFD19A66,                      // Dark orange
            Self::TypeAlias => 0xFF4EC9B0,                  // Teal
            Self::Field => 0xFF9CDCFE,                      // Light blue
        }
    }
}

/// A symbol extracted from source code.
#[derive(Clone, Debug)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub line: u32,
    pub parent: Option<String>,
    pub detail: String, // e.g. "fn name(args) -> RetType"
}

// ════════════════════════════════════════════════════════════════
//  Symbol extractor — language-aware
// ════════════════════════════════════════════════════════════════

/// Extract symbols from source code based on the detected language.
pub fn extract_symbols(content: &str, language: LanguageId) -> Vec<Symbol> {
    match language {
        LanguageId::Rust => extract_rust_symbols(content),
        LanguageId::C | LanguageId::Cpp => extract_c_symbols(content),
        LanguageId::Python => extract_python_symbols(content),
        LanguageId::JavaScript | LanguageId::TypeScript => extract_js_symbols(content),
        LanguageId::Shell => extract_shell_symbols(content),
        LanguageId::Makefile => extract_makefile_symbols(content),
        _ => Vec::new(),
    }
}

// ── Rust symbol extraction ─────────────────────────────────────

fn extract_rust_symbols(content: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    let mut current_impl: Option<String> = None;

    for (line_no, line) in content.split('\n').enumerate() {
        let trimmed = line.trim();

        // Skip comments and empty lines
        if trimmed.starts_with("//") || trimmed.is_empty() {
            continue;
        }

        // Detect `impl Type` blocks
        if trimmed.starts_with("impl ") || trimmed.starts_with("impl<") {
            let name = extract_impl_name(trimmed);
            if !name.is_empty() {
                symbols.push(Symbol {
                    name: name.clone(),
                    kind: SymbolKind::Impl,
                    line: line_no as u32,
                    parent: None,
                    detail: String::from(trimmed.trim_end_matches('{').trim()),
                });
                current_impl = Some(name);
            }
            continue;
        }

        // Track block nesting to clear impl context
        if trimmed == "}" && !trimmed.contains('{') {
            // Rough heuristic: closing brace at column 0 ends the current impl
            if line.starts_with('}') {
                current_impl = None;
            }
        }

        // pub fn / fn
        if let Some(fn_info) = try_extract_rust_fn(trimmed) {
            symbols.push(Symbol {
                name: fn_info.0,
                kind: if current_impl.is_some() { SymbolKind::Method } else { SymbolKind::Function },
                line: line_no as u32,
                parent: current_impl.clone(),
                detail: fn_info.1,
            });
            continue;
        }

        // struct
        if trimmed.starts_with("pub struct ") || trimmed.starts_with("struct ") {
            if let Some(name) = extract_ident_after(trimmed, "struct") {
                symbols.push(Symbol {
                    name,
                    kind: SymbolKind::Struct,
                    line: line_no as u32,
                    parent: None,
                    detail: String::from(trimmed.split('{').next().unwrap_or(trimmed).trim()),
                });
            }
            continue;
        }

        // enum
        if trimmed.starts_with("pub enum ") || trimmed.starts_with("enum ") {
            if let Some(name) = extract_ident_after(trimmed, "enum") {
                symbols.push(Symbol {
                    name,
                    kind: SymbolKind::Enum,
                    line: line_no as u32,
                    parent: None,
                    detail: String::from(trimmed.split('{').next().unwrap_or(trimmed).trim()),
                });
            }
            continue;
        }

        // trait
        if trimmed.starts_with("pub trait ") || trimmed.starts_with("trait ") {
            if let Some(name) = extract_ident_after(trimmed, "trait") {
                symbols.push(Symbol {
                    name,
                    kind: SymbolKind::Trait,
                    line: line_no as u32,
                    parent: None,
                    detail: String::from(trimmed.split('{').next().unwrap_or(trimmed).trim()),
                });
            }
            continue;
        }

        // mod
        if trimmed.starts_with("pub mod ") || trimmed.starts_with("mod ") {
            if let Some(name) = extract_ident_after(trimmed, "mod") {
                let name = String::from(name.trim_end_matches(';'));
                symbols.push(Symbol {
                    name,
                    kind: SymbolKind::Module,
                    line: line_no as u32,
                    parent: None,
                    detail: String::from(trimmed),
                });
            }
            continue;
        }

        // const / static
        if trimmed.starts_with("pub const ") || trimmed.starts_with("const ")
            || trimmed.starts_with("pub static ") || trimmed.starts_with("static ")
        {
            let kw = if trimmed.contains("const ") { "const" } else { "static" };
            if let Some(name) = extract_ident_after(trimmed, kw) {
                let name = String::from(name.trim_end_matches(':'));
                symbols.push(Symbol {
                    name,
                    kind: SymbolKind::Constant,
                    line: line_no as u32,
                    parent: current_impl.clone(),
                    detail: String::from(trimmed.split('=').next().unwrap_or(trimmed).trim()),
                });
            }
            continue;
        }

        // type alias
        if trimmed.starts_with("pub type ") || trimmed.starts_with("type ") {
            if let Some(name) = extract_ident_after(trimmed, "type") {
                let name = String::from(name.trim_end_matches('=').trim_end_matches('<'));
                symbols.push(Symbol {
                    name,
                    kind: SymbolKind::TypeAlias,
                    line: line_no as u32,
                    parent: None,
                    detail: String::from(trimmed.trim_end_matches(';').trim()),
                });
            }
            continue;
        }

        // macro_rules!
        if trimmed.starts_with("macro_rules!") {
            let rest = &trimmed["macro_rules!".len()..].trim();
            let name_end = rest.find(|c: char| !c.is_alphanumeric() && c != '_').unwrap_or(rest.len());
            if name_end > 0 {
                symbols.push(Symbol {
                    name: String::from(&rest[..name_end]),
                    kind: SymbolKind::Macro,
                    line: line_no as u32,
                    parent: None,
                    detail: format!("macro_rules! {}", &rest[..name_end]),
                });
            }
        }
    }
    symbols
}

fn try_extract_rust_fn(line: &str) -> Option<(String, String)> {
    // Match: (pub )?(async )?(unsafe )?fn name
    let fn_pos = line.find(" fn ").or_else(|| {
        if line.starts_with("fn ") { Some(0) } else { None }
    })?;

    let after_fn = if line.starts_with("fn ") {
        &line[3..]
    } else {
        &line[fn_pos + 4..]
    };

    let name_end = after_fn.find(|c: char| !c.is_alphanumeric() && c != '_').unwrap_or(after_fn.len());
    if name_end == 0 {
        return None;
    }
    let name = String::from(&after_fn[..name_end]);

    // Build detail up to the opening brace
    let detail = line.split('{').next().unwrap_or(line).trim();
    Some((name, String::from(detail)))
}

fn extract_impl_name(line: &str) -> String {
    // impl<T> Foo<T> for Bar { → "Foo for Bar" or just "Foo"
    let without_impl = line.trim_start_matches("impl").trim();

    // Skip generic params
    let rest = if without_impl.starts_with('<') {
        skip_angle_brackets(without_impl)
    } else {
        without_impl
    };

    let rest = rest.trim();
    let end = rest.find(|c: char| c == '{' || c == '<').unwrap_or(rest.len());
    let name_part = rest[..end].trim();

    // Check for "for" keyword  →  "TraitName for TypeName"
    String::from(name_part)
}

fn skip_angle_brackets(s: &str) -> &str {
    let mut depth = 0;
    for (i, c) in s.chars().enumerate() {
        match c {
            '<' => depth += 1,
            '>' => {
                depth -= 1;
                if depth == 0 {
                    return &s[i + 1..];
                }
            }
            _ => {}
        }
    }
    s
}

fn extract_ident_after(line: &str, keyword: &str) -> Option<String> {
    let pos = line.find(keyword)?;
    let after = &line[pos + keyword.len()..].trim_start();
    let end = after.find(|c: char| !c.is_alphanumeric() && c != '_').unwrap_or(after.len());
    if end == 0 {
        return None;
    }
    Some(String::from(&after[..end]))
}

// ── C/C++ symbol extraction ────────────────────────────────────

fn extract_c_symbols(content: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();

    for (line_no, line) in content.split('\n').enumerate() {
        let trimmed = line.trim();

        if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.is_empty() {
            continue;
        }

        // Function definitions: "type name(..."  — heuristic
        if let Some(sym) = try_extract_c_function(trimmed, line_no as u32) {
            symbols.push(sym);
            continue;
        }

        // struct/class/enum definitions
        if trimmed.starts_with("struct ") || trimmed.starts_with("typedef struct") {
            if let Some(name) = extract_c_type_name(trimmed, "struct") {
                symbols.push(Symbol {
                    name,
                    kind: SymbolKind::Struct,
                    line: line_no as u32,
                    parent: None,
                    detail: String::from(trimmed.split('{').next().unwrap_or(trimmed).trim()),
                });
            }
        } else if trimmed.starts_with("enum ") || trimmed.starts_with("typedef enum") {
            if let Some(name) = extract_c_type_name(trimmed, "enum") {
                symbols.push(Symbol {
                    name,
                    kind: SymbolKind::Enum,
                    line: line_no as u32,
                    parent: None,
                    detail: String::from(trimmed.split('{').next().unwrap_or(trimmed).trim()),
                });
            }
        } else if trimmed.starts_with("class ") {
            if let Some(name) = extract_ident_after(trimmed, "class") {
                symbols.push(Symbol {
                    name,
                    kind: SymbolKind::Class,
                    line: line_no as u32,
                    parent: None,
                    detail: String::from(trimmed.split('{').next().unwrap_or(trimmed).trim()),
                });
            }
        }

        // #define NAME
        if trimmed.starts_with("#define ") {
            let rest = &trimmed[8..];
            let end = rest.find(|c: char| !c.is_alphanumeric() && c != '_').unwrap_or(rest.len());
            if end > 0 {
                symbols.push(Symbol {
                    name: String::from(&rest[..end]),
                    kind: SymbolKind::Macro,
                    line: line_no as u32,
                    parent: None,
                    detail: String::from(trimmed),
                });
            }
        }
    }
    symbols
}

fn try_extract_c_function(line: &str, line_no: u32) -> Option<Symbol> {
    // Heuristic: line contains '(' and ')' or just '(', starts with a type
    let paren_pos = line.find('(')?;
    if paren_pos < 2 {
        return None;
    }

    // Must not start with control flow keywords
    let first_word_end = line.find(|c: char| !c.is_alphanumeric() && c != '_' && c != '*').unwrap_or(0);
    let first_word = &line[..first_word_end];
    match first_word {
        "if" | "for" | "while" | "switch" | "return" | "else" | "#define"
        | "#include" | "#ifdef" | "#ifndef" | "typedef" => return None,
        _ => {}
    }

    // Extract function name (word before the parenthesis)
    let before_paren = &line[..paren_pos];
    let name_start = before_paren.rfind(|c: char| !c.is_alphanumeric() && c != '_')
        .map(|p| p + 1)
        .unwrap_or(0);
    let name = &before_paren[name_start..];
    if name.is_empty() || name.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }

    // Must have a return type before the name
    if name_start == 0 {
        return None;
    }

    Some(Symbol {
        name: String::from(name),
        kind: SymbolKind::Function,
        line: line_no,
        parent: None,
        detail: String::from(line.split('{').next().unwrap_or(line).trim()),
    })
}

fn extract_c_type_name(line: &str, keyword: &str) -> Option<String> {
    // "typedef struct { ... } Name;" or "struct Name {"
    if line.contains("typedef") {
        // Look for name after the closing brace: "} Name;"
        if let Some(brace) = line.rfind('}') {
            let after = &line[brace + 1..].trim().trim_end_matches(';').trim();
            if !after.is_empty() {
                return Some(String::from(*after));
            }
        }
    }
    extract_ident_after(line, keyword)
}

// ── Python symbol extraction ───────────────────────────────────

fn extract_python_symbols(content: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    let mut current_class: Option<String> = None;

    for (line_no, line) in content.split('\n').enumerate() {
        let trimmed = line.trim();

        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }

        // class Foo:
        if trimmed.starts_with("class ") {
            let rest = &trimmed[6..];
            let name_end = rest.find(|c: char| c == '(' || c == ':' || c == ' ').unwrap_or(rest.len());
            if name_end > 0 {
                let name = String::from(&rest[..name_end]);
                current_class = Some(name.clone());
                symbols.push(Symbol {
                    name,
                    kind: SymbolKind::Class,
                    line: line_no as u32,
                    parent: None,
                    detail: String::from(trimmed.trim_end_matches(':')),
                });
            }
            continue;
        }

        // def foo(...)
        if trimmed.starts_with("def ") || trimmed.starts_with("async def ") {
            let rest = if trimmed.starts_with("async ") {
                &trimmed[10..]
            } else {
                &trimmed[4..]
            };
            let name_end = rest.find('(').unwrap_or(rest.len());
            if name_end > 0 {
                let name = String::from(&rest[..name_end]);
                let is_method = !line.starts_with("def") && !line.starts_with("async def");
                symbols.push(Symbol {
                    name,
                    kind: if is_method { SymbolKind::Method } else { SymbolKind::Function },
                    line: line_no as u32,
                    parent: if is_method { current_class.clone() } else { None },
                    detail: String::from(trimmed.trim_end_matches(':')),
                });
            }
            continue;
        }

        // Reset class context at top-level non-indented lines
        if !line.starts_with(' ') && !line.starts_with('\t') {
            current_class = None;
        }
    }
    symbols
}

// ── JavaScript/TypeScript symbol extraction ────────────────────

fn extract_js_symbols(content: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();

    for (line_no, line) in content.split('\n').enumerate() {
        let trimmed = line.trim();

        if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.is_empty() {
            continue;
        }

        // function name(...)
        if trimmed.starts_with("function ") || trimmed.starts_with("async function ") {
            let rest = if trimmed.starts_with("async ") {
                &trimmed[15..]
            } else {
                &trimmed[9..]
            };
            let name_end = rest.find('(').unwrap_or(rest.len());
            if name_end > 0 {
                symbols.push(Symbol {
                    name: String::from(&rest[..name_end]),
                    kind: SymbolKind::Function,
                    line: line_no as u32,
                    parent: None,
                    detail: String::from(trimmed.split('{').next().unwrap_or(trimmed).trim()),
                });
            }
            continue;
        }

        // const name = (...) => or const name = function
        if (trimmed.starts_with("const ") || trimmed.starts_with("let ") || trimmed.starts_with("var "))
            && (trimmed.contains("=>") || trimmed.contains("function"))
        {
            let rest = trimmed.split_whitespace().nth(1).unwrap_or("");
            let name = rest.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '_');
            if !name.is_empty() {
                symbols.push(Symbol {
                    name: String::from(name),
                    kind: SymbolKind::Function,
                    line: line_no as u32,
                    parent: None,
                    detail: String::from(trimmed.split('{').next().unwrap_or(trimmed).trim()),
                });
            }
            continue;
        }

        // class Name
        if trimmed.starts_with("class ") || trimmed.starts_with("export class ") {
            if let Some(name) = extract_ident_after(trimmed, "class") {
                symbols.push(Symbol {
                    name,
                    kind: SymbolKind::Class,
                    line: line_no as u32,
                    parent: None,
                    detail: String::from(trimmed.split('{').next().unwrap_or(trimmed).trim()),
                });
            }
            continue;
        }

        // interface Name (TypeScript)
        if trimmed.starts_with("interface ") || trimmed.starts_with("export interface ") {
            if let Some(name) = extract_ident_after(trimmed, "interface") {
                symbols.push(Symbol {
                    name,
                    kind: SymbolKind::Interface,
                    line: line_no as u32,
                    parent: None,
                    detail: String::from(trimmed.split('{').next().unwrap_or(trimmed).trim()),
                });
            }
            continue;
        }

        // type Name = ... (TypeScript)
        if trimmed.starts_with("type ") || trimmed.starts_with("export type ") {
            if let Some(name) = extract_ident_after(trimmed, "type") {
                symbols.push(Symbol {
                    name,
                    kind: SymbolKind::TypeAlias,
                    line: line_no as u32,
                    parent: None,
                    detail: String::from(trimmed.split('=').next().unwrap_or(trimmed).trim()),
                });
            }
        }
    }
    symbols
}

// ── Shell symbol extraction ────────────────────────────────────

fn extract_shell_symbols(content: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();

    for (line_no, line) in content.split('\n').enumerate() {
        let trimmed = line.trim();

        // function name() or name()
        if trimmed.starts_with("function ") {
            let rest = &trimmed[9..];
            let name_end = rest.find(|c: char| c == '(' || c == ' ' || c == '{').unwrap_or(rest.len());
            if name_end > 0 {
                symbols.push(Symbol {
                    name: String::from(&rest[..name_end]),
                    kind: SymbolKind::Function,
                    line: line_no as u32,
                    parent: None,
                    detail: String::from(trimmed),
                });
            }
        } else if trimmed.contains("()") && !trimmed.starts_with('#') {
            let paren_pos = trimmed.find("()").unwrap();
            let name = &trimmed[..paren_pos].trim();
            if !name.is_empty() && !name.contains(' ') {
                symbols.push(Symbol {
                    name: String::from(*name),
                    kind: SymbolKind::Function,
                    line: line_no as u32,
                    parent: None,
                    detail: String::from(trimmed),
                });
            }
        }
    }
    symbols
}

// ── Makefile symbol extraction ─────────────────────────────────

fn extract_makefile_symbols(content: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();

    for (line_no, line) in content.split('\n').enumerate() {
        let trimmed = line.trim();

        // Target: "name: deps"
        if let Some(colon) = trimmed.find(':') {
            if colon == 0 || trimmed.starts_with('#') || trimmed.starts_with('\t')
                || trimmed.contains('=') || trimmed.starts_with('.')
            {
                continue;
            }
            let target = &trimmed[..colon].trim();
            if !target.contains('$') && !target.contains('%') && !target.is_empty() {
                symbols.push(Symbol {
                    name: String::from(*target),
                    kind: SymbolKind::Function,
                    line: line_no as u32,
                    parent: None,
                    detail: String::from(trimmed),
                });
            }
        }
    }
    symbols
}
