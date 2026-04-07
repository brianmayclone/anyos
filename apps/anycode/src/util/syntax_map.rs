use alloc::string::String;
use alloc::format;

/// Map a file extension to its syntax definition filename.
fn syntax_filename_for_extension(ext: &str) -> Option<&'static str> {
    match ext {
        "c" | "h" => Some("c.syn"),
        "cpp" | "cc" | "cxx" | "hpp" | "hxx" | "hh" => Some("c.syn"),
        "rs" => Some("rust.syn"),
        "json" | "jsonc" => Some("json.syn"),
        "py" | "pyw" | "pyi" => Some("python.syn"),
        "sh" | "bash" | "zsh" => Some("sh.syn"),
        "mk" => Some("makefile.syn"),
        "toml" => Some("toml.syn"),
        "html" | "htm" | "xhtml" => Some("html.syn"),
        "css" | "scss" | "less" => Some("css.syn"),
        "js" | "mjs" | "cjs" | "jsx" => Some("js.syn"),
        "ts" | "tsx" => Some("ts.syn"),
        "xml" | "svg" | "xsl" => Some("xml.syn"),
        "yaml" | "yml" => Some("yaml.syn"),
        "md" | "markdown" => Some("markdown.syn"),
        "asm" | "s" | "S" => Some("asm.syn"),
        "ld" | "lds" => Some("ld.syn"),
        "ini" | "conf" | "cfg" => Some("ini.syn"),
        _ => None,
    }
}

/// Map a filename to its syntax definition file path,
/// using the configured syntax directory.
pub fn syntax_for_filename(syntax_dir: &str, filename: &str) -> Option<String> {
    if filename == "Makefile" || filename == "makefile" || filename == "GNUmakefile" {
        return Some(format!("{}/makefile.syn", syntax_dir));
    }
    if filename == "Cargo.toml" || filename == "Cargo.lock" {
        return Some(format!("{}/toml.syn", syntax_dir));
    }
    if filename == "CMakeLists.txt" {
        return Some(format!("{}/makefile.syn", syntax_dir));
    }
    if filename == ".gitignore" || filename == ".gitattributes" {
        return Some(format!("{}/ini.syn", syntax_dir));
    }
    if filename == "Dockerfile" {
        return Some(format!("{}/sh.syn", syntax_dir));
    }
    let ext = filename.rsplit('.').next()?;
    let syn_file = syntax_filename_for_extension(ext)?;
    Some(format!("{}/{}", syntax_dir, syn_file))
}

/// Get a human-readable language name for a file.
pub fn language_for_filename(filename: &str) -> &'static str {
    if filename == "Makefile" || filename == "makefile" || filename == "GNUmakefile" {
        return "Makefile";
    }
    if filename == "CMakeLists.txt" {
        return "CMake";
    }
    if filename == "Dockerfile" {
        return "Dockerfile";
    }
    let ext = match filename.rsplit('.').next() {
        Some(e) => e,
        None => return "Plain Text",
    };
    match ext {
        "c" | "h" => "C",
        "cpp" | "cc" | "cxx" | "hpp" | "hxx" | "hh" => "C++",
        "rs" => "Rust",
        "json" | "jsonc" => "JSON",
        "py" | "pyw" | "pyi" => "Python",
        "sh" | "bash" | "zsh" => "Shell",
        "mk" => "Makefile",
        "txt" | "text" => "Plain Text",
        "md" | "markdown" => "Markdown",
        "toml" => "TOML",
        "yaml" | "yml" => "YAML",
        "xml" | "xsl" | "xslt" | "svg" => "XML",
        "html" | "htm" | "xhtml" => "HTML",
        "css" | "scss" | "less" => "CSS",
        "js" | "mjs" | "cjs" | "jsx" => "JavaScript",
        "ts" | "tsx" => "TypeScript",
        "asm" | "s" | "S" => "Assembly",
        "ld" | "lds" => "Linker Script",
        "ini" | "conf" | "cfg" => "INI",
        "log" => "Log",
        "def" => "Module Definition",
        _ => "Plain Text",
    }
}
