use alloc::vec::Vec;

// ════════════════════════════════════════════════════════════════
//  Language definitions and registry
// ════════════════════════════════════════════════════════════════

/// Unique language identifier.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum LanguageId {
    Rust,
    C,
    Cpp,
    Python,
    JavaScript,
    TypeScript,
    Shell,
    Makefile,
    Json,
    Toml,
    Yaml,
    Xml,
    Html,
    Css,
    Markdown,
    Assembly,
    LinkerScript,
    Ini,
    PlainText,
}

impl LanguageId {
    /// Display name for status bar and UI.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Rust => "Rust",
            Self::C => "C",
            Self::Cpp => "C++",
            Self::Python => "Python",
            Self::JavaScript => "JavaScript",
            Self::TypeScript => "TypeScript",
            Self::Shell => "Shell",
            Self::Makefile => "Makefile",
            Self::Json => "JSON",
            Self::Toml => "TOML",
            Self::Yaml => "YAML",
            Self::Xml => "XML",
            Self::Html => "HTML",
            Self::Css => "CSS",
            Self::Markdown => "Markdown",
            Self::Assembly => "Assembly",
            Self::LinkerScript => "Linker Script",
            Self::Ini => "INI",
            Self::PlainText => "Plain Text",
        }
    }

    /// Icon color for the file explorer tree.
    pub fn icon_color(&self) -> u32 {
        match self {
            Self::Rust => 0xFFDEA584,          // Rust orange
            Self::C | Self::Cpp => 0xFF569CD6, // Blue
            Self::Python => 0xFF4EC9B0,        // Teal
            Self::JavaScript => 0xFFDCDCAA,    // Yellow
            Self::TypeScript => 0xFF569CD6,    // Blue
            Self::Shell => 0xFF6A9955,         // Green
            Self::Makefile => 0xFFDCDCAA,      // Yellow
            Self::Json => 0xFFCE9178,          // Orange
            Self::Toml => 0xFF9CDCFE,          // Light blue
            Self::Yaml => 0xFF9CDCFE,          // Light blue
            Self::Html => 0xFFE06C75,          // Red
            Self::Css => 0xFF56B6C2,           // Cyan
            Self::Markdown => 0xFF9CDCFE,      // Light blue
            Self::Assembly => 0xFFD19A66,      // Dark orange
            Self::Xml => 0xFFE06C75,           // Red
            _ => 0,
        }
    }

    /// Whether this language supports comment toggling (line comment prefix).
    pub fn line_comment(&self) -> Option<&'static str> {
        match self {
            Self::Rust | Self::C | Self::Cpp | Self::JavaScript | Self::TypeScript => Some("//"),
            Self::Python | Self::Shell => Some("#"),
            Self::Makefile => Some("#"),
            Self::Toml | Self::Yaml | Self::Ini => Some("#"),
            Self::Assembly => Some(";"),
            Self::Html | Self::Xml | Self::Css => None, // block comments only
            _ => None,
        }
    }

    /// Block comment delimiters.
    pub fn block_comment(&self) -> Option<(&'static str, &'static str)> {
        match self {
            Self::Rust | Self::C | Self::Cpp | Self::JavaScript | Self::TypeScript | Self::Css => {
                Some(("/*", "*/"))
            }
            Self::Html | Self::Xml => Some(("<!--", "-->")),
            Self::Python => Some(("\"\"\"", "\"\"\"")),
            _ => None,
        }
    }

    /// Auto-close bracket pairs for this language.
    pub fn bracket_pairs(&self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Rust | Self::C | Self::Cpp | Self::JavaScript | Self::TypeScript | Self::Css => {
                &[("{", "}"), ("(", ")"), ("[", "]"), ("\"", "\""), ("'", "'")]
            }
            Self::Python => &[("{", "}"), ("(", ")"), ("[", "]"), ("\"", "\""), ("'", "'")],
            Self::Html | Self::Xml => &[("<", ">"), ("\"", "\""), ("'", "'")],
            Self::Json => &[("{", "}"), ("[", "]"), ("\"", "\"")],
            _ => &[("{", "}"), ("(", ")"), ("[", "]"), ("\"", "\"")],
        }
    }
}

