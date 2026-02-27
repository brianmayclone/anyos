#![no_std]
#![no_main]

use anyos_std::String;
use anyos_std::Vec;
use libanyui_client as anyui;
use anyui::IconType;

anyos_std::entry!(main);

// ── Colors ──────────────────────────────────────────────────────────────────

const BG_ADDED: u32 = 0xFF1E3A1E;
const BG_DELETED: u32 = 0xFF3A1E1E;
const BG_CHANGED: u32 = 0xFF3A3A1E;

const CONN_ADDED: u32 = 0xFF2D6B2D;
const CONN_DELETED: u32 = 0xFF6B2D2D;
const CONN_CHANGED: u32 = 0xFF6B6B2D;

// Brighter versions for the currently active hunk
const BG_ADDED_CUR: u32 = 0xFF2A5A2A;
const BG_DELETED_CUR: u32 = 0xFF5A2A2A;
const BG_CHANGED_CUR: u32 = 0xFF5A5A2A;
const CONN_ADDED_CUR: u32 = 0xFF3D9B3D;
const CONN_DELETED_CUR: u32 = 0xFF9B3D3D;
const CONN_CHANGED_CUR: u32 = 0xFF9B9B3D;

const TEXT_ADDED: u32 = 0xFF90EE90;
const TEXT_DELETED: u32 = 0xFFFF9090;
const TEXT_CHANGED: u32 = 0xFFFFFF90;
const TEXT_CHANGED_HL: u32 = 0xFFFFFFFF; // bright white for differing chars within Changed lines
const LINE_NUM_COLOR: u32 = 0xFF606060;

const NUM_COLS: usize = 5;

// ── Color themes ────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct ColorTheme {
    bg_added: u32,
    bg_deleted: u32,
    bg_changed: u32,
    conn_added: u32,
    conn_deleted: u32,
    conn_changed: u32,
    bg_added_cur: u32,
    bg_deleted_cur: u32,
    bg_changed_cur: u32,
    conn_added_cur: u32,
    conn_deleted_cur: u32,
    conn_changed_cur: u32,
    text_added: u32,
    text_deleted: u32,
    text_changed: u32,
    text_changed_hl: u32,
    line_num_color: u32,
}

const THEME_DARK: ColorTheme = ColorTheme {
    bg_added: BG_ADDED, bg_deleted: BG_DELETED, bg_changed: BG_CHANGED,
    conn_added: CONN_ADDED, conn_deleted: CONN_DELETED, conn_changed: CONN_CHANGED,
    bg_added_cur: BG_ADDED_CUR, bg_deleted_cur: BG_DELETED_CUR, bg_changed_cur: BG_CHANGED_CUR,
    conn_added_cur: CONN_ADDED_CUR, conn_deleted_cur: CONN_DELETED_CUR, conn_changed_cur: CONN_CHANGED_CUR,
    text_added: TEXT_ADDED, text_deleted: TEXT_DELETED, text_changed: TEXT_CHANGED,
    text_changed_hl: TEXT_CHANGED_HL, line_num_color: LINE_NUM_COLOR,
};

const THEME_LIGHT: ColorTheme = ColorTheme {
    bg_added: 0xFFD4EDDA, bg_deleted: 0xFFF8D7DA, bg_changed: 0xFFFFF3CD,
    conn_added: 0xFF28A745, conn_deleted: 0xFFDC3545, conn_changed: 0xFFFFC107,
    bg_added_cur: 0xFFC3E6CB, bg_deleted_cur: 0xFFF5C6CB, bg_changed_cur: 0xFFFFEEBA,
    conn_added_cur: 0xFF1E7E34, conn_deleted_cur: 0xFFC82333, conn_changed_cur: 0xFFE0A800,
    text_added: 0xFF155724, text_deleted: 0xFF721C24, text_changed: 0xFF856404,
    text_changed_hl: 0xFF000000, line_num_color: 0xFF999999,
};

const THEME_HIGH_CONTRAST: ColorTheme = ColorTheme {
    bg_added: 0xFF003300, bg_deleted: 0xFF330000, bg_changed: 0xFF333300,
    conn_added: 0xFF00FF00, conn_deleted: 0xFFFF0000, conn_changed: 0xFFFFFF00,
    bg_added_cur: 0xFF004400, bg_deleted_cur: 0xFF440000, bg_changed_cur: 0xFF444400,
    conn_added_cur: 0xFF00FF00, conn_deleted_cur: 0xFFFF0000, conn_changed_cur: 0xFFFFFF00,
    text_added: 0xFF00FF00, text_deleted: 0xFFFF4444, text_changed: 0xFFFFFF00,
    text_changed_hl: 0xFFFFFFFF, line_num_color: 0xFF888888,
};

// ── Syntax highlighting ─────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum SyntaxLang {
    None,
    C,         // C/C++
    Rust,
    Python,
    Shell,
    JavaScript,
}

const SYN_KEYWORD: u32 = 0xFF569CD6;   // blue
const SYN_STRING: u32 = 0xFFCE9178;    // orange
const SYN_COMMENT: u32 = 0xFF6A9955;   // green
const SYN_NUMBER: u32 = 0xFFB5CEA8;    // light green
const SYN_TYPE: u32 = 0xFF4EC9B0;      // teal
const SYN_PREPROC: u32 = 0xFFC586C0;   // purple

fn detect_language(path: &str) -> SyntaxLang {
    let ext = path.rsplit('.').next().unwrap_or("");
    match ext {
        "c" | "h" | "cpp" | "cxx" | "cc" | "hpp" | "hxx" => SyntaxLang::C,
        "rs" => SyntaxLang::Rust,
        "py" => SyntaxLang::Python,
        "sh" | "bash" | "zsh" => SyntaxLang::Shell,
        "js" | "ts" | "jsx" | "tsx" => SyntaxLang::JavaScript,
        _ => SyntaxLang::None,
    }
}

fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn colorize_line(line: &[u8], lang: SyntaxLang) -> Vec<u32> {
    if lang == SyntaxLang::None || line.is_empty() {
        return Vec::new();
    }
    let n = line.len();
    let mut colors: Vec<u32> = Vec::new();
    colors.resize(n, 0);
    let mut i = 0;

    while i < n {
        // Line comments
        if i + 1 < n && line[i] == b'/' && line[i + 1] == b'/' {
            for j in i..n { colors[j] = SYN_COMMENT; }
            break;
        }
        // Python/Shell comments
        if line[i] == b'#' && (lang == SyntaxLang::Python || lang == SyntaxLang::Shell) {
            for j in i..n { colors[j] = SYN_COMMENT; }
            break;
        }
        // Preprocessor
        if i == 0 && line[i] == b'#' && (lang == SyntaxLang::C) {
            for j in 0..n { colors[j] = SYN_PREPROC; }
            break;
        }
        // Strings
        if line[i] == b'"' || line[i] == b'\'' {
            let quote = line[i];
            let start = i;
            i += 1;
            while i < n && line[i] != quote {
                if line[i] == b'\\' && i + 1 < n { i += 1; }
                i += 1;
            }
            if i < n { i += 1; }
            for j in start..i { colors[j] = SYN_STRING; }
            continue;
        }
        // Numbers
        if line[i].is_ascii_digit() && (i == 0 || !is_ident_char(line[i - 1])) {
            let start = i;
            while i < n && (line[i].is_ascii_alphanumeric() || line[i] == b'.' || line[i] == b'x' || line[i] == b'X') {
                i += 1;
            }
            for j in start..i { colors[j] = SYN_NUMBER; }
            continue;
        }
        // Identifiers / keywords
        if line[i].is_ascii_alphabetic() || line[i] == b'_' {
            let start = i;
            while i < n && is_ident_char(line[i]) { i += 1; }
            let word = &line[start..i];
            if is_keyword(word, lang) {
                for j in start..i { colors[j] = SYN_KEYWORD; }
            } else if is_type_word(word, lang) {
                for j in start..i { colors[j] = SYN_TYPE; }
            }
            continue;
        }
        i += 1;
    }
    colors
}

fn is_keyword(word: &[u8], lang: SyntaxLang) -> bool {
    match lang {
        SyntaxLang::C => matches!(word,
            b"if" | b"else" | b"for" | b"while" | b"do" | b"switch" | b"case" | b"break" |
            b"continue" | b"return" | b"goto" | b"typedef" | b"struct" | b"union" | b"enum" |
            b"const" | b"static" | b"extern" | b"inline" | b"sizeof" | b"volatile" |
            b"class" | b"public" | b"private" | b"protected" | b"virtual" | b"override" |
            b"template" | b"typename" | b"namespace" | b"using" | b"new" | b"delete" |
            b"try" | b"catch" | b"throw" | b"nullptr" | b"auto" | b"constexpr"
        ),
        SyntaxLang::Rust => matches!(word,
            b"fn" | b"let" | b"mut" | b"if" | b"else" | b"for" | b"while" | b"loop" |
            b"match" | b"return" | b"break" | b"continue" | b"struct" | b"enum" | b"impl" |
            b"trait" | b"pub" | b"use" | b"mod" | b"crate" | b"self" | b"super" | b"as" |
            b"in" | b"ref" | b"move" | b"where" | b"type" | b"const" | b"static" |
            b"unsafe" | b"extern" | b"async" | b"await" | b"dyn" | b"true" | b"false"
        ),
        SyntaxLang::Python => matches!(word,
            b"def" | b"class" | b"if" | b"elif" | b"else" | b"for" | b"while" | b"return" |
            b"import" | b"from" | b"as" | b"with" | b"try" | b"except" | b"finally" |
            b"raise" | b"pass" | b"break" | b"continue" | b"lambda" | b"yield" | b"in" |
            b"not" | b"and" | b"or" | b"is" | b"True" | b"False" | b"None" | b"self" |
            b"global" | b"nonlocal" | b"assert" | b"del" | b"async" | b"await"
        ),
        SyntaxLang::Shell => matches!(word,
            b"if" | b"then" | b"else" | b"elif" | b"fi" | b"for" | b"while" | b"do" |
            b"done" | b"case" | b"esac" | b"function" | b"return" | b"local" | b"export" |
            b"source" | b"in" | b"select" | b"until" | b"shift" | b"eval" | b"exec" |
            b"exit" | b"set" | b"unset" | b"readonly" | b"declare" | b"typeset"
        ),
        SyntaxLang::JavaScript => matches!(word,
            b"function" | b"var" | b"let" | b"const" | b"if" | b"else" | b"for" | b"while" |
            b"do" | b"switch" | b"case" | b"break" | b"continue" | b"return" | b"new" |
            b"delete" | b"typeof" | b"instanceof" | b"in" | b"of" | b"class" | b"extends" |
            b"import" | b"export" | b"default" | b"try" | b"catch" | b"finally" | b"throw" |
            b"async" | b"await" | b"yield" | b"this" | b"super" | b"true" | b"false" | b"null"
        ),
        SyntaxLang::None => false,
    }
}

