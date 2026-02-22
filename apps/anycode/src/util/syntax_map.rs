use alloc::string::String;
use alloc::format;

/// Map a file extension to its syntax definition filename.
fn syntax_filename_for_extension(ext: &str) -> Option<&'static str> {
    match ext {
        "c" | "h" => Some("c.syn"),
        "rs" => Some("rust.syn"),
        "json" => Some("json.syn"),
        "py" => Some("python.syn"),
        "sh" | "bash" => Some("sh.syn"),
        "mk" => Some("makefile.syn"),
        _ => None,
    }
}

/// Map a filename to its syntax definition file path,
/// using the configured syntax directory.
pub fn syntax_for_filename(syntax_dir: &str, filename: &str) -> Option<String> {
    if filename == "Makefile" || filename == "makefile" || filename == "GNUmakefile" {
        return Some(format!("{}/makefile.syn", syntax_dir));
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
    let ext = match filename.rsplit('.').next() {
        Some(e) => e,
        None => return "Plain Text",
    };
    match ext {
        "c" | "h" => "C",
        "rs" => "Rust",
        "json" => "JSON",
        "py" => "Python",
        "sh" | "bash" => "Shell",
        "mk" => "Makefile",
        "txt" => "Plain Text",
        "md" => "Markdown",
        "toml" => "TOML",
        "yaml" | "yml" => "YAML",
        "xml" => "XML",
        "html" | "htm" => "HTML",
        "css" => "CSS",
        "js" => "JavaScript",
        "ts" => "TypeScript",
        "asm" | "s" | "S" => "Assembly",
        "ld" => "Linker Script",
        _ => "Plain Text",
    }
}