// ════════════════════════════════════════════════════════════════
//  Language information struct
// ════════════════════════════════════════════════════════════════

/// Extended language information for IDE features.
pub struct LanguageInfo {
    pub id: LanguageId,
    pub extensions: &'static [&'static str],
    pub filenames: &'static [&'static str],
    /// Common keywords for basic completion.
    pub keywords: &'static [&'static str],
    /// Common snippets (trigger → expansion).
    pub snippets: &'static [(&'static str, &'static str)],
    /// Syntax file name (from bundle).
    pub syntax_file: Option<&'static str>,
}

// ════════════════════════════════════════════════════════════════
//  Built-in language definitions
// ════════════════════════════════════════════════════════════════

static LANG_RUST: LanguageInfo = LanguageInfo {
    id: LanguageId::Rust,
    extensions: &["rs"],
    filenames: &[],
    keywords: &[
        "fn", "let", "mut", "const", "static", "struct", "enum", "impl", "trait", "pub", "use",
        "mod", "crate", "self", "super", "extern", "type", "where", "if", "else", "match", "for",
        "while", "loop", "break", "continue", "return", "async", "await", "move", "unsafe", "dyn",
        "ref", "as", "in", "true", "false", "Some", "None", "Ok", "Err", "Self", "u8", "u16",
        "u32", "u64", "usize", "i8", "i16", "i32", "i64", "isize", "f32", "f64", "bool", "char",
        "str", "String", "Vec", "Option", "Result", "Box", "Rc", "Arc", "HashMap", "HashSet",
        "BTreeMap",
    ],
    snippets: &[
        ("fn", "fn ${1:name}(${2}) {\n    ${0}\n}"),
        ("pfn", "pub fn ${1:name}(${2}) {\n    ${0}\n}"),
        ("struct", "struct ${1:Name} {\n    ${0}\n}"),
        ("enum", "enum ${1:Name} {\n    ${0}\n}"),
        ("impl", "impl ${1:Type} {\n    ${0}\n}"),
        ("test", "#[test]\nfn ${1:test_name}() {\n    ${0}\n}"),
        ("match", "match ${1:expr} {\n    ${0}\n}"),
        ("if let", "if let ${1:Some(val)} = ${2:expr} {\n    ${0}\n}"),
        ("println", "anyos_std::println!(\"${0}\");"),
        ("derive", "#[derive(${0})]"),
    ],
    syntax_file: Some("rust.syn"),
};

static LANG_C: LanguageInfo = LanguageInfo {
    id: LanguageId::C,
    extensions: &["c", "h"],
    filenames: &[],
    keywords: &[
        "auto", "break", "case", "char", "const", "continue", "default", "do", "double", "else",
        "enum", "extern", "float", "for", "goto", "if", "int", "long", "register", "return",
        "short", "signed", "sizeof", "static", "struct", "switch", "typedef", "union", "unsigned",
        "void", "volatile", "while", "NULL", "stdin", "stdout", "stderr", "FILE", "size_t",
        "uint8_t", "uint16_t", "uint32_t", "uint64_t", "int8_t", "int16_t", "int32_t", "int64_t",
        "#include", "#define", "#ifdef", "#ifndef", "#endif", "#pragma", "printf", "fprintf",
        "sprintf", "scanf", "malloc", "calloc", "realloc", "free", "memcpy", "memset", "strlen",
        "strcmp", "strcpy", "strcat",
    ],
    snippets: &[
        (
            "main",
            "int main(int argc, char *argv[]) {\n    ${0}\n    return 0;\n}",
        ),
        (
            "for",
            "for (int ${1:i} = 0; ${1:i} < ${2:n}; ${1:i}++) {\n    ${0}\n}",
        ),
        ("while", "while (${1:cond}) {\n    ${0}\n}"),
        ("if", "if (${1:cond}) {\n    ${0}\n}"),
        ("struct", "typedef struct {\n    ${0}\n} ${1:Name};"),
        ("include", "#include <${0}>"),
        (
            "guard",
            "#ifndef ${1:HEADER}_H\n#define ${1:HEADER}_H\n\n${0}\n\n#endif",
        ),
    ],
    syntax_file: Some("c.syn"),
};