fn is_type_word(word: &[u8], lang: SyntaxLang) -> bool {
    match lang {
        SyntaxLang::C => matches!(word,
            b"int" | b"char" | b"float" | b"double" | b"void" | b"long" | b"short" |
            b"unsigned" | b"signed" | b"bool" | b"size_t" | b"uint8_t" | b"uint16_t" |
            b"uint32_t" | b"uint64_t" | b"int8_t" | b"int16_t" | b"int32_t" | b"int64_t" |
            b"string" | b"vector" | b"map" | b"set"
        ),
        SyntaxLang::Rust => matches!(word,
            b"u8" | b"u16" | b"u32" | b"u64" | b"u128" | b"usize" | b"i8" | b"i16" |
            b"i32" | b"i64" | b"i128" | b"isize" | b"f32" | b"f64" | b"bool" | b"char" |
            b"str" | b"String" | b"Vec" | b"Option" | b"Result" | b"Box" | b"Rc" | b"Arc" |
            b"Self" | b"Some" | b"None" | b"Ok" | b"Err"
        ),
        SyntaxLang::Python => matches!(word,
            b"int" | b"float" | b"str" | b"bool" | b"list" | b"dict" | b"set" | b"tuple" |
            b"bytes" | b"type" | b"object" | b"range" | b"print" | b"len" | b"super"
        ),
        SyntaxLang::JavaScript => matches!(word,
            b"Array" | b"Object" | b"String" | b"Number" | b"Boolean" | b"Map" | b"Set" |
            b"Promise" | b"undefined" | b"NaN" | b"Infinity" | b"console" | b"document" | b"window"
        ),
        _ => false,
    }
}

// ── Comment stripping ───────────────────────────────────────────────────────

fn strip_comments(line: &str, lang: SyntaxLang, in_block_comment: &mut bool) -> String {
    if lang == SyntaxLang::None {
        return String::from(line);
    }
    let bytes = line.as_bytes();
    let n = bytes.len();
    let mut result = Vec::new();
    let mut i = 0;

    while i < n {
        if *in_block_comment {
            // Look for */
            if i + 1 < n && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                *in_block_comment = false;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        // Block comment start /* (C, Rust, JS)
        if i + 1 < n && bytes[i] == b'/' && bytes[i + 1] == b'*'
            && (lang == SyntaxLang::C || lang == SyntaxLang::Rust || lang == SyntaxLang::JavaScript)
        {
            *in_block_comment = true;
            i += 2;
            continue;
        }
        // Line comment //
        if i + 1 < n && bytes[i] == b'/' && bytes[i + 1] == b'/'
            && (lang == SyntaxLang::C || lang == SyntaxLang::Rust || lang == SyntaxLang::JavaScript)
        {
            break;
        }
        // Python/Shell line comment #
        if bytes[i] == b'#' && (lang == SyntaxLang::Python || lang == SyntaxLang::Shell) {
            break;
        }
        // Skip strings
        if bytes[i] == b'"' || bytes[i] == b'\'' {
            let quote = bytes[i];
            result.push(bytes[i]);
            i += 1;
            while i < n && bytes[i] != quote {
                if bytes[i] == b'\\' && i + 1 < n {
                    result.push(bytes[i]);
                    i += 1;
                }
                result.push(bytes[i]);
                i += 1;
            }
            if i < n {
                result.push(bytes[i]);
                i += 1;
            }
            continue;
        }
        result.push(bytes[i]);
        i += 1;
    }

    String::from(core::str::from_utf8(&result).unwrap_or(""))
}

// ── Data structures ─────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum DiffKind {
    Equal,
    Added,
    Deleted,
    Changed,
}

struct DiffLine {
    kind: DiffKind,
    left_idx: Option<usize>,
    right_idx: Option<usize>,
}

struct DiffHunk {
    start: usize,
    end: usize,
}

struct UndoState {
    left_lines: Vec<String>,
    right_lines: Vec<String>,
    left_modified: bool,
    right_modified: bool,
}

// ── Diff algorithm (Myers O(ND)) ────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum EditOp {
    Equal(usize, usize),
    Insert(usize),
    Delete(usize),
}

fn compute_edit_script(left: &[String], right: &[String]) -> Vec<EditOp> {
    let n = left.len();
    let m = right.len();

    if n == 0 && m == 0 {
        return Vec::new();
    }
    if n == 0 {
        let mut ops = Vec::new();
        for j in 0..m {
            ops.push(EditOp::Insert(j));
        }
        return ops;
    }
    if m == 0 {
        let mut ops = Vec::new();
        for i in 0..n {
            ops.push(EditOp::Delete(i));
        }
        return ops;
    }

    let max_d = n + m;
    let v_size = 2 * max_d + 1;
    let offset = max_d as isize;

    let mut v: Vec<usize> = Vec::new();
    v.resize(v_size, 0);

    let mut trace: Vec<Vec<usize>> = Vec::new();

    let mut found = false;
    for d in 0..=max_d {
        let v_snap = v.clone();

        let k_min = -(d as isize);
        let k_max = d as isize;
        let mut k = k_min;
        while k <= k_max {
            let ki = (k + offset) as usize;

            let mut x = if k == k_min
                || (k != k_max && v[ki - 1] < v[ki + 1])
            {
                v[ki + 1]
            } else {
                v[ki - 1] + 1
            };

            let mut y = (x as isize - k) as usize;

            while x < n && y < m && left[x] == right[y] {
                x += 1;
                y += 1;
            }

            v[ki] = x;

            if x >= n && y >= m {
                found = true;
                break;
            }

            k += 2;
        }
        trace.push(v_snap);
        if found {
            break;
        }
    }

    backtrack(&trace, n, m, offset)
}

fn backtrack(trace: &[Vec<usize>], n: usize, m: usize, offset: isize) -> Vec<EditOp> {
    let mut ops: Vec<EditOp> = Vec::new();
    let mut x = n;
    let mut y = m;

    let d_count = trace.len();
    let mut d = d_count;
    while d > 0 {
        d -= 1;
        let v = &trace[d];
        let k = x as isize - y as isize;

        let prev_k = if k == -(d as isize)
            || (k != d as isize && v[(k - 1 + offset) as usize] < v[(k + 1 + offset) as usize])
        {
            k + 1
        } else {
            k - 1
        };

        let prev_x = v[(prev_k + offset) as usize];
        let prev_y = (prev_x as isize - prev_k) as usize;

        let mut cx = x;
        let mut cy = y;
        while cx > prev_x && cy > prev_y {
            cx -= 1;
            cy -= 1;
            ops.push(EditOp::Equal(cx, cy));
        }

        if d > 0 {
            if prev_k == k - 1 {
                ops.push(EditOp::Delete(prev_x));
            } else {
                ops.push(EditOp::Insert(prev_y));
            }
        }

        x = prev_x;
        y = prev_y;
    }

    ops.reverse();
    ops
}

/// Compute similarity between two lines as a percentage (0..100).
/// Uses longest common subsequence length.
fn line_similarity(a: &str, b: &str) -> usize {
    if a == b {
        return 100;
    }
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    let n = ab.len();
    let m = bb.len();
    if n == 0 || m == 0 {
        return 0;
    }
    let mut prev: Vec<usize> = Vec::new();
    prev.resize(m + 1, 0);
    let mut curr: Vec<usize> = Vec::new();
    curr.resize(m + 1, 0);

    for i in 1..=n {
        for j in 1..=m {
            curr[j] = if ab[i - 1] == bb[j - 1] {
                prev[j - 1] + 1
            } else if prev[j] >= curr[j - 1] {
                prev[j]
            } else {
                curr[j - 1]
            };
        }
        for j in 0..=m {
            prev[j] = curr[j];
            curr[j] = 0;
        }
    }

    (200 * prev[m]) / (n + m)
}

/// Match a block of consecutive Delete/Insert operations using similarity-based alignment.
/// Similar lines (above threshold) become Changed, others remain Delete/Add.
fn match_block(
    deletes: &[usize],
    inserts: &[usize],
    left: &[String],
    right: &[String],
    result: &mut Vec<DiffLine>,
) {
    let nd = deletes.len();
    let ni = inserts.len();
    if nd == 0 {
        for idx in 0..ni {
            result.push(DiffLine {
                kind: DiffKind::Added,
                left_idx: None,
                right_idx: Some(inserts[idx]),
            });
        }
        return;
    }
    if ni == 0 {
        for idx in 0..nd {
            result.push(DiffLine {
                kind: DiffKind::Deleted,
                left_idx: Some(deletes[idx]),
                right_idx: None,
            });
        }
        return;
    }

    const THRESHOLD: usize = 40;
    let w = ni + 1;

    // Precompute similarity matrix
    let mut sims: Vec<usize> = Vec::new();
    sims.resize(nd * ni, 0);
    for d in 0..nd {
        let ls = if deletes[d] < left.len() { left[deletes[d]].as_str() } else { "" };
        for ins in 0..ni {
            let rs = if inserts[ins] < right.len() { right[inserts[ins]].as_str() } else { "" };
            sims[d * ni + ins] = line_similarity(ls, rs);
        }
    }

    // DP alignment
    let sz = (nd + 1) * w;
    let mut dp: Vec<isize> = Vec::new();
    dp.resize(sz, 0);
    let mut choice: Vec<u8> = Vec::new();
    choice.resize(sz, 0);

    for d in 1..=nd {
        for i in 1..=ni {
            let sim = sims[(d - 1) * ni + (i - 1)];
            let match_score = if sim >= THRESHOLD {
                dp[(d - 1) * w + (i - 1)] + sim as isize
            } else {
                -1
            };
            let skip_d = dp[(d - 1) * w + i];
            let skip_i = dp[d * w + (i - 1)];

            if match_score >= skip_d && match_score >= skip_i {
                dp[d * w + i] = match_score;
                choice[d * w + i] = 1;
            } else if skip_d >= skip_i {
                dp[d * w + i] = skip_d;
                choice[d * w + i] = 2;
            } else {
                dp[d * w + i] = skip_i;
                choice[d * w + i] = 3;
            }
        }
    }

    // Backtrack
    let start_idx = result.len();
    let mut d = nd;
    let mut i = ni;
    while d > 0 || i > 0 {
        if d == 0 {
            result.push(DiffLine {
                kind: DiffKind::Added,
                left_idx: None,
                right_idx: Some(inserts[i - 1]),
            });
            i -= 1;
        } else if i == 0 {
            result.push(DiffLine {
                kind: DiffKind::Deleted,
                left_idx: Some(deletes[d - 1]),
                right_idx: None,
            });
            d -= 1;
        } else {
            match choice[d * w + i] {
                1 => {
                    result.push(DiffLine {
                        kind: DiffKind::Changed,
                        left_idx: Some(deletes[d - 1]),
                        right_idx: Some(inserts[i - 1]),
                    });
                    d -= 1;
                    i -= 1;
                }
                2 => {
                    result.push(DiffLine {
                        kind: DiffKind::Deleted,
                        left_idx: Some(deletes[d - 1]),
                        right_idx: None,
                    });
                    d -= 1;
                }
                _ => {
                    result.push(DiffLine {
                        kind: DiffKind::Added,
                        left_idx: None,
                        right_idx: Some(inserts[i - 1]),
                    });
                    i -= 1;
                }
            }
        }
    }
    result[start_idx..].reverse();
}

