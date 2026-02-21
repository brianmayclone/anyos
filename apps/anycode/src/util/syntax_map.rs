/// Map a file extension to its syntax definition file path.
pub fn syntax_path_for_extension(ext: &str) -> Option<&'static str> {
    match ext {
        "c" | "h" => Some("/System/syntax/c.syn"),
        "rs" => Some("/System/syntax/rust.syn"),
        "json" => Some("/System/syntax/json.syn"),
        "py" => Some("/System/syntax/python.syn"),
        "sh" | "bash" => Some("/System/syntax/sh.syn"),
        "mk" => Some("/System/syntax/makefile.syn"),
        _ => None,
    }
}

/// Map a filename to its syntax definition file path.
/// Handles special filenames (Makefile) and falls back to extension matching.
pub fn syntax_for_filename(filename: &str) -> Option<&'static str> {
    if filename == "Makefile" || filename == "makefile" || filename == "GNUmakefile" {
        return Some("/System/syntax/makefile.syn");
    }
    let ext = filename.rsplit('.').next()?;
    syntax_path_for_extension(ext)
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