static LANG_CPP: LanguageInfo = LanguageInfo {
    id: LanguageId::Cpp,
    extensions: &["cpp", "cc", "cxx", "hpp", "hxx", "hh"],
    filenames: &[],
    keywords: &[
        "class",
        "namespace",
        "template",
        "typename",
        "virtual",
        "override",
        "final",
        "public",
        "private",
        "protected",
        "new",
        "delete",
        "this",
        "nullptr",
        "bool",
        "true",
        "false",
        "auto",
        "decltype",
        "constexpr",
        "noexcept",
        "try",
        "catch",
        "throw",
        "using",
        "operator",
        "friend",
        "mutable",
        "std",
        "string",
        "vector",
        "map",
        "set",
        "unique_ptr",
        "shared_ptr",
        "cout",
        "cin",
        "endl",
        "cerr",
    ],
    snippets: &[
        ("class", "class ${1:Name} {\npublic:\n    ${0}\n};"),
        (
            "main",
            "int main(int argc, char* argv[]) {\n    ${0}\n    return 0;\n}",
        ),
    ],
    syntax_file: Some("c.syn"),
};

static LANG_PYTHON: LanguageInfo = LanguageInfo {
    id: LanguageId::Python,
    extensions: &["py", "pyw", "pyi"],
    filenames: &[],
    keywords: &[
        "def",
        "class",
        "if",
        "elif",
        "else",
        "for",
        "while",
        "break",
        "continue",
        "return",
        "yield",
        "import",
        "from",
        "as",
        "try",
        "except",
        "finally",
        "raise",
        "with",
        "pass",
        "lambda",
        "global",
        "nonlocal",
        "assert",
        "del",
        "True",
        "False",
        "None",
        "and",
        "or",
        "not",
        "in",
        "is",
        "self",
        "cls",
        "super",
        "print",
        "len",
        "range",
        "enumerate",
        "zip",
        "list",
        "dict",
        "set",
        "tuple",
        "str",
        "int",
        "float",
        "bool",
        "open",
        "input",
        "type",
        "isinstance",
        "hasattr",
        "getattr",
        "setattr",
    ],
    snippets: &[
        ("def", "def ${1:name}(${2:self}):\n    ${0}"),
        (
            "class",
            "class ${1:Name}:\n    def __init__(self${2}):\n        ${0}",
        ),
        ("if", "if ${1:cond}:\n    ${0}"),
        ("for", "for ${1:item} in ${2:items}:\n    ${0}"),
        ("main", "if __name__ == \"__main__\":\n    ${0}"),
        ("with", "with ${1:ctx} as ${2:var}:\n    ${0}"),
    ],
    syntax_file: Some("python.syn"),
};

static LANG_JAVASCRIPT: LanguageInfo = LanguageInfo {
    id: LanguageId::JavaScript,
    extensions: &["js", "mjs", "cjs", "jsx"],
    filenames: &[],
    keywords: &[
        "function",
        "const",
        "let",
        "var",
        "if",
        "else",
        "for",
        "while",
        "do",
        "switch",
        "case",
        "break",
        "continue",
        "return",
        "throw",
        "try",
        "catch",
        "finally",
        "class",
        "extends",
        "new",
        "this",
        "super",
        "import",
        "export",
        "default",
        "from",
        "async",
        "await",
        "yield",
        "typeof",
        "instanceof",
        "true",
        "false",
        "null",
        "undefined",
        "NaN",
        "Infinity",
        "console",
        "document",
        "window",
        "Array",
        "Object",
        "String",
        "Number",
        "Promise",
        "Map",
        "Set",
        "JSON",
        "Math",
        "Date",
        "RegExp",
        "Error",
        "setTimeout",
        "setInterval",
        "fetch",
        "addEventListener",
    ],
    snippets: &[
        ("fn", "function ${1:name}(${2}) {\n    ${0}\n}"),
        ("afn", "const ${1:name} = (${2}) => {\n    ${0}\n};"),
        (
            "class",
            "class ${1:Name} {\n    constructor(${2}) {\n        ${0}\n    }\n}",
        ),
        ("if", "if (${1:cond}) {\n    ${0}\n}"),
        (
            "for",
            "for (let ${1:i} = 0; ${1:i} < ${2:n}; ${1:i}++) {\n    ${0}\n}",
        ),
        ("log", "console.log(${0});"),
    ],
    syntax_file: None,
};