fn build_diff_lines(ops: &[EditOp], left: &[String], right: &[String]) -> Vec<DiffLine> {
    let mut result: Vec<DiffLine> = Vec::new();
    let mut i = 0;
    while i < ops.len() {
        match ops[i] {
            EditOp::Equal(li, ri) => {
                result.push(DiffLine {
                    kind: DiffKind::Equal,
                    left_idx: Some(li),
                    right_idx: Some(ri),
                });
                i += 1;
            }
            _ => {
                // Collect consecutive Delete/Insert block
                let mut deletes: Vec<usize> = Vec::new();
                let mut inserts: Vec<usize> = Vec::new();
                while i < ops.len() {
                    match ops[i] {
                        EditOp::Delete(li) => {
                            deletes.push(li);
                            i += 1;
                        }
                        EditOp::Insert(ri) => {
                            inserts.push(ri);
                            i += 1;
                        }
                        _ => break,
                    }
                }
                match_block(&deletes, &inserts, left, right, &mut result);
            }
        }
    }
    result
}

fn extract_hunks(diff_lines: &[DiffLine]) -> Vec<DiffHunk> {
    let mut hunks = Vec::new();
    let mut i = 0;
    while i < diff_lines.len() {
        if diff_lines[i].kind != DiffKind::Equal {
            let start = i;
            while i < diff_lines.len() && diff_lines[i].kind != DiffKind::Equal {
                i += 1;
            }
            hunks.push(DiffHunk { start, end: i });
        } else {
            i += 1;
        }
    }
    hunks
}

fn count_stats(diff_lines: &[DiffLine]) -> (usize, usize, usize) {
    let mut added = 0;
    let mut deleted = 0;
    let mut changed = 0;
    for dl in diff_lines {
        match dl.kind {
            DiffKind::Added => added += 1,
            DiffKind::Deleted => deleted += 1,
            DiffKind::Changed => changed += 1,
            DiffKind::Equal => {}
        }
    }
    (added, deleted, changed)
}

// ── File I/O ────────────────────────────────────────────────────────────────

fn read_file(path: &str) -> Option<Vec<u8>> {
    let fd = anyos_std::fs::open(path, 0);
    if fd == u32::MAX {
        return None;
    }
    let mut content = Vec::new();
    let mut buf = [0u8; 512];
    loop {
        let n = anyos_std::fs::read(fd, &mut buf);
        if n == 0 || n == u32::MAX {
            break;
        }
        content.extend_from_slice(&buf[..n as usize]);
    }
    anyos_std::fs::close(fd);
    Some(content)
}

fn load_lines(path: &str) -> Vec<String> {
    match read_file(path) {
        Some(data) => {
            let text = core::str::from_utf8(&data).unwrap_or("");
            if text.is_empty() {
                Vec::new()
            } else {
                text.split('\n').map(String::from).collect()
            }
        }
        None => Vec::new(),
    }
}

fn save_file(path: &str, lines: &[String]) -> bool {
    let content = lines.join("\n");
    anyos_std::fs::write_bytes(path, content.as_bytes()).is_ok()
}

// ── Argument parsing ────────────────────────────────────────────────────────

struct CliArgs {
    left_path: String,
    right_path: String,
    left_label: String,
    right_label: String,
    output_path: String,
    auto_compare: bool,
}

fn parse_args(raw: &str) -> CliArgs {
    let raw = raw.trim();
    let mut args = CliArgs {
        left_path: String::new(),
        right_path: String::new(),
        left_label: String::new(),
        right_label: String::new(),
        output_path: String::new(),
        auto_compare: false,
    };
    if raw.is_empty() {
        return args;
    }

    // Split into tokens
    let mut tokens: Vec<&str> = Vec::new();
    let mut rest = raw;
    while !rest.is_empty() {
        rest = rest.trim_start();
        if rest.is_empty() { break; }
        if let Some(pos) = rest.find(' ') {
            tokens.push(&rest[..pos]);
            rest = &rest[pos + 1..];
        } else {
            tokens.push(rest);
            break;
        }
    }

    let mut i = 0;
    let mut positional = 0;
    while i < tokens.len() {
        let tok = tokens[i];
        if tok == "--label-left" || tok == "--ll" {
            if i + 1 < tokens.len() {
                args.left_label = String::from(tokens[i + 1]);
                i += 2;
                continue;
            }
        } else if tok == "--label-right" || tok == "--lr" {
            if i + 1 < tokens.len() {
                args.right_label = String::from(tokens[i + 1]);
                i += 2;
                continue;
            }
        } else if tok == "--output" || tok == "-o" {
            if i + 1 < tokens.len() {
                args.output_path = String::from(tokens[i + 1]);
                i += 2;
                continue;
            }
        } else if tok == "--auto-compare" || tok == "-a" {
            args.auto_compare = true;
            i += 1;
            continue;
        } else if !tok.starts_with('-') {
            if positional == 0 {
                args.left_path = String::from(tok);
            } else if positional == 1 {
                args.right_path = String::from(tok);
            }
            positional += 1;
        }
        i += 1;
    }
    args
}

// ── App state ───────────────────────────────────────────────────────────────

struct AppState {
    win: anyui::Window,
    grid: anyui::DataGrid,
    stats_label: anyui::Label,
    status_label: anyui::Label,
    hunk_label: anyui::Label,

    left_lines: Vec<String>,
    right_lines: Vec<String>,
    left_path: String,
    right_path: String,
    diff_lines: Vec<DiffLine>,
    hunks: Vec<DiffHunk>,
    current_hunk: usize,
    num_added: usize,
    num_deleted: usize,
    num_changed: usize,

    // Merge/edit/save state
    left_modified: bool,
    right_modified: bool,

    // Undo/Redo
    undo_stack: Vec<UndoState>,
    redo_stack: Vec<UndoState>,

    // Filters
    ignore_whitespace: bool,
    ignore_blank_lines: bool,
    ignore_comments: bool,
    text_filter: String,

    // Edit panel widgets
    edit_panel: anyui::View,
    edit_left_field: anyui::TextField,
    edit_right_field: anyui::TextField,
    editing_row: Option<usize>,

    // Search state
    search_panel: anyui::View,
    search_field: anyui::TextField,
    search_active: bool,
    search_matches: Vec<usize>,
    search_current: usize,

    // Go to line panel
    goto_panel: anyui::View,
    goto_field: anyui::TextField,
    goto_active: bool,

    // Fullscreen
    fullscreen: bool,
    pre_fullscreen_size: (u32, u32),
    pre_fullscreen_pos: (i32, i32),

    // Color theme
    theme: ColorTheme,

    // Syntax highlighting
    left_lang: SyntaxLang,
    right_lang: SyntaxLang,

    // CLI options
    left_label: String,
    right_label: String,
    output_path: String,
    auto_compare: bool,
}

static mut APP: Option<AppState> = None;

fn app() -> &'static mut AppState {
    unsafe { APP.as_mut().unwrap() }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn write_num(buf: &mut Vec<u8>, n: usize) {
    if n == 0 {
        buf.push(b'0');
        return;
    }
    let start = buf.len();
    let mut v = n;
    while v > 0 {
        buf.push(b'0' + (v % 10) as u8);
        v /= 10;
    }
    buf[start..].reverse();
}

fn make_title(left_path: &str, right_path: &str) -> String {
    let left = if left_path.is_empty() { "(none)" } else {
        left_path.rsplit('/').next().unwrap_or(left_path)
    };
    let right = if right_path.is_empty() { "(none)" } else {
        right_path.rsplit('/').next().unwrap_or(right_path)
    };
    anyos_std::format!("{} vs {} - Diff", left, right)
}

fn get_text_field_text(field: &anyui::TextField) -> String {
    let mut buf = [0u8; 1024];
    let len = field.get_text(&mut buf);
    let s = core::str::from_utf8(&buf[..len as usize]).unwrap_or("");
    String::from(s)
}

// ── Character-level diff for highlighting within Changed lines ───────────────

/// Compute which bytes in `a` and `b` are NOT part of the LCS (i.e. differ).
/// Returns two Vec<u8> of the same lengths as a and b. 1 = different, 0 = same.
fn char_diff_flags(a: &[u8], b: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let n = a.len();
    let m = b.len();
    let mut a_diff: Vec<u8> = Vec::new();
    a_diff.resize(n, 1);
    let mut b_diff: Vec<u8> = Vec::new();
    b_diff.resize(m, 1);

    if n == 0 || m == 0 {
        return (a_diff, b_diff);
    }

    // Build LCS DP table (u16 to save memory)
    let w = m + 1;
    let mut dp: Vec<u16> = Vec::new();
    dp.resize((n + 1) * w, 0);

    for i in 1..=n {
        for j in 1..=m {
            dp[i * w + j] = if a[i - 1] == b[j - 1] {
                dp[(i - 1) * w + (j - 1)] + 1
            } else if dp[(i - 1) * w + j] >= dp[i * w + (j - 1)] {
                dp[(i - 1) * w + j]
            } else {
                dp[i * w + (j - 1)]
            };
        }
    }

    // Backtrack to mark LCS characters as same (0)
    let mut i = n;
    let mut j = m;
    while i > 0 && j > 0 {
        if a[i - 1] == b[j - 1] {
            a_diff[i - 1] = 0;
            b_diff[j - 1] = 0;
            i -= 1;
            j -= 1;
        } else if dp[(i - 1) * w + j] >= dp[i * w + (j - 1)] {
            i -= 1;
        } else {
            j -= 1;
        }
    }

    expand_to_word_boundaries(a, &mut a_diff);
    expand_to_word_boundaries(b, &mut b_diff);

    (a_diff, b_diff)
}

/// Expand diff flags to cover full words rather than individual characters.
fn expand_to_word_boundaries(text: &[u8], flags: &mut [u8]) {
    let n = text.len();
    let mut i = 0;
    while i < n {
        if flags[i] != 0 {
            // Expand backwards to start of word
            let mut start = i;
            while start > 0 && !is_word_sep(text[start - 1]) {
                start -= 1;
            }
            // Skip forward past diff region
            while i < n && flags[i] != 0 {
                i += 1;
            }
            // Expand forward to end of word
            while i < n && !is_word_sep(text[i]) {
                i += 1;
            }
            // Mark entire range
            for j in start..i {
                flags[j] = 1;
            }
        } else {
            i += 1;
        }
    }
}

fn is_word_sep(b: u8) -> bool {
    b == b' ' || b == b'\t' || b == b',' || b == b';' || b == b'.' || b == b':' ||
    b == b'(' || b == b')' || b == b'[' || b == b']' || b == b'{' || b == b'}' ||
    b == b'<' || b == b'>' || b == b'"' || b == b'\''
}

// ── Grid population ─────────────────────────────────────────────────────────

