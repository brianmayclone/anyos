fn parse_declaration_list_ast(input: &str) -> Vec<CssDeclarationAst> {
    let mut decls = Vec::new();
    let bytes = input.as_bytes();
    let mut i = 0usize;

    while i < bytes.len() {
        while i < bytes.len()
            && (bytes[i] == b' ' || bytes[i] == b'\t' || bytes[i] == b'\n' || bytes[i] == b'\r')
        {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() {
                if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    i += 2;
                    break;
                }
                i += 1;
            }
            continue;
        }

        let name_start = i;
        let mut colon = None;
        let mut paren_depth = 0i32;
        let mut string_quote = 0u8;
        while i < bytes.len() {
            let ch = bytes[i];
            if string_quote != 0 {
                if ch == b'\\' {
                    i = (i + 2).min(bytes.len());
                    continue;
                }
                i += 1;
                if ch == string_quote {
                    string_quote = 0;
                }
                continue;
            }
            match ch {
                b'"' | b'\'' => {
                    string_quote = ch;
                    i += 1;
                }
                b'(' => {
                    paren_depth += 1;
                    i += 1;
                }
                b')' => {
                    if paren_depth > 0 {
                        paren_depth -= 1;
                    }
                    i += 1;
                }
                b':' if paren_depth == 0 => {
                    colon = Some(i);
                    i += 1;
                    break;
                }
                b';' | b'{' | b'}' if paren_depth == 0 => {
                    break;
                }
                _ => i += 1,
            }
        }

        let Some(colon_pos) = colon else {
            while i < bytes.len() && bytes[i] != b';' {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b';' {
                i += 1;
            }
            continue;
        };

        let name = input[name_start..colon_pos].trim();
        let value_start = i;
        paren_depth = 0;
        string_quote = 0;
        while i < bytes.len() {
            let ch = bytes[i];
            if string_quote != 0 {
                if ch == b'\\' {
                    i = (i + 2).min(bytes.len());
                    continue;
                }
                i += 1;
                if ch == string_quote {
                    string_quote = 0;
                }
                continue;
            }
            match ch {
                b'"' | b'\'' => {
                    string_quote = ch;
                    i += 1;
                }
                b'(' => {
                    paren_depth += 1;
                    i += 1;
                }
                b')' => {
                    if paren_depth > 0 {
                        paren_depth -= 1;
                    }
                    i += 1;
                }
                b';' if paren_depth == 0 => break,
                b'{' | b'}' if paren_depth == 0 => break,
                _ => i += 1,
            }
        }

        let raw_value = input[value_start..i].trim();
        let (value, important) = strip_important(raw_value);
        if !name.is_empty() && !value.is_empty() {
            decls.push(CssDeclarationAst {
                name: String::from(name),
                value: parse_value_ast(value),
                important,
            });
        }

        if i < bytes.len() && bytes[i] == b';' {
            i += 1;
        }
    }

    decls
}