static LANG_TYPESCRIPT: LanguageInfo = LanguageInfo {
    id: LanguageId::TypeScript,
    extensions: &["ts", "tsx"],
    filenames: &[],
    keywords: &[
        "interface",
        "type",
        "enum",
        "namespace",
        "declare",
        "module",
        "abstract",
        "implements",
        "readonly",
        "private",
        "protected",
        "public",
        "keyof",
        "infer",
        "extends",
        "as",
        "is",
        "in",
        "out",
        "string",
        "number",
        "boolean",
        "void",
        "never",
        "unknown",
        "any",
        "object",
        "Record",
        "Partial",
        "Required",
        "Pick",
        "Omit",
        "Exclude",
        "Extract",
    ],
    snippets: &[
        ("interface", "interface ${1:Name} {\n    ${0}\n}"),
        ("type", "type ${1:Name} = ${0};"),
    ],
    syntax_file: None,
};

static LANG_SHELL: LanguageInfo = LanguageInfo {
    id: LanguageId::Shell,
    extensions: &["sh", "bash", "zsh"],
    filenames: &[".bashrc", ".zshrc", ".bash_profile", ".profile"],
    keywords: &[
        "if", "then", "else", "elif", "fi", "for", "do", "done", "while", "until", "case", "esac",
        "function", "return", "exit", "break", "continue", "echo", "printf", "read", "local",
        "export", "unset", "shift", "test", "true", "false", "set", "source", "eval", "cd", "ls",
        "cp", "mv", "rm", "mkdir", "chmod", "chown", "grep", "sed", "awk", "find", "sort", "cut",
        "tr", "wc", "cat", "head", "tail", "tee", "xargs", "pipe",
    ],
    snippets: &[
        ("if", "if [ ${1:cond} ]; then\n    ${0}\nfi"),
        ("for", "for ${1:var} in ${2:list}; do\n    ${0}\ndone"),
        ("fn", "${1:name}() {\n    ${0}\n}"),
        ("shebang", "#!/bin/sh\n${0}"),
    ],
    syntax_file: Some("sh.syn"),
};

static LANG_MAKEFILE: LanguageInfo = LanguageInfo {
    id: LanguageId::Makefile,
    extensions: &["mk"],
    filenames: &["Makefile", "makefile", "GNUmakefile"],
    keywords: &[
        ".PHONY",
        ".SUFFIXES",
        ".DEFAULT",
        ".PRECIOUS",
        ".INTERMEDIATE",
        "all",
        "clean",
        "install",
        "test",
        "check",
        "dist",
        "CC",
        "CXX",
        "CFLAGS",
        "CXXFLAGS",
        "LDFLAGS",
        "LIBS",
        "AR",
        "ARFLAGS",
        "RM",
        "MAKE",
        "SHELL",
        "$(wildcard",
        "$(patsubst",
        "$(foreach",
        "$(if",
        "$(call",
        "$(shell",
        "$(dir",
        "$(notdir",
        "$(basename",
        "$(suffix",
        "@",
        "$@",
        "$<",
        "$^",
        "$?",
        "$*",
    ],
    snippets: &[
        ("target", "${1:target}: ${2:deps}\n\t${0}"),
        ("phony", ".PHONY: ${1:target}\n${1:target}:\n\t${0}"),
        ("var", "${1:VAR} := ${0}"),
    ],
    syntax_file: Some("makefile.syn"),
};