fn populate_grid(s: &AppState) {
    let n = s.diff_lines.len();
    if n == 0 {
        s.grid.set_row_count(0);
        s.grid.set_cell_colors(&[]);
        s.grid.set_cell_bg_colors(&[]);
        s.grid.set_char_colors(&[], &[]);
        return;
    }

    let t = &s.theme;
    s.grid.set_row_count(n as u32);

    let mut data_buf: Vec<u8> = Vec::new();
    let total_cells = n * NUM_COLS;
    let mut text_colors: Vec<u32> = Vec::new();
    text_colors.resize(total_cells, 0);
    let mut bg_colors: Vec<u32> = Vec::new();
    bg_colors.resize(total_cells, 0);
    let mut char_colors: Vec<u32> = Vec::new();
    let mut char_color_offsets: Vec<u32> = Vec::new();
    char_color_offsets.resize(total_cells, u32::MAX);

    // Current hunk range for highlighting
    let (cur_start, cur_end) = if !s.hunks.is_empty() && s.current_hunk < s.hunks.len() {
        (s.hunks[s.current_hunk].start, s.hunks[s.current_hunk].end)
    } else {
        (usize::MAX, usize::MAX)
    };

    for (row, dl) in s.diff_lines.iter().enumerate() {
        if row > 0 {
            data_buf.push(0x1E);
        }
        let base = row * NUM_COLS;
        let cur = row >= cur_start && row < cur_end;

        // Col 0: Left line number
        if let Some(li) = dl.left_idx {
            write_num(&mut data_buf, li + 1);
        }
        data_buf.push(0x1F);

        // Col 1: Left text
        let left_text_start = data_buf.len();
        match dl.kind {
            DiffKind::Added => {}
            _ => {
                if let Some(li) = dl.left_idx {
                    if li < s.left_lines.len() {
                        data_buf.extend_from_slice(s.left_lines[li].as_bytes());
                    }
                }
            }
        }
        let left_text_end = data_buf.len();
        data_buf.push(0x1F);

        // Col 2: Connector marker
        match dl.kind {
            DiffKind::Added => data_buf.push(b'+'),
            DiffKind::Deleted => data_buf.push(b'-'),
            DiffKind::Changed => data_buf.push(b'~'),
            DiffKind::Equal => {}
        }
        data_buf.push(0x1F);

        // Col 3: Right line number
        if let Some(ri) = dl.right_idx {
            write_num(&mut data_buf, ri + 1);
        }
        data_buf.push(0x1F);

        // Col 4: Right text
        let right_text_start = data_buf.len();
        match dl.kind {
            DiffKind::Deleted => {}
            _ => {
                if let Some(ri) = dl.right_idx {
                    if ri < s.right_lines.len() {
                        data_buf.extend_from_slice(s.right_lines[ri].as_bytes());
                    }
                }
            }
        }
        let right_text_end = data_buf.len();

        // Text colors
        match dl.kind {
            DiffKind::Equal => {
                // Syntax highlighting for equal lines
                if s.left_lang != SyntaxLang::None {
                    let left_bytes = &data_buf[left_text_start..left_text_end];
                    let syn = colorize_line(left_bytes, s.left_lang);
                    if !syn.is_empty() {
                        char_color_offsets[base + 1] = char_colors.len() as u32;
                        char_colors.extend_from_slice(&syn);
                    }
                }
                if s.right_lang != SyntaxLang::None {
                    let right_bytes = &data_buf[right_text_start..right_text_end];
                    let syn = colorize_line(right_bytes, s.right_lang);
                    if !syn.is_empty() {
                        char_color_offsets[base + 4] = char_colors.len() as u32;
                        char_colors.extend_from_slice(&syn);
                    }
                }
            }
            DiffKind::Added => {
                text_colors[base + 2] = t.text_added;
                text_colors[base + 4] = t.text_added;
            }
            DiffKind::Deleted => {
                text_colors[base + 1] = t.text_deleted;
                text_colors[base + 2] = t.text_deleted;
            }
            DiffKind::Changed => {
                text_colors[base + 1] = t.text_changed;
                text_colors[base + 2] = t.text_changed;
                text_colors[base + 4] = t.text_changed;

                // Compute per-character highlight for differing chars
                let left_text = match dl.left_idx {
                    Some(li) if li < s.left_lines.len() => s.left_lines[li].as_bytes(),
                    _ => &[],
                };
                let right_text = match dl.right_idx {
                    Some(ri) if ri < s.right_lines.len() => s.right_lines[ri].as_bytes(),
                    _ => &[],
                };
                let (left_diff, right_diff) = char_diff_flags(left_text, right_text);

                // Col 1 (left text) per-char colors
                char_color_offsets[base + 1] = char_colors.len() as u32;
                for idx in 0..left_diff.len() {
                    char_colors.push(if left_diff[idx] != 0 { t.text_changed_hl } else { 0 });
                }

                // Col 4 (right text) per-char colors
                char_color_offsets[base + 4] = char_colors.len() as u32;
                for idx in 0..right_diff.len() {
                    char_colors.push(if right_diff[idx] != 0 { t.text_changed_hl } else { 0 });
                }
            }
        }

        // Line number colors
        if dl.left_idx.is_some() {
            text_colors[base + 0] = t.line_num_color;
        }
        if dl.right_idx.is_some() {
            text_colors[base + 3] = t.line_num_color;
        }

        // Background colors (brighter for current hunk)
        match dl.kind {
            DiffKind::Equal => {}
            DiffKind::Added => {
                bg_colors[base + 2] = if cur { t.conn_added_cur } else { t.conn_added };
                bg_colors[base + 3] = if cur { t.bg_added_cur } else { t.bg_added };
                bg_colors[base + 4] = if cur { t.bg_added_cur } else { t.bg_added };
            }
            DiffKind::Deleted => {
                bg_colors[base + 0] = if cur { t.bg_deleted_cur } else { t.bg_deleted };
                bg_colors[base + 1] = if cur { t.bg_deleted_cur } else { t.bg_deleted };
                bg_colors[base + 2] = if cur { t.conn_deleted_cur } else { t.conn_deleted };
            }
            DiffKind::Changed => {
                bg_colors[base + 0] = if cur { t.bg_changed_cur } else { t.bg_changed };
                bg_colors[base + 1] = if cur { t.bg_changed_cur } else { t.bg_changed };
                bg_colors[base + 2] = if cur { t.conn_changed_cur } else { t.conn_changed };
                bg_colors[base + 3] = if cur { t.bg_changed_cur } else { t.bg_changed };
                bg_colors[base + 4] = if cur { t.bg_changed_cur } else { t.bg_changed };
            }
        }
    }

    s.grid.set_data_raw(&data_buf);
    s.grid.set_cell_colors(&text_colors);
    s.grid.set_cell_bg_colors(&bg_colors);
    s.grid.set_char_colors(&char_colors, &char_color_offsets);
}

fn update_labels(s: &AppState) {
    let stats = anyos_std::format!("{}A {}D {}C", s.num_added, s.num_deleted, s.num_changed);
    s.stats_label.set_text(&stats);

    let left_name = if s.left_path.is_empty() { "(none)" } else {
        s.left_path.rsplit('/').next().unwrap_or(&s.left_path)
    };
    let right_name = if s.right_path.is_empty() { "(none)" } else {
        s.right_path.rsplit('/').next().unwrap_or(&s.right_path)
    };
    let lmod = if s.left_modified { "*" } else { "" };
    let rmod = if s.right_modified { "*" } else { "" };

    let status = anyos_std::format!("{}A {}D {}C  |  {}{} vs {}{}  |  {} lines",
        s.num_added, s.num_deleted, s.num_changed,
        left_name, lmod, right_name, rmod,
        s.diff_lines.len());
    s.status_label.set_text(&status);

    if !s.hunks.is_empty() {
        let hunk_info = anyos_std::format!("Diff {}/{}", s.current_hunk + 1, s.hunks.len());
        s.hunk_label.set_text(&hunk_info);
    } else {
        s.hunk_label.set_text("No diffs");
    }
}

// ── Undo/Redo ───────────────────────────────────────────────────────────

const MAX_UNDO: usize = 50;

fn push_undo() {
    let s = app();
    s.undo_stack.push(UndoState {
        left_lines: s.left_lines.clone(),
        right_lines: s.right_lines.clone(),
        left_modified: s.left_modified,
        right_modified: s.right_modified,
    });
    if s.undo_stack.len() > MAX_UNDO {
        s.undo_stack.remove(0);
    }
    s.redo_stack.clear();
}

fn undo() {
    let s = app();
    if s.undo_stack.is_empty() { return; }
    s.redo_stack.push(UndoState {
        left_lines: s.left_lines.clone(),
        right_lines: s.right_lines.clone(),
        left_modified: s.left_modified,
        right_modified: s.right_modified,
    });
    let state = s.undo_stack.pop().unwrap();
    s.left_lines = state.left_lines;
    s.right_lines = state.right_lines;
    s.left_modified = state.left_modified;
    s.right_modified = state.right_modified;
    recompute();
}

fn redo() {
    let s = app();
    if s.redo_stack.is_empty() { return; }
    s.undo_stack.push(UndoState {
        left_lines: s.left_lines.clone(),
        right_lines: s.right_lines.clone(),
        left_modified: s.left_modified,
        right_modified: s.right_modified,
    });
    let state = s.redo_stack.pop().unwrap();
    s.left_lines = state.left_lines;
    s.right_lines = state.right_lines;
    s.left_modified = state.left_modified;
    s.right_modified = state.right_modified;
    recompute();
}

// ── Filtering ───────────────────────────────────────────────────────────

fn normalize_for_compare(line: &str, ignore_ws: bool) -> String {
    if !ignore_ws {
        return String::from(line);
    }
    let mut result = String::new();
    let mut in_space = true;
    for c in line.chars() {
        if c == ' ' || c == '\t' {
            if !in_space && !result.is_empty() {
                result.push(' ');
                in_space = true;
            }
        } else {
            result.push(c);
            in_space = false;
        }
    }
    if result.ends_with(' ') {
        result.pop();
    }
    result
}

fn lines_for_compare(
    lines: &[String], ignore_ws: bool, ignore_blank: bool,
    ignore_comments: bool, lang: SyntaxLang, text_filter: &str,
) -> Vec<String> {
    let mut in_block = false;
    lines.iter().map(|l| {
        let mut s = if ignore_comments && lang != SyntaxLang::None {
            strip_comments(l, lang, &mut in_block)
        } else {
            String::from(l.as_str())
        };
        if !text_filter.is_empty() {
            // Simple substring removal filter
            while let Some(pos) = s.find(text_filter) {
                let end = pos + text_filter.len();
                s = anyos_std::format!("{}{}", &s[..pos], &s[end..]);
            }
        }
        if ignore_blank && s.trim().is_empty() {
            String::from("")
        } else {
            normalize_for_compare(&s, ignore_ws)
        }
    }).collect()
}

// ── Actions ─────────────────────────────────────────────────────────────────

