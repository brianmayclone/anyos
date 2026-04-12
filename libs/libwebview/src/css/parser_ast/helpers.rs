fn skip_ignorable_tokens(tokens: &[CssToken], mut cursor: usize) -> usize {
    while cursor < tokens.len() {
        match tokens[cursor].kind {
            CssTokenKind::Whitespace | CssTokenKind::Comment => cursor += 1,
            _ => break,
        }
    }
    if cursor < tokens.len() {
        tokens[cursor].start
    } else {
        0
    }
}

fn slice_trimmed(input: &str, start: usize, end: usize) -> String {
    if start >= end || start >= input.len() {
        return String::new();
    }
    String::from(input[start..end.min(input.len())].trim())
}

fn read_ident_at(input: &str, i: &mut usize) -> String {
    let bytes = input.as_bytes();
    let mut result = String::new();
    while *i < bytes.len() {
        let ch = bytes[*i];
        if ch == b'\\' && *i + 1 < bytes.len() {
            *i += 1;
            let escaped = bytes[*i];
            if escaped.is_ascii_hexdigit() {
                let hex_start = *i;
                let mut count = 0usize;
                while *i < bytes.len() && bytes[*i].is_ascii_hexdigit() && count < 6 {
                    *i += 1;
                    count += 1;
                }
                if *i < bytes.len() && bytes[*i] == b' ' {
                    *i += 1;
                }
                if let Ok(s) = core::str::from_utf8(&bytes[hex_start..hex_start + count]) {
                    if let Ok(cp) = u32::from_str_radix(s, 16) {
                        if let Some(c) = char::from_u32(cp) {
                            result.push(c);
                            continue;
                        }
                    }
                }
            } else {
                result.push(escaped as char);
                *i += 1;
            }
        } else if ch.is_ascii_alphanumeric() || ch == b'-' || ch == b'_' {
            result.push(ch as char);
            *i += 1;
        } else {
            break;
        }
    }
    result
}

fn read_quoted_string_at(input: &str, i: &mut usize) -> String {
    let bytes = input.as_bytes();
    if *i >= bytes.len() {
        return String::new();
    }
    let quote = bytes[*i];
    *i += 1;
    let start = *i;
    while *i < bytes.len() {
        if bytes[*i] == b'\\' {
            *i = (*i + 2).min(bytes.len());
            continue;
        }
        if bytes[*i] == quote {
            let value = String::from(&input[start..*i]);
            *i += 1;
            return value;
        }
        *i += 1;
    }
    String::from(&input[start..bytes.len()])
}

fn skip_balanced_parens(input: &str, i: &mut usize) {
    let bytes = input.as_bytes();
    if *i >= bytes.len() || bytes[*i] != b'(' {
        return;
    }
    let mut depth = 0u32;
    let mut string_quote = 0u8;
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
            b')' => {
                depth = depth.saturating_sub(1);
                *i += 1;
                if depth == 0 {
                    break;
                }
            }
            _ => *i += 1,
        }
    }
}

fn starts_with_at(bytes: &[u8], index: usize, prefix: &[u8]) -> bool {
    index + prefix.len() <= bytes.len() && &bytes[index..index + prefix.len()] == prefix
}

fn is_ident_start(ch: u8) -> bool {
    ch.is_ascii_alphabetic() || ch == b'_' || ch == b'-'
}
