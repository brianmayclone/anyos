fn parse_value_ast(input: &str) -> CssValueAst {
    CssValueAst {
        raw: String::from(input.trim()),
        components: parse_value_components_ast(input.trim()),
    }
}

fn parse_value_components_ast(input: &str) -> Vec<CssValueComponentAst> {
    let bytes = input.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;

    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' | b'\n' | b'\r' => i += 1,
            b',' => {
                out.push(CssValueComponentAst::Comma);
                i += 1;
            }
            b'/' => {
                out.push(CssValueComponentAst::Slash);
                i += 1;
            }
            b'#' => {
                let start = i;
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_hexdigit() {
                    i += 1;
                }
                out.push(CssValueComponentAst::Hash(String::from(&input[start..i])));
            }
            b'"' | b'\'' => {
                let value = read_quoted_string_at(input, &mut i);
                out.push(CssValueComponentAst::String(value));
            }
            b'+' | b'-' | b'.' | b'0'..=b'9' => {
                if let Some((component, next_i)) = parse_numeric_component_ast(input, i) {
                    out.push(component);
                    i = next_i;
                } else {
                    out.push(CssValueComponentAst::Delim(bytes[i] as char));
                    i += 1;
                }
            }
            _ if is_ident_start(bytes[i]) || bytes[i] == b'\\' => {
                let start = i;
                let ident = read_ident_at(input, &mut i);
                if i < bytes.len() && bytes[i] == b'(' {
                    let args = parse_function_arguments_ast(input, &mut i);
                    out.push(CssValueComponentAst::Function { name: ident, args });
                } else if !ident.is_empty() {
                    out.push(CssValueComponentAst::Ident(String::from(&input[start..i])));
                }
            }
            other => {
                out.push(CssValueComponentAst::Delim(other as char));
                i += 1;
            }
        }
    }

    out
}

fn parse_numeric_component_ast(input: &str, start: usize) -> Option<(CssValueComponentAst, usize)> {
    let bytes = input.as_bytes();
    let mut i = start;
    if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
        i += 1;
    }
    let number_start = start;
    let mut has_digit = false;
    let mut seen_dot = false;
    while i < bytes.len() {
        match bytes[i] {
            b'0'..=b'9' => {
                has_digit = true;
                i += 1;
            }
            b'.' if !seen_dot => {
                seen_dot = true;
                i += 1;
            }
            _ => break,
        }
    }
    if !has_digit {
        return None;
    }

    let mut j = i;
    while j < bytes.len()
        && (is_ident_start(bytes[j]) || bytes[j].is_ascii_digit() || bytes[j] == b'%')
    {
        j += 1;
    }

    if j > i {
        Some((
            CssValueComponentAst::Dimension(String::from(&input[number_start..j])),
            j,
        ))
    } else {
        Some((
            CssValueComponentAst::Number(String::from(&input[number_start..i])),
            i,
        ))
    }
}

fn parse_function_arguments_ast(input: &str, i: &mut usize) -> Vec<CssValueAst> {
    let bytes = input.as_bytes();
    if *i >= bytes.len() || bytes[*i] != b'(' {
        return Vec::new();
    }
    *i += 1;
    let arg_start = *i;
    let mut start = arg_start;
    let mut depth = 0i32;
    let mut string_quote = 0u8;
    let mut args = Vec::new();

    while *i < bytes.len() {
        let ch = bytes[*i];
        if string_quote != 0 {
            if ch == b'\\' {
                *i = (*i + 2).min(bytes.len());
                continue;
            }
            *i += 1;
            if ch == string_quote {
                string_quote = 0;
            }
            continue;
        }

        match ch {
            b'"' | b'\'' => {
                string_quote = ch;
                *i += 1;
            }
            b'(' => {
                depth += 1;
                *i += 1;
            }
            b')' if depth == 0 => {
                let arg = input[start..*i].trim();
                if !arg.is_empty() {
                    args.push(parse_value_ast(arg));
                }
                *i += 1;
                break;
            }
            b')' => {
                depth -= 1;
                *i += 1;
            }
            b',' if depth == 0 => {
                let arg = input[start..*i].trim();
                if !arg.is_empty() {
                    args.push(parse_value_ast(arg));
                }
                *i += 1;
                start = *i;
            }
            _ => *i += 1,
        }
    }

    args
}