fn recompute() {
    let s = app();
    let left_cmp = lines_for_compare(
        &s.left_lines, s.ignore_whitespace, s.ignore_blank_lines,
        s.ignore_comments, s.left_lang, &s.text_filter,
    );
    let right_cmp = lines_for_compare(
        &s.right_lines, s.ignore_whitespace, s.ignore_blank_lines,
        s.ignore_comments, s.right_lang, &s.text_filter,
    );
    let ops = compute_edit_script(&left_cmp, &right_cmp);
    s.diff_lines = build_diff_lines(&ops, &left_cmp, &right_cmp);
    s.hunks = extract_hunks(&s.diff_lines);
    let (a, d, c) = count_stats(&s.diff_lines);
    s.num_added = a;
    s.num_deleted = d;
    s.num_changed = c;
    s.current_hunk = 0;
    populate_grid(s);
    update_labels(s);
}

fn open_left() {
    if let Some(path) = anyui::FileDialog::open_file() {
        let s = app();
        s.left_lines = load_lines(&path);
        s.left_path = path;
        s.left_modified = false;
        recompute();
        update_title();
    }
}

fn open_right() {
    if let Some(path) = anyui::FileDialog::open_file() {
        let s = app();
        s.right_lines = load_lines(&path);
        s.right_path = path;
        s.right_modified = false;
        recompute();
        update_title();
    }
}

fn save_as_left() {
    let s = app();
    let default = if s.left_path.is_empty() { "untitled.txt" } else {
        s.left_path.rsplit('/').next().unwrap_or("untitled.txt")
    };
    if let Some(path) = anyui::FileDialog::save_file(default) {
        if save_file(&path, &s.left_lines) {
            s.left_path = path;
            s.left_modified = false;
            update_labels(s);
            update_title();
        }
    }
}

fn save_as_right() {
    let s = app();
    let default = if s.right_path.is_empty() { "untitled.txt" } else {
        s.right_path.rsplit('/').next().unwrap_or("untitled.txt")
    };
    if let Some(path) = anyui::FileDialog::save_file(default) {
        if save_file(&path, &s.right_lines) {
            s.right_path = path;
            s.right_modified = false;
            update_labels(s);
            update_title();
        }
    }
}

fn refresh() {
    let s = app();
    if !s.left_path.is_empty() {
        s.left_lines = load_lines(&s.left_path);
        s.left_modified = false;
    }
    if !s.right_path.is_empty() {
        s.right_lines = load_lines(&s.right_path);
        s.right_modified = false;
    }
    s.undo_stack.clear();
    s.redo_stack.clear();
    recompute();
}

fn toggle_ignore_whitespace() {
    let s = app();
    s.ignore_whitespace = !s.ignore_whitespace;
    recompute();
}

fn toggle_ignore_blank_lines() {
    let s = app();
    s.ignore_blank_lines = !s.ignore_blank_lines;
    recompute();
}

fn toggle_ignore_comments() {
    let s = app();
    s.ignore_comments = !s.ignore_comments;
    let state = if s.ignore_comments { "ON" } else { "OFF" };
    let msg = anyos_std::format!("Ignore comments: {}", state);
    s.status_label.set_text(&msg);
    recompute();
}

fn set_text_filter(filter: &str) {
    let s = app();
    s.text_filter = String::from(filter);
    if filter.is_empty() {
        s.status_label.set_text("Text filter cleared");
    } else {
        let msg = anyos_std::format!("Filter: \"{}\"", filter);
        s.status_label.set_text(&msg);
    }
    recompute();
}

fn cycle_theme() {
    let s = app();
    // Detect current theme by checking one field
    if s.theme.bg_added == THEME_DARK.bg_added {
        s.theme = THEME_LIGHT;
        s.status_label.set_text("Theme: Light");
    } else if s.theme.bg_added == THEME_LIGHT.bg_added {
        s.theme = THEME_HIGH_CONTRAST;
        s.status_label.set_text("Theme: High Contrast");
    } else {
        s.theme = THEME_DARK;
        s.status_label.set_text("Theme: Dark");
    }
    populate_grid(s);
}

fn navigate_to_hunk(hunk_idx: usize) {
    let s = app();
    if hunk_idx < s.hunks.len() {
        s.current_hunk = hunk_idx;
        let row = s.hunks[hunk_idx].start;
        s.grid.set_selected_row(row as u32);
        populate_grid(s);
        update_labels(s);
    }
}

fn navigate_next() {
    let s = app();
    if !s.hunks.is_empty() && s.current_hunk + 1 < s.hunks.len() {
        let next = s.current_hunk + 1;
        navigate_to_hunk(next);
    }
}

fn navigate_prev() {
    let s = app();
    if !s.hunks.is_empty() && s.current_hunk > 0 {
        let prev = s.current_hunk - 1;
        navigate_to_hunk(prev);
    }
}

// ── Merge ───────────────────────────────────────────────────────────────────

fn merge_hunk_to_right() {
    let s = app();
    if s.hunks.is_empty() { return; }
    push_undo();
    let hunk_start = s.hunks[s.current_hunk].start;
    let hunk_end = s.hunks[s.current_hunk].end;

    // Rebuild right_lines by replaying the diff with this hunk merged from left
    let mut new_right: Vec<String> = Vec::new();

    for (i, dl) in s.diff_lines.iter().enumerate() {
        let in_hunk = i >= hunk_start && i < hunk_end;
        match dl.kind {
            DiffKind::Equal => {
                if let Some(idx) = dl.right_idx {
                    if idx < s.right_lines.len() {
                        new_right.push(s.right_lines[idx].clone());
                    }
                }
            }
            DiffKind::Added => {
                if in_hunk {
                    // Merging left→right: drop line that was only on right
                } else {
                    if let Some(idx) = dl.right_idx {
                        if idx < s.right_lines.len() {
                            new_right.push(s.right_lines[idx].clone());
                        }
                    }
                }
            }
            DiffKind::Deleted => {
                if in_hunk {
                    // Merging left→right: add the left line to right
                    if let Some(idx) = dl.left_idx {
                        if idx < s.left_lines.len() {
                            new_right.push(s.left_lines[idx].clone());
                        }
                    }
                }
                // Outside hunk: deleted line is only on left, nothing on right
            }
            DiffKind::Changed => {
                if in_hunk {
                    // Overwrite right with left
                    if let Some(idx) = dl.left_idx {
                        if idx < s.left_lines.len() {
                            new_right.push(s.left_lines[idx].clone());
                        }
                    }
                } else {
                    // Keep right side
                    if let Some(idx) = dl.right_idx {
                        if idx < s.right_lines.len() {
                            new_right.push(s.right_lines[idx].clone());
                        }
                    }
                }
            }
        }
    }

    s.right_lines = new_right;
    s.right_modified = true;
    recompute();
}

fn merge_hunk_to_left() {
    let s = app();
    if s.hunks.is_empty() { return; }
    push_undo();
    let hunk_start = s.hunks[s.current_hunk].start;
    let hunk_end = s.hunks[s.current_hunk].end;

    // Rebuild left_lines by replaying the diff with this hunk merged from right
    let mut new_left: Vec<String> = Vec::new();

    for (i, dl) in s.diff_lines.iter().enumerate() {
        let in_hunk = i >= hunk_start && i < hunk_end;
        match dl.kind {
            DiffKind::Equal => {
                if let Some(idx) = dl.left_idx {
                    if idx < s.left_lines.len() {
                        new_left.push(s.left_lines[idx].clone());
                    }
                }
            }
            DiffKind::Added => {
                if in_hunk {
                    // Merging right→left: add the right line to left
                    if let Some(idx) = dl.right_idx {
                        if idx < s.right_lines.len() {
                            new_left.push(s.right_lines[idx].clone());
                        }
                    }
                }
                // Outside hunk: added line is only on right, nothing on left
            }
            DiffKind::Deleted => {
                if in_hunk {
                    // Merging right→left: drop line that was only on left
                } else {
                    if let Some(idx) = dl.left_idx {
                        if idx < s.left_lines.len() {
                            new_left.push(s.left_lines[idx].clone());
                        }
                    }
                }
            }
            DiffKind::Changed => {
                if in_hunk {
                    // Overwrite left with right
                    if let Some(idx) = dl.right_idx {
                        if idx < s.right_lines.len() {
                            new_left.push(s.right_lines[idx].clone());
                        }
                    }
                } else {
                    // Keep left side
                    if let Some(idx) = dl.left_idx {
                        if idx < s.left_lines.len() {
                            new_left.push(s.left_lines[idx].clone());
                        }
                    }
                }
            }
        }
    }

    s.left_lines = new_left;
    s.left_modified = true;
    recompute();
}

// ── Delete hunk ─────────────────────────────────────────────────────────────

fn delete_hunk_right() {
    let s = app();
    if s.hunks.is_empty() { return; }
    push_undo();
    let hunk_start = s.hunks[s.current_hunk].start;
    let hunk_end = s.hunks[s.current_hunk].end;

    // Collect right-side line indices to remove
    let mut to_remove: Vec<usize> = Vec::new();
    for i in hunk_start..hunk_end {
        let dl = &s.diff_lines[i];
        if dl.kind == DiffKind::Added || dl.kind == DiffKind::Changed {
            if let Some(ri) = dl.right_idx {
                to_remove.push(ri);
            }
        }
    }

    if to_remove.is_empty() { return; }

    let mut new_right: Vec<String> = Vec::new();
    for (i, line) in s.right_lines.iter().enumerate() {
        if !to_remove.contains(&i) {
            new_right.push(line.clone());
        }
    }

    s.right_lines = new_right;
    s.right_modified = true;
    recompute();
}

fn delete_hunk_left() {
    let s = app();
    if s.hunks.is_empty() { return; }
    push_undo();
    let hunk_start = s.hunks[s.current_hunk].start;
    let hunk_end = s.hunks[s.current_hunk].end;

    // Collect left-side line indices to remove
    let mut to_remove: Vec<usize> = Vec::new();
    for i in hunk_start..hunk_end {
        let dl = &s.diff_lines[i];
        if dl.kind == DiffKind::Deleted || dl.kind == DiffKind::Changed {
            if let Some(li) = dl.left_idx {
                to_remove.push(li);
            }
        }
    }

    if to_remove.is_empty() { return; }

    let mut new_left: Vec<String> = Vec::new();
    for (i, line) in s.left_lines.iter().enumerate() {
        if !to_remove.contains(&i) {
            new_left.push(line.clone());
        }
    }

    s.left_lines = new_left;
    s.left_modified = true;
    recompute();
}

// ── Insert hunk (above/below) ───────────────────────────────────────────────

