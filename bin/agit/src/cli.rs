use alloc::string::String;

pub fn normalize_raw_args(raw: &str) -> String {
    let mut tokens = anyos_std::args::tokenize(raw);
    if tokens.first().map(|arg| is_git_argv0(arg)).unwrap_or(false) {
        tokens.remove(0);
    }

    let mut out = String::new();
    for (idx, token) in tokens.iter().enumerate() {
        if idx > 0 {
            out.push(' ');
        }
        append_shell_arg(&mut out, token);
    }
    out
}

pub fn has_long_option(tokens: &[String], option: &str) -> bool {
    tokens.iter().any(|token| token == option)
}

fn is_git_argv0(arg: &str) -> bool {
    let name = arg.rsplit('/').next().unwrap_or(arg);
    name == "git" || name == "cgit" || name == "agit"
}

fn append_shell_arg(out: &mut String, arg: &str) {
    if arg.is_empty()
        || arg
            .as_bytes()
            .iter()
            .any(|b| matches!(*b, b' ' | b'\t' | b'"' | b'\'' | b'\\'))
    {
        out.push('"');
        for ch in arg.chars() {
            if ch == '"' || ch == '\\' {
                out.push('\\');
            }
            out.push(ch);
        }
        out.push('"');
    } else {
        out.push_str(arg);
    }
}