static LANG_JSON: LanguageInfo = LanguageInfo {
    id: LanguageId::Json,
    extensions: &["json", "jsonc"],
    filenames: &[],
    keywords: &["true", "false", "null"],
    snippets: &[],
    syntax_file: Some("json.syn"),
};

static LANG_TOML: LanguageInfo = LanguageInfo {
    id: LanguageId::Toml,
    extensions: &["toml"],
    filenames: &["Cargo.toml", "pyproject.toml"],
    keywords: &["true", "false"],
    snippets: &[],
    syntax_file: None,
};

static LANG_YAML: LanguageInfo = LanguageInfo {
    id: LanguageId::Yaml,
    extensions: &["yaml", "yml"],
    filenames: &[],
    keywords: &["true", "false", "null", "yes", "no"],
    snippets: &[],
    syntax_file: None,
};

static LANG_XML: LanguageInfo = LanguageInfo {
    id: LanguageId::Xml,
    extensions: &["xml", "xsl", "xslt", "svg"],
    filenames: &[],
    keywords: &[],
    snippets: &[],
    syntax_file: None,
};

static LANG_HTML: LanguageInfo = LanguageInfo {
    id: LanguageId::Html,
    extensions: &["html", "htm", "xhtml"],
    filenames: &[],
    keywords: &[
        "html", "head", "body", "div", "span", "p", "a", "img", "ul", "ol", "li",
        "table", "tr", "td", "th", "form", "input", "button", "select", "option",
        "h1", "h2", "h3", "h4", "h5", "h6", "header", "footer", "nav", "main",
        "section", "article", "aside", "script", "style", "link", "meta", "title",
    ],
    snippets: &[
        ("html5", "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n    <meta charset=\"UTF-8\">\n    <title>${1:Title}</title>\n</head>\n<body>\n    ${0}\n</body>\n</html>"),
        ("div", "<div class=\"${1:class}\">\n    ${0}\n</div>"),
    ],
    syntax_file: None,
};

static LANG_CSS: LanguageInfo = LanguageInfo {
    id: LanguageId::Css,
    extensions: &["css", "scss", "less"],
    filenames: &[],
    keywords: &[
        "display",
        "position",
        "width",
        "height",
        "margin",
        "padding",
        "border",
        "background",
        "color",
        "font",
        "text",
        "flex",
        "grid",
        "align",
        "justify",
        "overflow",
        "opacity",
        "transform",
        "transition",
        "animation",
        "none",
        "block",
        "inline",
        "flex",
        "grid",
        "absolute",
        "relative",
        "fixed",
        "auto",
        "inherit",
        "initial",
        "unset",
        "@media",
        "@keyframes",
        "@import",
        "@font-face",
    ],
    snippets: &[
        (
            "flex",
            "display: flex;\njustify-content: ${1:center};\nalign-items: ${2:center};",
        ),
        (
            "grid",
            "display: grid;\ngrid-template-columns: ${1:1fr 1fr};",
        ),
    ],
    syntax_file: None,
};

static LANG_MARKDOWN: LanguageInfo = LanguageInfo {
    id: LanguageId::Markdown,
    extensions: &["md", "markdown"],
    filenames: &["README", "CHANGELOG", "LICENSE"],
    keywords: &[],
    snippets: &[
        ("link", "[${1:text}](${0:url})"),
        ("img", "![${1:alt}](${0:url})"),
        ("code", "```${1:lang}\n${0}\n```"),
        (
            "table",
            "| ${1:Header} | ${2:Header} |\n|---|---|\n| ${0} | |",
        ),
    ],
    syntax_file: None,
};