/// Insert the current hunk's left-side lines into the right side ABOVE the
/// corresponding position (i.e. the lines are added before the existing right
/// content at the hunk location).
fn insert_hunk_to_right_above() {
    let s = app();
    if s.hunks.is_empty() { return; }
    push_undo();
    let hunk_start = s.hunks[s.current_hunk].start;
    let hunk_end = s.hunks[s.current_hunk].end;

    // Find the right-side insertion point: the first right_idx in the hunk
    let mut insert_at: Option<usize> = None;
    for i in hunk_start..hunk_end {
        if let Some(ri) = s.diff_lines[i].right_idx {
            insert_at = Some(ri);
            break;
        }
    }
    // If no right_idx in hunk, insert after the last known right_idx before hunk
    let insert_at = insert_at.unwrap_or_else(|| {
        for i in (0..hunk_start).rev() {
            if let Some(ri) = s.diff_lines[i].right_idx {
                return ri + 1;
            }
        }
        0
    });

    // Collect left-side lines from the hunk
    let mut to_insert: Vec<String> = Vec::new();
    for i in hunk_start..hunk_end {
        if let Some(li) = s.diff_lines[i].left_idx {
            if li < s.left_lines.len() && (s.diff_lines[i].kind == DiffKind::Deleted || s.diff_lines[i].kind == DiffKind::Changed) {
                to_insert.push(s.left_lines[li].clone());
            }
        }
    }
    if to_insert.is_empty() { return; }

    // Insert into right_lines
    let mut new_right = Vec::new();
    for i in 0..insert_at {
        if i < s.right_lines.len() {
            new_right.push(s.right_lines[i].clone());
        }
    }
    new_right.extend(to_insert);
    for i in insert_at..s.right_lines.len() {
        new_right.push(s.right_lines[i].clone());
    }
    s.right_lines = new_right;
    s.right_modified = true;
    recompute();
}

/// Insert the current hunk's right-side lines into the left side ABOVE the
/// corresponding position.
fn insert_hunk_to_left_above() {
    let s = app();
    if s.hunks.is_empty() { return; }
    push_undo();
    let hunk_start = s.hunks[s.current_hunk].start;
    let hunk_end = s.hunks[s.current_hunk].end;

    let mut insert_at: Option<usize> = None;
    for i in hunk_start..hunk_end {
        if let Some(li) = s.diff_lines[i].left_idx {
            insert_at = Some(li);
            break;
        }
    }
    let insert_at = insert_at.unwrap_or_else(|| {
        for i in (0..hunk_start).rev() {
            if let Some(li) = s.diff_lines[i].left_idx {
                return li + 1;
            }
        }
        0
    });

    let mut to_insert: Vec<String> = Vec::new();
    for i in hunk_start..hunk_end {
        if let Some(ri) = s.diff_lines[i].right_idx {
            if ri < s.right_lines.len() && (s.diff_lines[i].kind == DiffKind::Added || s.diff_lines[i].kind == DiffKind::Changed) {
                to_insert.push(s.right_lines[ri].clone());
            }
        }
    }
    if to_insert.is_empty() { return; }

    let mut new_left = Vec::new();
    for i in 0..insert_at {
        if i < s.left_lines.len() {
            new_left.push(s.left_lines[i].clone());
        }
    }
    new_left.extend(to_insert);
    for i in insert_at..s.left_lines.len() {
        new_left.push(s.left_lines[i].clone());
    }
    s.left_lines = new_left;
    s.left_modified = true;
    recompute();
}

// ── Hunk tracking ───────────────────────────────────────────────────────────

fn update_current_hunk_for_row(row: usize) {
    let s = app();
    let mut found = None;
    for (i, h) in s.hunks.iter().enumerate() {
        if row >= h.start && row < h.end {
            found = Some(i);
            break;
        }
    }
    if let Some(i) = found {
        if s.current_hunk != i {
            s.current_hunk = i;
            populate_grid(s);
            update_labels(s);
        }
    }
}

// ── Save ────────────────────────────────────────────────────────────────────

fn save_left() {
    let s = app();
    if s.left_path.is_empty() { return; }
    if save_file(&s.left_path, &s.left_lines) {
        s.left_modified = false;
        update_labels(s);
    }
}

fn save_right() {
    let s = app();
    if s.right_path.is_empty() { return; }
    if save_file(&s.right_path, &s.right_lines) {
        s.right_modified = false;
        update_labels(s);
    }
}

// ── Edit ────────────────────────────────────────────────────────────────────

fn start_edit(row: usize) {
    let s = app();
    if row >= s.diff_lines.len() { return; }

    s.editing_row = Some(row);
    let dl = &s.diff_lines[row];

    // Populate left field
    let left_text = match dl.left_idx {
        Some(li) if li < s.left_lines.len() => s.left_lines[li].as_str(),
        _ => "",
    };
    s.edit_left_field.set_text(left_text);

    // Populate right field
    let right_text = match dl.right_idx {
        Some(ri) if ri < s.right_lines.len() => s.right_lines[ri].as_str(),
        _ => "",
    };
    s.edit_right_field.set_text(right_text);

    s.edit_panel.set_visible(true);
    s.edit_left_field.focus();
}

fn apply_edit() {
    let s = app();
    let row = match s.editing_row {
        Some(r) => r,
        None => return,
    };
    if row >= s.diff_lines.len() {
        cancel_edit();
        return;
    }

    push_undo();

    let new_left = get_text_field_text(&s.edit_left_field);
    let new_right = get_text_field_text(&s.edit_right_field);

    let dl = &s.diff_lines[row];

    // Update left line
    if let Some(li) = dl.left_idx {
        if li < s.left_lines.len() {
            if s.left_lines[li] != new_left {
                s.left_lines[li] = new_left;
                s.left_modified = true;
            }
        }
    } else if !new_left.is_empty() {
        // Line didn't exist on left (Added line) — insert it
        // Find the insertion position based on surrounding context
        let insert_pos = find_left_insert_pos(row);
        s.left_lines.insert(insert_pos, new_left);
        s.left_modified = true;
    }

    // Update right line
    if let Some(ri) = dl.right_idx {
        if ri < s.right_lines.len() {
            if s.right_lines[ri] != new_right {
                s.right_lines[ri] = new_right;
                s.right_modified = true;
            }
        }
    } else if !new_right.is_empty() {
        // Line didn't exist on right (Deleted line) — insert it
        let insert_pos = find_right_insert_pos(row);
        s.right_lines.insert(insert_pos, new_right);
        s.right_modified = true;
    }

    s.edit_panel.set_visible(false);
    s.editing_row = None;
    recompute();
}

fn find_left_insert_pos(row: usize) -> usize {
    let s = app();
    // Look backwards for the nearest line with a left_idx
    let mut i = row;
    loop {
        if i == 0 { return 0; }
        i -= 1;
        if let Some(li) = s.diff_lines[i].left_idx {
            return li + 1;
        }
    }
}

fn find_right_insert_pos(row: usize) -> usize {
    let s = app();
    let mut i = row;
    loop {
        if i == 0 { return 0; }
        i -= 1;
        if let Some(ri) = s.diff_lines[i].right_idx {
            return ri + 1;
        }
    }
}

fn cancel_edit() {
    let s = app();
    s.edit_panel.set_visible(false);
    s.editing_row = None;
}

// ── Window title update ─────────────────────────────────────────────────────

fn update_title() {
    let s = app();
    let title = make_title(&s.left_path, &s.right_path);
    s.win.set_title(&title);
}

// ── Search ──────────────────────────────────────────────────────────────────

fn toggle_search() {
    let s = app();
    if s.search_active {
        close_search();
    } else {
        open_search();
    }
}

fn open_search() {
    let s = app();
    s.search_active = true;
    s.search_panel.set_visible(true);
    s.search_field.focus();
}

fn close_search() {
    let s = app();
    s.search_active = false;
    s.search_panel.set_visible(false);
    s.search_matches.clear();
    s.search_current = 0;
    // Remove search highlighting by repopulating
    populate_grid(s);
}

fn do_search() {
    let s = app();
    let mut buf = [0u8; 256];
    let len = s.search_field.get_text(&mut buf);
    if len == 0 {
        s.search_matches.clear();
        s.search_current = 0;
        populate_grid(s);
        return;
    }
    let query = core::str::from_utf8(&buf[..len as usize]).unwrap_or("");
    let query_lower = query.to_lowercase();

    s.search_matches.clear();
    for (i, dl) in s.diff_lines.iter().enumerate() {
        if let Some(li) = dl.left_idx {
            if s.left_lines[li].to_lowercase().contains(&query_lower) {
                s.search_matches.push(i);
                continue;
            }
        }
        if let Some(ri) = dl.right_idx {
            if s.right_lines[ri].to_lowercase().contains(&query_lower) {
                s.search_matches.push(i);
            }
        }
    }
    s.search_current = 0;
    if !s.search_matches.is_empty() {
        let row = s.search_matches[0];
        s.grid.set_selected_row(row as u32);
    }
    let count = s.search_matches.len();
    let info = anyos_std::format!("{} matches", count);
    s.status_label.set_text(&info);
}

fn search_next() {
    let s = app();
    if s.search_matches.is_empty() { return; }
    s.search_current = (s.search_current + 1) % s.search_matches.len();
    let row = s.search_matches[s.search_current];
    s.grid.set_selected_row(row as u32);
    let info = anyos_std::format!("{}/{}", s.search_current + 1, s.search_matches.len());
    s.status_label.set_text(&info);
}

fn search_prev() {
    let s = app();
    if s.search_matches.is_empty() { return; }
    if s.search_current == 0 {
        s.search_current = s.search_matches.len() - 1;
    } else {
        s.search_current -= 1;
    }
    let row = s.search_matches[s.search_current];
    s.grid.set_selected_row(row as u32);
    let info = anyos_std::format!("{}/{}", s.search_current + 1, s.search_matches.len());
    s.status_label.set_text(&info);
}

// ── Fullscreen ──────────────────────────────────────────────────────────────

fn toggle_fullscreen() {
    let s = app();
    if s.fullscreen {
        // Restore
        s.win.set_position(s.pre_fullscreen_pos.0, s.pre_fullscreen_pos.1);
        s.win.set_size(s.pre_fullscreen_size.0, s.pre_fullscreen_size.1);
        s.fullscreen = false;
    } else {
        // Save current size/pos and maximize
        s.pre_fullscreen_size = s.win.get_size();
        s.pre_fullscreen_pos = s.win.get_position();
        let (sw, sh) = anyui::screen_size();
        s.win.set_position(0, 0);
        s.win.set_size(sw, sh);
        s.fullscreen = true;
    }
}

// ── Go to Line ──────────────────────────────────────────────────────────────

fn toggle_goto_line() {
    let s = app();
    if s.goto_active {
        close_goto_line();
    } else {
        open_goto_line();
    }
}

fn open_goto_line() {
    let s = app();
    s.goto_active = true;
    s.goto_panel.set_visible(true);
    s.goto_field.set_text("");
    s.goto_field.focus();
}

fn close_goto_line() {
    let s = app();
    s.goto_active = false;
    s.goto_panel.set_visible(false);
}

fn do_goto_line() {
    let s = app();
    let mut buf = [0u8; 64];
    let len = s.goto_field.get_text(&mut buf);
    if len == 0 { return; }
    let text = core::str::from_utf8(&buf[..len as usize]).unwrap_or("");
    let text = text.trim();
    if let Some(line_num) = parse_number(text) {
        if line_num == 0 { return; }
        // Find the diff_lines row that contains this line number (left or right)
        for (i, dl) in s.diff_lines.iter().enumerate() {
            if let Some(li) = dl.left_idx {
                if li + 1 == line_num {
                    s.grid.set_selected_row(i as u32);
                    close_goto_line();
                    return;
                }
            }
            if let Some(ri) = dl.right_idx {
                if ri + 1 == line_num {
                    s.grid.set_selected_row(i as u32);
                    close_goto_line();
                    return;
                }
            }
        }
        let msg = anyos_std::format!("Line {} not found", line_num);
        s.status_label.set_text(&msg);
    }
}

