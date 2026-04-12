#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CssTokenKind {
    AtKeyword,
    OpenBrace,
    CloseBrace,
    OpenParen,
    CloseParen,
    Semicolon,
    Whitespace,
    Comment,
    String,
    Other,
}

#[derive(Clone, Debug)]
struct CssToken {
    kind: CssTokenKind,
    start: usize,
    end: usize,
}

fn lex_css(input: &str) -> Vec<CssToken> {
    let bytes = input.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0usize;

    while i < bytes.len() {
        let start = i;
        let kind = match bytes[i] {
            b'{' => {
                i += 1;
                CssTokenKind::OpenBrace
            }
            b'}' => {
                i += 1;
                CssTokenKind::CloseBrace
            }
            b'(' => {
                i += 1;
                CssTokenKind::OpenParen
            }
            b')' => {
                i += 1;
                CssTokenKind::CloseParen
            }
            b';' => {
                i += 1;
                CssTokenKind::Semicolon
            }
            b'"' | b'\'' => {
                let quote = bytes[i];
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' {
                        i = (i + 2).min(bytes.len());
                        continue;
                    }
                    let ch = bytes[i];
                    i += 1;
                    if ch == quote {
                        break;
                    }
                }
                CssTokenKind::String
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                i += 2;
                while i + 1 < bytes.len() {
                    if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
                CssTokenKind::Comment
            }
            b'@' => {
                i += 1;
                while i < bytes.len() {
                    let ch = bytes[i];
                    if ch.is_ascii_alphanumeric() || ch == b'-' || ch == b'_' {
                        i += 1;
                    } else {
                        break;
                    }
                }
                CssTokenKind::AtKeyword
            }
            b' ' | b'\t' | b'\n' | b'\r' => {
                i += 1;
                while i < bytes.len() {
                    let ch = bytes[i];
                    if ch == b' ' || ch == b'\t' || ch == b'\n' || ch == b'\r' {
                        i += 1;
                    } else {
                        break;
                    }
                }
                CssTokenKind::Whitespace
            }
            _ => {
                i += 1;
                CssTokenKind::Other
            }
        };
        tokens.push(CssToken {
            kind,
            start,
            end: i,
        });
    }

    tokens
}