static LANG_ASSEMBLY: LanguageInfo = LanguageInfo {
    id: LanguageId::Assembly,
    extensions: &["asm", "s", "S"],
    filenames: &[],
    keywords: &[
        "mov", "add", "sub", "mul", "div", "push", "pop", "call", "ret", "jmp", "je", "jne", "jz",
        "jnz", "jg", "jl", "jge", "jle", "cmp", "test", "and", "or", "xor", "not", "shl", "shr",
        "lea", "nop", "int", "iret", "section", "global", "extern", "db", "dw", "dd", "dq", "equ",
        "times", "eax", "ebx", "ecx", "edx", "esi", "edi", "esp", "ebp", "rax", "rbx", "rcx",
        "rdx", "rsi", "rdi", "rsp", "rbp",
    ],
    snippets: &[],
    syntax_file: None,
};

static LANG_LINKER_SCRIPT: LanguageInfo = LanguageInfo {
    id: LanguageId::LinkerScript,
    extensions: &["ld", "lds"],
    filenames: &[],
    keywords: &[
        "SECTIONS", "MEMORY", "ENTRY", "PROVIDE", "ALIGN", "AT", "LOADADDR", ".text", ".data",
        ".bss", ".rodata",
    ],
    snippets: &[],
    syntax_file: None,
};

static LANG_INI: LanguageInfo = LanguageInfo {
    id: LanguageId::Ini,
    extensions: &["ini", "conf", "cfg"],
    filenames: &[],
    keywords: &["true", "false", "yes", "no"],
    snippets: &[],
    syntax_file: None,
};

static LANG_PLAIN_TEXT: LanguageInfo = LanguageInfo {
    id: LanguageId::PlainText,
    extensions: &["txt", "log"],
    filenames: &[],
    keywords: &[],
    snippets: &[],
    syntax_file: None,
};

// ════════════════════════════════════════════════════════════════
//  Language registry
// ════════════════════════════════════════════════════════════════

/// All built-in languages, ordered for lookup priority.
static ALL_LANGUAGES: &[&LanguageInfo] = &[
    &LANG_RUST,
    &LANG_C,
    &LANG_CPP,
    &LANG_PYTHON,
    &LANG_JAVASCRIPT,
    &LANG_TYPESCRIPT,
    &LANG_SHELL,
    &LANG_MAKEFILE,
    &LANG_JSON,
    &LANG_TOML,
    &LANG_YAML,
    &LANG_XML,
    &LANG_HTML,
    &LANG_CSS,
    &LANG_MARKDOWN,
    &LANG_ASSEMBLY,
    &LANG_LINKER_SCRIPT,
    &LANG_INI,
    &LANG_PLAIN_TEXT,
];

/// Look up a language by file extension.
pub fn language_for_extension(ext: &str) -> &'static LanguageInfo {
    for lang in ALL_LANGUAGES {
        if lang.extensions.contains(&ext) {
            return lang;
        }
    }
    &LANG_PLAIN_TEXT
}

/// Look up a language by filename (checks special filenames first, then extension).
pub fn language_for_filename(filename: &str) -> &'static LanguageInfo {
    // Check special filenames first
    for lang in ALL_LANGUAGES {
        if lang.filenames.contains(&filename) {
            return lang;
        }
    }

    // Fall back to extension
    if let Some(dot_pos) = filename.rfind('.') {
        let ext = &filename[dot_pos + 1..];
        language_for_extension(ext)
    } else {
        &LANG_PLAIN_TEXT
    }
}

/// Get all registered languages for extension browser / settings.
pub fn all_languages() -> &'static [&'static LanguageInfo] {
    ALL_LANGUAGES
}

/// Look up a language by its stable id.
pub fn info_for_id(id: LanguageId) -> Option<&'static LanguageInfo> {
    ALL_LANGUAGES.iter().copied().find(|lang| lang.id == id)
}

/// Get keywords for the current language (for basic completion).
pub fn keywords_for_file(filename: &str) -> &'static [&'static str] {
    language_for_filename(filename).keywords
}

/// Get snippets for the current language.
pub fn snippets_for_file(filename: &str) -> &'static [(&'static str, &'static str)] {
    language_for_filename(filename).snippets
}