fn parse_number(s: &str) -> Option<usize> {
    let mut n: usize = 0;
    let mut any = false;
    for b in s.bytes() {
        if b >= b'0' && b <= b'9' {
            n = n.wrapping_mul(10).wrapping_add((b - b'0') as usize);
            any = true;
        } else {
            break;
        }
    }
    if any { Some(n) } else { None }
}

// ── Text filter dialog ──────────────────────────────────────────────────────

fn prompt_text_filter() {
    // Re-use the goto_line panel for text filter input
    let s = app();
    s.goto_active = true;
    s.goto_panel.set_visible(true);
    s.goto_field.set_text(&s.text_filter);
    s.goto_field.focus();
    s.status_label.set_text("Enter text filter (empty to clear):");
}

// ── Output save ─────────────────────────────────────────────────────────────

fn save_output() {
    let s = app();
    if s.output_path.is_empty() { return; }
    // Write unified diff output
    let mut output = String::new();
    let left_name = if s.left_label.is_empty() { &s.left_path } else { &s.left_label };
    let right_name = if s.right_label.is_empty() { &s.right_path } else { &s.right_label };
    output.push_str("--- ");
    output.push_str(left_name);
    output.push('\n');
    output.push_str("+++ ");
    output.push_str(right_name);
    output.push('\n');
    for dl in &s.diff_lines {
        match dl.kind {
            DiffKind::Equal => {
                output.push(' ');
                if let Some(li) = dl.left_idx {
                    if li < s.left_lines.len() {
                        output.push_str(&s.left_lines[li]);
                    }
                }
                output.push('\n');
            }
            DiffKind::Deleted => {
                output.push('-');
                if let Some(li) = dl.left_idx {
                    if li < s.left_lines.len() {
                        output.push_str(&s.left_lines[li]);
                    }
                }
                output.push('\n');
            }
            DiffKind::Added => {
                output.push('+');
                if let Some(ri) = dl.right_idx {
                    if ri < s.right_lines.len() {
                        output.push_str(&s.right_lines[ri]);
                    }
                }
                output.push('\n');
            }
            DiffKind::Changed => {
                output.push('-');
                if let Some(li) = dl.left_idx {
                    if li < s.left_lines.len() {
                        output.push_str(&s.left_lines[li]);
                    }
                }
                output.push('\n');
                output.push('+');
                if let Some(ri) = dl.right_idx {
                    if ri < s.right_lines.len() {
                        output.push_str(&s.right_lines[ri]);
                    }
                }
                output.push('\n');
            }
        }
    }
    if anyos_std::fs::write_bytes(&s.output_path, output.as_bytes()).is_ok() {
        let msg = anyos_std::format!("Output saved to {}", s.output_path);
        s.status_label.set_text(&msg);
    }
}

// ── Clipboard ───────────────────────────────────────────────────────────────

fn copy_selected_line() {
    let s = app();
    let row = s.grid.selected_row();
    if row == u32::MAX { return; }
    let row = row as usize;
    if row >= s.diff_lines.len() { return; }
    let dl = &s.diff_lines[row];

    let mut text = String::new();
    if let Some(li) = dl.left_idx {
        text.push_str(&s.left_lines[li]);
    }
    if let Some(ri) = dl.right_idx {
        if !text.is_empty() { text.push('\t'); }
        text.push_str(&s.right_lines[ri]);
    }
    anyui::clipboard_set(&text);
    s.status_label.set_text("Copied to clipboard");
}

// ── Keyboard handler ────────────────────────────────────────────────────────

fn handle_key(ke: &anyui::KeyEvent) {
    // Ctrl+F: Search
    if ke.ctrl() && (ke.char_code == b'f' as u32 || ke.char_code == b'F' as u32) {
        toggle_search();
        return;
    }
    // Escape: Close search/goto or cancel edit
    if ke.keycode == anyui::KEY_ESCAPE {
        let s = app();
        if s.search_active {
            close_search();
        } else if s.goto_active {
            close_goto_line();
        } else if s.editing_row.is_some() {
            cancel_edit();
        }
        return;
    }
    // Ctrl+L: Go to Line
    if ke.ctrl() && (ke.char_code == b'l' as u32 || ke.char_code == b'L' as u32) {
        toggle_goto_line();
        return;
    }
    // Ctrl+T: Cycle color theme
    if ke.ctrl() && (ke.char_code == b't' as u32 || ke.char_code == b'T' as u32) {
        cycle_theme();
        return;
    }
    // Ctrl+Shift+Right: Insert hunk to right (above)
    if ke.ctrl() && ke.shift() && ke.keycode == anyui::KEY_RIGHT {
        insert_hunk_to_right_above();
        return;
    }
    // Ctrl+Shift+Left: Insert hunk to left (above)
    if ke.ctrl() && ke.shift() && ke.keycode == anyui::KEY_LEFT {
        insert_hunk_to_left_above();
        return;
    }
    // Ctrl+Z: Undo
    if ke.ctrl() && (ke.char_code == b'z' as u32 || ke.char_code == b'Z' as u32) {
        undo();
        return;
    }
    // Ctrl+Y: Redo
    if ke.ctrl() && (ke.char_code == b'y' as u32 || ke.char_code == b'Y' as u32) {
        redo();
        return;
    }
    // Ctrl+S: Save both
    if ke.ctrl() && (ke.char_code == b's' as u32 || ke.char_code == b'S' as u32) {
        save_left();
        save_right();
        return;
    }
    // Ctrl+C: Copy selected line
    if ke.ctrl() && (ke.char_code == b'c' as u32 || ke.char_code == b'C' as u32) {
        copy_selected_line();
        return;
    }
    // Ctrl+R: Reload
    if ke.ctrl() && (ke.char_code == b'r' as u32 || ke.char_code == b'R' as u32) {
        refresh();
        return;
    }
    // F3 or Ctrl+G: Search next
    if ke.keycode == anyui::KEY_F3 || (ke.ctrl() && (ke.char_code == b'g' as u32 || ke.char_code == b'G' as u32)) {
        search_next();
        return;
    }
    // Alt+Down or Ctrl+Down: Next hunk
    if (ke.alt() || ke.ctrl()) && ke.keycode == anyui::KEY_DOWN {
        navigate_next();
        return;
    }
    // Alt+Up or Ctrl+Up: Previous hunk
    if (ke.alt() || ke.ctrl()) && ke.keycode == anyui::KEY_UP {
        navigate_prev();
        return;
    }
    // Alt+Right: Merge current hunk to right
    if ke.alt() && ke.keycode == anyui::KEY_RIGHT {
        merge_hunk_to_right();
        return;
    }
    // Alt+Left: Merge current hunk to left
    if ke.alt() && ke.keycode == anyui::KEY_LEFT {
        merge_hunk_to_left();
        return;
    }
    // F11: Fullscreen
    if ke.keycode == anyui::KEY_F11 {
        toggle_fullscreen();
        return;
    }
    // F5: Refresh/Reload
    if ke.keycode == anyui::KEY_F5 {
        refresh();
        return;
    }
}

// ── Main entry ──────────────────────────────────────────────────────────────

fn main() {
    if !anyui::init() {
        anyos_std::println!("diff: failed to load libanyui.so");
        return;
    }

    // Parse command line arguments
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf).trim();
    let cli = parse_args(raw);
    let left_path = cli.left_path;
    let right_path = cli.right_path;

    // Detect syntax languages
    let left_lang = detect_language(&left_path);
    let right_lang = detect_language(&right_path);

    // Load files
    let left_lines = if !left_path.is_empty() { load_lines(&left_path) } else { Vec::new() };
    let right_lines = if !right_path.is_empty() { load_lines(&right_path) } else { Vec::new() };

    // Create window with labels if provided
    let title = if !cli.left_label.is_empty() || !cli.right_label.is_empty() {
        let ll = if cli.left_label.is_empty() { left_path.rsplit('/').next().unwrap_or("(none)") } else { &cli.left_label };
        let rl = if cli.right_label.is_empty() { right_path.rsplit('/').next().unwrap_or("(none)") } else { &cli.right_label };
        anyos_std::format!("{} vs {} - Diff", ll, rl)
    } else {
        make_title(&left_path, &right_path)
    };
    let win = anyui::Window::new(&title, -1, -1, 800, 500);

    let tc = anyui::theme::colors();

    // ── Toolbar ──
    let toolbar = anyui::Toolbar::new();
    toolbar.set_dock(anyui::DOCK_TOP);
    toolbar.set_size(800, 36);
    toolbar.set_color(tc.toolbar_bg);
    toolbar.set_padding(4, 4, 4, 4);

    let ic = tc.text_secondary; // icon color
    let isz: u32 = 18;         // icon size

    let btn_open_left = toolbar.add_icon_button("");
    btn_open_left.set_size(34, 28);
    btn_open_left.set_system_icon("folder-open", IconType::Outline, ic, isz);
    btn_open_left.set_tooltip("Open Left");

    let btn_open_right = toolbar.add_icon_button("");
    btn_open_right.set_size(34, 28);
    btn_open_right.set_system_icon("folder-open", IconType::Outline, ic, isz);
    btn_open_right.set_tooltip("Open Right");

    let btn_refresh = toolbar.add_icon_button("");
    btn_refresh.set_size(34, 28);
    btn_refresh.set_system_icon("refresh", IconType::Outline, ic, isz);
    btn_refresh.set_tooltip("Reload");

    toolbar.add_separator();

    let btn_undo = toolbar.add_icon_button("");
    btn_undo.set_size(34, 28);
    btn_undo.set_system_icon("arrow-back-up", IconType::Outline, ic, isz);
    btn_undo.set_tooltip("Undo");

    let btn_redo = toolbar.add_icon_button("");
    btn_redo.set_size(34, 28);
    btn_redo.set_system_icon("arrow-forward-up", IconType::Outline, ic, isz);
    btn_redo.set_tooltip("Redo");

    toolbar.add_separator();

    let btn_prev = toolbar.add_icon_button("");
    btn_prev.set_size(34, 28);
    btn_prev.set_system_icon("chevrons-left", IconType::Outline, ic, isz);
    btn_prev.set_tooltip("Previous Hunk");

    let btn_next = toolbar.add_icon_button("");
    btn_next.set_size(34, 28);
    btn_next.set_system_icon("chevrons-right", IconType::Outline, ic, isz);
    btn_next.set_tooltip("Next Hunk");

    toolbar.add_separator();

    let btn_merge_right = toolbar.add_icon_button("");
    btn_merge_right.set_size(34, 28);
    btn_merge_right.set_system_icon("arrow-bar-to-right", IconType::Outline, ic, isz);
    btn_merge_right.set_tooltip("Merge Right");

    let btn_merge_left = toolbar.add_icon_button("");
    btn_merge_left.set_size(34, 28);
    btn_merge_left.set_system_icon("arrow-bar-to-left", IconType::Outline, ic, isz);
    btn_merge_left.set_tooltip("Merge Left");

    let btn_del_left = toolbar.add_icon_button("");
    btn_del_left.set_size(34, 28);
    btn_del_left.set_system_icon("trash", IconType::Outline, tc.destructive, isz);
    btn_del_left.set_tooltip("Delete Left");

    let btn_del_right = toolbar.add_icon_button("");
    btn_del_right.set_size(34, 28);
    btn_del_right.set_system_icon("trash", IconType::Outline, tc.destructive, isz);
    btn_del_right.set_tooltip("Delete Right");

    toolbar.add_separator();

    let btn_save_left = toolbar.add_icon_button("");
    btn_save_left.set_size(34, 28);
    btn_save_left.set_system_icon("device-floppy", IconType::Outline, ic, isz);
    btn_save_left.set_tooltip("Save Left");

    let btn_save_right = toolbar.add_icon_button("");
    btn_save_right.set_size(34, 28);
    btn_save_right.set_system_icon("device-floppy", IconType::Outline, ic, isz);
    btn_save_right.set_tooltip("Save Right");

    toolbar.add_separator();

    let stats_label = toolbar.add_label("0A 0D 0C");

    win.add(&toolbar);

    // ── Status bar (add DOCK_BOTTOM first — goes to very bottom) ──
    let status_bar = anyui::View::new();
    status_bar.set_dock(anyui::DOCK_BOTTOM);
    status_bar.set_size(800, 24);
    status_bar.set_color(tc.sidebar_bg);

    let status_label = anyui::Label::new("");
    status_label.set_position(8, 4);
    status_label.set_text_color(tc.text_secondary);
    status_label.set_font_size(12);
    status_bar.add(&status_label);

    let hunk_label = anyui::Label::new("");
    hunk_label.set_position(600, 4);
    hunk_label.set_text_color(tc.text_secondary);
    hunk_label.set_font_size(12);
    status_bar.add(&hunk_label);

    win.add(&status_bar);

    // ── Edit panel (DOCK_BOTTOM, added after status bar — appears above it) ──
    let edit_panel = anyui::View::new();
    edit_panel.set_dock(anyui::DOCK_BOTTOM);
    edit_panel.set_size(800, 80);
    edit_panel.set_color(tc.window_bg);
    edit_panel.set_visible(false);

    let lbl_left = anyui::Label::new("Left:");
    lbl_left.set_position(8, 10);
    lbl_left.set_text_color(tc.text);
    lbl_left.set_font_size(13);
    edit_panel.add(&lbl_left);

    let edit_left_field = anyui::TextField::new();
    edit_left_field.set_position(52, 6);
    edit_left_field.set_size(500, 26);
    edit_panel.add(&edit_left_field);

    let lbl_right = anyui::Label::new("Right:");
    lbl_right.set_position(8, 42);
    lbl_right.set_text_color(tc.text);
    lbl_right.set_font_size(13);
    edit_panel.add(&lbl_right);

    let edit_right_field = anyui::TextField::new();
    edit_right_field.set_position(52, 38);
    edit_right_field.set_size(500, 26);
    edit_panel.add(&edit_right_field);

    let btn_apply = anyui::Button::new("Apply");
    btn_apply.set_position(570, 8);
    btn_apply.set_size(70, 26);
    edit_panel.add(&btn_apply);

    let btn_cancel = anyui::Button::new("Cancel");
    btn_cancel.set_position(570, 38);
    btn_cancel.set_size(70, 26);
    edit_panel.add(&btn_cancel);

    win.add(&edit_panel);

    // ── Search panel (DOCK_BOTTOM, above edit panel) ──
    let search_panel = anyui::View::new();
    search_panel.set_dock(anyui::DOCK_BOTTOM);
    search_panel.set_size(800, 32);
    search_panel.set_color(tc.card_bg);
    search_panel.set_visible(false);

    let search_lbl = anyui::Label::new("Find:");
    search_lbl.set_position(8, 7);
    search_lbl.set_text_color(tc.text);
    search_lbl.set_font_size(13);
    search_panel.add(&search_lbl);

    let search_field = anyui::TextField::new();
    search_field.set_position(50, 3);
    search_field.set_size(300, 26);
    search_panel.add(&search_field);

    let btn_search_prev = anyui::Button::new("<");
    btn_search_prev.set_position(360, 3);
    btn_search_prev.set_size(28, 26);
    search_panel.add(&btn_search_prev);

    let btn_search_next = anyui::Button::new(">");
    btn_search_next.set_position(392, 3);
    btn_search_next.set_size(28, 26);
    search_panel.add(&btn_search_next);

    let btn_search_close = anyui::Button::new("X");
    btn_search_close.set_position(430, 3);
    btn_search_close.set_size(28, 26);
    search_panel.add(&btn_search_close);

    win.add(&search_panel);

    // ── Go to Line panel (DOCK_BOTTOM) ──
    let goto_panel = anyui::View::new();
    goto_panel.set_dock(anyui::DOCK_BOTTOM);
    goto_panel.set_size(800, 32);
    goto_panel.set_color(tc.card_bg);
    goto_panel.set_visible(false);

    let goto_lbl = anyui::Label::new("Line:");
    goto_lbl.set_position(8, 7);
    goto_lbl.set_text_color(tc.text);
    goto_lbl.set_font_size(13);
    goto_panel.add(&goto_lbl);

    let goto_field = anyui::TextField::new();
    goto_field.set_position(50, 3);
    goto_field.set_size(120, 26);
    goto_panel.add(&goto_field);

    let btn_goto_go = anyui::Button::new("Go");
    btn_goto_go.set_position(180, 3);
    btn_goto_go.set_size(36, 26);
    goto_panel.add(&btn_goto_go);

    let btn_goto_close = anyui::Button::new("X");
    btn_goto_close.set_position(222, 3);
    btn_goto_close.set_size(28, 26);
    goto_panel.add(&btn_goto_close);

    win.add(&goto_panel);

    // ── DataGrid (DOCK_FILL added last) ──
    let grid = anyui::DataGrid::new(780, 430);
    grid.set_dock(anyui::DOCK_FILL);
    grid.set_row_height(20);
    grid.set_header_height(24);
    grid.set_columns(&[
        anyui::ColumnDef::new("#").width(50).align(anyui::ALIGN_RIGHT),
        anyui::ColumnDef::new("Left").width(340),
        anyui::ColumnDef::new(" ").width(30).align(anyui::ALIGN_CENTER),
        anyui::ColumnDef::new("#").width(50).align(anyui::ALIGN_RIGHT),
        anyui::ColumnDef::new("Right").width(340),
    ]);
    // ── Context menu on grid ──
    let ctx_menu = anyui::ContextMenu::new(
        "Save As Left|Save As Right|-|Ignore Whitespace|Ignore Blank Lines|Ignore Comments|-|Insert > Above|< Insert Above|-|Cycle Theme"
    );
    grid.set_context_menu(&ctx_menu);

    win.add(&grid);

    // ── Compute initial diff ──
    let ops = compute_edit_script(&left_lines, &right_lines);
    let diff_lines = build_diff_lines(&ops, &left_lines, &right_lines);
    let hunks = extract_hunks(&diff_lines);
    let (num_added, num_deleted, num_changed) = count_stats(&diff_lines);

    // ── Initialize global state ──
    unsafe {
        APP = Some(AppState {
            win,
            grid,
            stats_label,
            status_label,
            hunk_label,
            left_lines,
            right_lines,
            left_path,
            right_path,
            diff_lines,
            hunks,
            current_hunk: 0,
            num_added,
            num_deleted,
            num_changed,
            left_modified: false,
            right_modified: false,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            ignore_whitespace: false,
            ignore_blank_lines: false,
            ignore_comments: false,
            text_filter: String::new(),
            edit_panel,
            edit_left_field,
            edit_right_field,
            editing_row: None,
            search_panel,
            search_field,
            search_active: false,
            search_matches: Vec::new(),
            search_current: 0,
            goto_panel,
            goto_field,
            goto_active: false,
            fullscreen: false,
            pre_fullscreen_size: (800, 500),
            pre_fullscreen_pos: (0, 0),
            theme: THEME_DARK,
            left_lang,
            right_lang,
            left_label: cli.left_label,
            right_label: cli.right_label,
            output_path: cli.output_path,
            auto_compare: cli.auto_compare,
        });
    }

    // Populate grid and labels
    let s = app();
    populate_grid(s);
    update_labels(s);

    // ── Register callbacks ──
    btn_open_left.on_click(|_| { open_left(); });
    btn_open_right.on_click(|_| { open_right(); });
    btn_refresh.on_click(|_| { refresh(); });
    btn_undo.on_click(|_| { undo(); });
    btn_redo.on_click(|_| { redo(); });
    btn_prev.on_click(|_| { navigate_prev(); });
    btn_next.on_click(|_| { navigate_next(); });
    btn_merge_right.on_click(|_| { merge_hunk_to_right(); });
    btn_merge_left.on_click(|_| { merge_hunk_to_left(); });
    btn_del_left.on_click(|_| { delete_hunk_left(); });
    btn_del_right.on_click(|_| { delete_hunk_right(); });
    btn_save_left.on_click(|_| { save_left(); });
    btn_save_right.on_click(|_| { save_right(); });
    btn_apply.on_click(|_| { apply_edit(); });
    btn_cancel.on_click(|_| { cancel_edit(); });

    // Context menu
    ctx_menu.on_item_click(|e| {
        match e.index {
            0 => save_as_left(),
            1 => save_as_right(),
            3 => toggle_ignore_whitespace(),
            4 => toggle_ignore_blank_lines(),
            5 => toggle_ignore_comments(),
            7 => insert_hunk_to_right_above(),
            8 => insert_hunk_to_left_above(),
            10 => cycle_theme(),
            _ => {}
        }
    });

    // Track current hunk on row selection
    app().grid.on_selection_changed(|e| {
        if e.index != u32::MAX {
            update_current_hunk_for_row(e.index as usize);
        }
    });

    // Double-click a row to edit
    app().grid.on_submit(|e| {
        start_edit(e.index as usize);
    });

    win.on_close(|_| {
        save_output();
        anyui::quit();
    });

    // Search callbacks
    btn_search_next.on_click(|_| { search_next(); });
    btn_search_prev.on_click(|_| { search_prev(); });
    btn_search_close.on_click(|_| { close_search(); });
    search_field.on_submit(|_| { do_search(); });

    // Go to Line callbacks
    btn_goto_go.on_click(|_| { do_goto_line(); });
    btn_goto_close.on_click(|_| { close_goto_line(); });
    goto_field.on_submit(|_| { do_goto_line(); });

    // Keyboard shortcuts (fires for unhandled keys bubbling to window)
    win.on_key_down(|ke| { handle_key(ke); });

    // Auto-compare: if --auto-compare flag was set and output path provided,
    // save output immediately and quit
    if cli.auto_compare {
        save_output();
    }

    // ── Run event loop ──
    anyui::run();
}
