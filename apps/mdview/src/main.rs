#![no_std]
#![no_main]

use anyos_std::String;
use anyos_std::Vec;
use libanyui_client as anyui;

anyos_std::entry!(main);

// ── Font IDs ──────────────────────────────────────────────────────────────────
const FONT_REGULAR: u32 = 0;
const FONT_BOLD: u32 = 1;
const FONT_MONO: u32 = 4;

// ── Colors (dark theme) ──────────────────────────────────────────────────────
const COLOR_BG: u32 = 0xFF1E1E1E;
const COLOR_TOOLBAR: u32 = 0xFF252526;
const COLOR_STATUS: u32 = 0xFF252525;
const COLOR_TAB_BAR: u32 = 0xFF2D2D2D;

const TEXT_HEADING: u32 = 0xFFE0E0E0;
const TEXT_BODY: u32 = 0xFFCCCCCC;
const TEXT_CODE: u32 = 0xFFD4D4D4;
const TEXT_QUOTE: u32 = 0xFF9E9E9E;
const TEXT_LINK: u32 = 0xFF569CD6;
const TEXT_STATUS: u32 = 0xFF969696;
const TEXT_LIST_BULLET: u32 = 0xFF569CD6;

const BG_CODE_BLOCK: u32 = 0xFF0D0D0D;
const BG_CODE_INLINE: u32 = 0xFF2A2A2A;
const BG_QUOTE: u32 = 0xFF252525;

// ── Heading sizes ────────────────────────────────────────────────────────────
const H_SIZES: [u32; 6] = [28, 24, 20, 18, 16, 15];

// ── Wrap width (characters) ──────────────────────────────────────────────────
const WRAP_WIDTH: usize = 110;

// ── Data structures ──────────────────────────────────────────────────────────

struct OpenFile {
    path: String,
    content: String,
    panel: anyui::StackPanel,
    source_editor: Option<anyui::TextEditor>,
    showing_source: bool,
    modified: bool,
}

struct AppState {
    win: anyui::Window,
    tab_bar: anyui::TabBar,
    scroll: anyui::ScrollView,
    status_label: anyui::Label,
    path_label: anyui::Label,
    files: Vec<OpenFile>,
    active: usize,
}

static mut APP: Option<AppState> = None;

fn app() -> &'static mut AppState {
    unsafe { APP.as_mut().unwrap() }
}

// ── Utility ──────────────────────────────────────────────────────────────────

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn tab_labels(files: &[OpenFile]) -> String {
    let mut s = String::new();
    for (i, f) in files.iter().enumerate() {
        if i > 0 { s.push('|'); }
        if f.modified { s.push('*'); }
        s.push_str(basename(&f.path));
    }
    s
}

// ── Word wrapping ────────────────────────────────────────────────────────────

fn word_wrap(text: &str, max_chars: usize) -> String {
    let mut result = String::new();
    let mut col = 0usize;

    for word in text.split(' ') {
        if word.is_empty() { continue; }
        let wlen = word.len();
        if col > 0 && col + 1 + wlen > max_chars {
            result.push('\n');
            col = 0;
        }
        if col > 0 {
            result.push(' ');
            col += 1;
        }
        result.push_str(word);
        col += wlen;
    }
    result
}

// ── Emoji shortcode replacement ──────────────────────────────────────────────

/// Map emoji shortcodes to Unicode emoji characters.
/// Rendered via NotoColorEmoji bitmap font (CBDT/CBLC) with cross-font fallback.
fn emoji_for_shortcode(code: &str) -> Option<&'static str> {
    match code {
        // Faces
        "smile" | "smiley" | "grinning" => Some("\u{1F600}"),
        "grin" => Some("\u{1F601}"),
        "laughing" | "satisfied" => Some("\u{1F606}"),
        "joy" => Some("\u{1F602}"),
        "wink" => Some("\u{1F609}"),
        "blush" => Some("\u{1F60A}"),
        "heart_eyes" => Some("\u{1F60D}"),
        "kissing_heart" => Some("\u{1F618}"),
        "yum" => Some("\u{1F60B}"),
        "sunglasses" | "cool" => Some("\u{1F60E}"),
        "smirk" => Some("\u{1F60F}"),
        "relaxed" => Some("\u{263A}"),
        "stuck_out_tongue" => Some("\u{1F61B}"),
        "stuck_out_tongue_winking_eye" => Some("\u{1F61C}"),
        "stuck_out_tongue_closed_eyes" => Some("\u{1F61D}"),
        "disappointed" | "sad" => Some("\u{1F61E}"),
        "worried" => Some("\u{1F61F}"),
        "angry" => Some("\u{1F620}"),
        "rage" => Some("\u{1F621}"),
        "cry" => Some("\u{1F622}"),
        "sob" => Some("\u{1F62D}"),
        "fearful" => Some("\u{1F628}"),
        "scream" => Some("\u{1F631}"),
        "sweat" => Some("\u{1F613}"),
        "sweat_smile" => Some("\u{1F605}"),
        "confused" => Some("\u{1F615}"),
        "pensive" => Some("\u{1F614}"),
        "flushed" => Some("\u{1F633}"),
        "sleeping" | "zzz" => Some("\u{1F634}"),
        "sleepy" => Some("\u{1F62A}"),
        "mask" => Some("\u{1F637}"),
        "neutral_face" => Some("\u{1F610}"),
        "expressionless" => Some("\u{1F611}"),
        "unamused" => Some("\u{1F612}"),
        "thinking" | "thinking_face" => Some("\u{1F914}"),
        "innocent" | "angel" => Some("\u{1F607}"),
        "imp" | "devil" => Some("\u{1F608}"),
        // Symbols
        "heart" => Some("\u{2764}"),
        "broken_heart" => Some("\u{1F494}"),
        "star" => Some("\u{2B50}"),
        "fire" | "flame" => Some("\u{1F525}"),
        "thumbsup" | "+1" => Some("\u{1F44D}"),
        "thumbsdown" | "-1" => Some("\u{1F44E}"),
        "clap" => Some("\u{1F44F}"),
        "wave" => Some("\u{1F44B}"),
        "ok_hand" => Some("\u{1F44C}"),
        "point_up" => Some("\u{261D}"),
        "point_down" => Some("\u{1F447}"),
        "point_left" => Some("\u{1F448}"),
        "point_right" => Some("\u{1F449}"),
        "raised_hands" => Some("\u{1F64C}"),
        "pray" => Some("\u{1F64F}"),
        "muscle" => Some("\u{1F4AA}"),
        "eyes" => Some("\u{1F440}"),
        "warning" => Some("\u{26A0}"),
        "white_check_mark" | "check" => Some("\u{2705}"),
        "x" | "cross_mark" => Some("\u{274C}"),
        "exclamation" | "heavy_exclamation_mark" => Some("\u{2757}"),
        "question" => Some("\u{2753}"),
        "100" => Some("\u{1F4AF}"),
        "rocket" => Some("\u{1F680}"),
        "tada" => Some("\u{1F389}"),
        "sparkles" => Some("\u{2728}"),
        "zap" | "lightning" => Some("\u{26A1}"),
        "bulb" => Some("\u{1F4A1}"),
        "bug" => Some("\u{1F41B}"),
        "gear" => Some("\u{2699}"),
        "lock" => Some("\u{1F512}"),
        "key" => Some("\u{1F511}"),
        "hammer" => Some("\u{1F528}"),
        "wrench" => Some("\u{1F527}"),
        "package" => Some("\u{1F4E6}"),
        "book" | "open_book" => Some("\u{1F4D6}"),
        "memo" | "pencil" => Some("\u{1F4DD}"),
        "clipboard" => Some("\u{1F4CB}"),
        "calendar" => Some("\u{1F4C5}"),
        "link" => Some("\u{1F517}"),
        "email" | "envelope" => Some("\u{1F4E7}"),
        "computer" | "desktop_computer" => Some("\u{1F4BB}"),
        "phone" | "telephone" => Some("\u{1F4DE}"),
        "globe_with_meridians" | "earth_americas" => Some("\u{1F30E}"),
        "sun" | "sunny" => Some("\u{2600}"),
        "cloud" => Some("\u{2601}"),
        "umbrella" => Some("\u{2614}"),
        "snowflake" => Some("\u{2744}"),
        "coffee" => Some("\u{2615}"),
        "pizza" => Some("\u{1F355}"),
        "beer" => Some("\u{1F37A}"),
        "trophy" => Some("\u{1F3C6}"),
        "gem" => Some("\u{1F48E}"),
        "crown" => Some("\u{1F451}"),
        "bell" => Some("\u{1F514}"),
        "mega" | "loudspeaker" => Some("\u{1F4E2}"),
        "speech_balloon" => Some("\u{1F4AC}"),
        "thought_balloon" => Some("\u{1F4AD}"),
        _ => None,
    }
}

// ── UTF-8 helper ────────────────────────────────────────────────────────────

/// Push one UTF-8 character from `bytes` at `pos` into `out`.
/// Returns the number of bytes consumed.
fn push_utf8(out: &mut String, bytes: &[u8], pos: usize) -> usize {
    let b = bytes[pos];
    let seq_len = if b < 0x80 { 1 }
        else if b < 0xE0 { 2 }
        else if b < 0xF0 { 3 }
        else { 4 };
    let end = (pos + seq_len).min(bytes.len());
    if let Ok(s) = core::str::from_utf8(&bytes[pos..end]) {
        out.push_str(s);
    } else {
        out.push(b as char);
    }
    end - pos
}

// ── Typographic replacements ────────────────────────────────────────────────

/// Replace typographic shortcuts: (c)→©, (r)→®, (tm)→™, (p)→§, +-→±, ...→…, --→–, ---→—
fn replace_typographic(text: &str) -> String {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut out = String::new();
    let mut i = 0;

    while i < len {
        // (tm) (TM) (Tm) → ™  (4 chars, check before 3-char patterns)
        if i + 3 < len && bytes[i] == b'(' {
            let b1 = bytes[i + 1];
            let b2 = bytes[i + 2];
            let b3 = bytes[i + 3];
            if (b1 == b't' || b1 == b'T') && (b2 == b'm' || b2 == b'M') && b3 == b')' {
                out.push('\u{2122}');
                i += 4;
                continue;
            }
        }
        // (c) (C) → ©
        if i + 2 < len && bytes[i] == b'(' && (bytes[i + 1] == b'c' || bytes[i + 1] == b'C') && bytes[i + 2] == b')' {
            out.push('\u{00A9}');
            i += 3;
            continue;
        }
        // (r) (R) → ®
        if i + 2 < len && bytes[i] == b'(' && (bytes[i + 1] == b'r' || bytes[i + 1] == b'R') && bytes[i + 2] == b')' {
            out.push('\u{00AE}');
            i += 3;
            continue;
        }
        // (p) (P) → §
        if i + 2 < len && bytes[i] == b'(' && (bytes[i + 1] == b'p' || bytes[i + 1] == b'P') && bytes[i + 2] == b')' {
            out.push('\u{00A7}');
            i += 3;
            continue;
        }
        // --- → em dash (before --)
        if i + 2 < len && bytes[i] == b'-' && bytes[i + 1] == b'-' && bytes[i + 2] == b'-' {
            out.push('\u{2014}');
            i += 3;
            continue;
        }
        // -- → en dash
        if i + 1 < len && bytes[i] == b'-' && bytes[i + 1] == b'-' {
            out.push('\u{2013}');
            i += 2;
            continue;
        }
        // ... → ellipsis
        if i + 2 < len && bytes[i] == b'.' && bytes[i + 1] == b'.' && bytes[i + 2] == b'.' {
            out.push('\u{2026}');
            i += 3;
            continue;
        }
        // +- → ±
        if i + 1 < len && bytes[i] == b'+' && bytes[i + 1] == b'-' {
            out.push('\u{00B1}');
            i += 2;
            continue;
        }
        i += push_utf8(&mut out, bytes, i);
    }
    out
}

/// Try to match an emoticon shortcut at position `i`. Returns (emoji, length) or None.
fn match_emoticon(bytes: &[u8], i: usize) -> Option<(&'static str, usize)> {
    let rem = bytes.len() - i;
    // 3-char emoticons
    if rem >= 3 {
        match &bytes[i..i + 3] {
            b":-)" => return Some(("\u{1F642}", 3)),  // slightly smiling face
            b":-(" => return Some(("\u{1F61E}", 3)),  // disappointed face
            b":-D" => return Some(("\u{1F600}", 3)),  // grinning face
            b":-P" => return Some(("\u{1F61B}", 3)),  // tongue out
            b":-O" => return Some(("\u{1F62E}", 3)),  // open mouth
            b":-/" => return Some(("\u{1F615}", 3)),  // confused
            b":-|" => return Some(("\u{1F610}", 3)),  // neutral
            b":-*" => return Some(("\u{1F618}", 3)),  // kissing
            b"8-)" => return Some(("\u{1F60E}", 3)),  // sunglasses
            b";-)" => return Some(("\u{1F609}", 3)),  // winking
            b">:(" => return Some(("\u{1F620}", 3)),  // angry
            b">:)" => return Some(("\u{1F608}", 3)),  // devil
            _ => {}
        }
    }
    // 2-char emoticons
    if rem >= 2 {
        match &bytes[i..i + 2] {
            b":)" => return Some(("\u{1F642}", 2)),   // slightly smiling
            b":(" => return Some(("\u{1F61E}", 2)),   // disappointed
            b":D" => return Some(("\u{1F600}", 2)),   // grinning
            b":P" => return Some(("\u{1F61B}", 2)),   // tongue out
            b":O" => return Some(("\u{1F62E}", 2)),   // open mouth
            b";)" => return Some(("\u{1F609}", 2)),   // winking
            b"<3" => return Some(("\u{2764}", 2)),    // heart
            _ => {}
        }
    }
    None
}

fn replace_emojis(text: &str) -> String {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut out = String::new();
    let mut i = 0;

    while i < len {
        // Emoji shortcode :name:
        if bytes[i] == b':' && i + 2 < len {
            let start = i + 1;
            let mut end = start;
            while end < len && bytes[end] != b':' && bytes[end] != b' ' && bytes[end] != b'\n' {
                end += 1;
            }
            if end < len && bytes[end] == b':' && end > start {
                let code = &text[start..end];
                if let Some(emoji) = emoji_for_shortcode(code) {
                    out.push_str(emoji);
                    i = end + 1;
                    continue;
                }
            }
        }
        // Emoticon shortcuts (:-) 8-) ;) etc.)
        // Only match if preceded by whitespace/start or follows whitespace
        let at_boundary = i == 0 || bytes[i - 1] == b' ' || bytes[i - 1] == b'\n' || bytes[i - 1] == b'\t';
        if at_boundary {
            if let Some((emoji, consumed)) = match_emoticon(bytes, i) {
                // Check that emoticon ends at boundary (space, newline, end, or punctuation)
                let after = i + consumed;
                let end_ok = after >= len
                    || bytes[after] == b' '
                    || bytes[after] == b'\n'
                    || bytes[after] == b'\t'
                    || bytes[after] == b','
                    || bytes[after] == b'.'
                    || bytes[after] == b'!'
                    || bytes[after] == b'?';
                if end_ok {
                    out.push_str(emoji);
                    i += consumed;
                    continue;
                }
            }
        }
        i += push_utf8(&mut out, bytes, i);
    }
    out
}

// ── Link extraction ─────────────────────────────────────────────────────────

struct LinkInfo {
    display: String,
    url: String,
}

/// Extract [text](url) links and bare https:// URLs from markdown text.
fn extract_links(text: &str) -> Vec<LinkInfo> {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut links = Vec::new();
    let mut i = 0;

    while i < len {
        // ![alt](url) — skip images
        if bytes[i] == b'!' && i + 1 < len && bytes[i+1] == b'[' {
            i += 2;
            while i < len && bytes[i] != b']' { i += 1; }
            if i < len { i += 1; }
            if i < len && bytes[i] == b'(' {
                i += 1;
                while i < len && bytes[i] != b')' { i += 1; }
                if i < len { i += 1; }
            }
            continue;
        }
        // [text](url)
        if bytes[i] == b'[' && (i == 0 || bytes[i-1] != b'!') {
            let bracket_start = i;
            i += 1;
            let text_start = i;
            while i < len && bytes[i] != b']' { i += 1; }
            if i >= len { continue; }
            let link_text = &text[text_start..i];
            i += 1; // skip ]
            if i < len && bytes[i] == b'(' {
                i += 1;
                let url_start = i;
                // Handle optional title: [text](url "title")
                let mut paren_depth = 1;
                while i < len && paren_depth > 0 {
                    if bytes[i] == b'(' { paren_depth += 1; }
                    if bytes[i] == b')' { paren_depth -= 1; }
                    if paren_depth > 0 { i += 1; }
                }
                let url_raw = &text[url_start..i];
                // Strip title if present
                let url = if let Some(space) = url_raw.find(' ') {
                    url_raw[..space].trim()
                } else {
                    url_raw.trim()
                };
                if i < len { i += 1; } // skip )
                if !url.is_empty() && (url.starts_with("http://") || url.starts_with("https://")) {
                    links.push(LinkInfo {
                        display: String::from(link_text),
                        url: String::from(url),
                    });
                }
            }
            continue;
        }
        // Bare URL: http:// or https://
        if i + 7 < len && (text[i..].starts_with("http://") || text[i..].starts_with("https://")) {
            let url_start = i;
            while i < len && bytes[i] != b' ' && bytes[i] != b'\n' && bytes[i] != b')' && bytes[i] != b'>' {
                i += 1;
            }
            let url = &text[url_start..i];
            links.push(LinkInfo {
                display: String::from(url),
                url: String::from(url),
            });
            continue;
        }
        // Skip one full UTF-8 character
        let b = bytes[i];
        i += if b < 0x80 { 1 } else if b < 0xE0 { 2 } else if b < 0xF0 { 3 } else { 4 };
    }
    links
}

// ── Inline markdown stripping ────────────────────────────────────────────────

/// Try to process a markdown link `[text](url)` or image `![alt](url)` at position `i`.
/// Returns `Some(new_i)` if handled (writing result to `out`), or `None`.
fn try_strip_link(text: &str, bytes: &[u8], i: usize, out: &mut String) -> Option<usize> {
    let len = bytes.len();
    // ![alt](url) → [Image: alt]
    if bytes[i] == b'!' && i + 1 < len && bytes[i+1] == b'[' {
        let mut j = i + 2;
        let start = j;
        while j < len && bytes[j] != b']' { j += 1; }
        let alt = &text[start..j];
        if j < len { j += 1; }
        if j < len && bytes[j] == b'(' {
            j += 1;
            while j < len && bytes[j] != b')' { j += 1; }
            if j < len { j += 1; }
        }
        out.push_str("[Image: ");
        out.push_str(alt);
        out.push(']');
        return Some(j);
    }
    // [text](url) → text  or  [text] → text
    if bytes[i] == b'[' {
        if i + 1 < len && bytes[i+1] == b'^' { return None; }
        let mut j = i + 1;
        let start = j;
        while j < len && bytes[j] != b']' { j += 1; }
        let link_text = &text[start..j];
        if j < len { j += 1; }
        if j < len && bytes[j] == b'(' {
            j += 1;
            while j < len && bytes[j] != b')' { j += 1; }
            if j < len { j += 1; }
        }
        out.push_str(link_text);
        return Some(j);
    }
    None
}

/// Strip common inline markers: **bold**, *italic*, `code`, [text](url)
fn strip_inline(text: &str) -> String {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut out = String::new();
    let mut i = 0;

    while i < len {
        // ~~strikethrough~~
        if i + 1 < len && bytes[i] == b'~' && bytes[i+1] == b'~' {
            i += 2;
            while i + 1 < len && !(bytes[i] == b'~' && bytes[i+1] == b'~') {
                if let Some(ni) = try_strip_link(text, bytes, i, &mut out) { i = ni; continue; }
                i += push_utf8(&mut out, bytes, i);
            }
            if i + 1 < len { i += 2; }
            continue;
        }
        // **bold** or __bold__
        if i + 1 < len && ((bytes[i] == b'*' && bytes[i+1] == b'*') || (bytes[i] == b'_' && bytes[i+1] == b'_')) {
            let marker = bytes[i];
            i += 2;
            while i + 1 < len && !(bytes[i] == marker && bytes[i+1] == marker) {
                if let Some(ni) = try_strip_link(text, bytes, i, &mut out) { i = ni; continue; }
                i += push_utf8(&mut out, bytes, i);
            }
            if i + 1 < len { i += 2; }
            continue;
        }
        // *italic* or _italic_ (single)
        if (bytes[i] == b'*' || bytes[i] == b'_') && i + 1 < len && bytes[i+1] != b' ' {
            let marker = bytes[i];
            i += 1;
            while i < len && bytes[i] != marker {
                if let Some(ni) = try_strip_link(text, bytes, i, &mut out) { i = ni; continue; }
                i += push_utf8(&mut out, bytes, i);
            }
            if i < len { i += 1; }
            continue;
        }
        // `inline code` — push content verbatim (no link processing)
        if bytes[i] == b'`' {
            i += 1;
            while i < len && bytes[i] != b'`' {
                i += push_utf8(&mut out, bytes, i);
            }
            if i < len { i += 1; }
            continue;
        }
        // Links and images
        if let Some(ni) = try_strip_link(text, bytes, i, &mut out) { i = ni; continue; }
        i += push_utf8(&mut out, bytes, i);
    }
    let typographic = replace_typographic(&out);
    replace_emojis(&typographic)
}

// ── Markdown renderer ────────────────────────────────────────────────────────

fn add_spacing(panel: &anyui::StackPanel, height: u32) {
    let spacer = anyui::Label::new("");
    spacer.set_size(10, height);
    panel.add(&spacer);
}

fn add_heading(panel: &anyui::StackPanel, text: &str, level: usize) {
    let lvl = if level > 6 { 5 } else { level - 1 };
    let size = H_SIZES[lvl];

    add_spacing(panel, if level <= 2 { 12 } else { 8 });

    let stripped = strip_inline(text);
    let label = anyui::Label::new(&stripped);
    label.set_font(FONT_BOLD);
    label.set_font_size(size);
    label.set_text_color(TEXT_HEADING);
    label.set_padding(16, 2, 16, 2);
    label.set_auto_size(true);
    panel.add(&label);

    // Underline for H1 and H2
    if level <= 2 {
        let div = anyui::Divider::new();
        div.set_size(800, 1);
        div.set_color(0xFF404040);
        div.set_margin(16, 4, 16, 4);
        panel.add(&div);
    }
}

fn add_paragraph(panel: &anyui::StackPanel, text: &str) {
    // Extract links before stripping inline markers
    let links = extract_links(text);

    let stripped = strip_inline(text);
    let wrapped = word_wrap(&stripped, WRAP_WIDTH);
    let label = anyui::Label::new(&wrapped);
    label.set_font(FONT_REGULAR);
    label.set_font_size(14);
    label.set_text_color(TEXT_BODY);
    label.set_padding(16, 4, 16, 4);
    label.set_auto_size(true);
    panel.add(&label);

    // Render clickable link tags below the paragraph text
    for link in &links {
        let display = if link.display == link.url {
            // Bare URL — show as-is
            anyos_std::format!("  \u{1F517} {}", link.url)
        } else {
            anyos_std::format!("  \u{1F517} {} \u{2014} {}", link.display, link.url)
        };
        let tag = anyui::Tag::new(&display);
        tag.set_text_color(TEXT_LINK);
        tag.set_color(0xFF2D2D30); // subtle dark background for link tags
        tag.set_font_size(12);
        // Measure text to set correct tag size (auto_size not implemented for Tags)
        let (tw, _th) = anyui::measure_text(&display, 0, 12);
        tag.set_size(tw + 16, 22); // 8px padding on each side, 22px height
        tag.set_padding(20, 0, 16, 2);
        let url = link.url.clone();
        tag.on_click(move |_| {
            anyos_std::process::spawn("/Applications/Surf.app", &url);
        });
        panel.add(&tag);
    }
}

fn add_code_block(panel: &anyui::StackPanel, lines: &[&str]) {
    let code_text = {
        let mut s = String::new();
        for (i, line) in lines.iter().enumerate() {
            if i > 0 { s.push('\n'); }
            s.push_str(line);
        }
        s
    };

    let container = anyui::View::new();
    container.set_color(BG_CODE_BLOCK);
    container.set_margin(16, 4, 16, 4);
    container.set_padding(12, 8, 12, 8);
    container.set_auto_size(true);

    let label = anyui::Label::new(&code_text);
    label.set_font(FONT_MONO);
    label.set_font_size(13);
    label.set_text_color(TEXT_CODE);
    label.set_auto_size(true);
    container.add(&label);

    panel.add(&container);
}

fn add_list_item(panel: &anyui::StackPanel, text: &str, ordered: bool, number: usize, indent_level: usize) {
    let stripped = strip_inline(text);
    // Build indent prefix: 2 spaces per indent level
    let mut prefix = String::new();
    prefix.push_str("  ");
    for _ in 0..indent_level {
        prefix.push_str("    ");
    }
    let prefixed = if ordered {
        anyos_std::format!("{}{}. {}", prefix, number, stripped)
    } else {
        let bullet = match indent_level {
            0 => "\u{2022}",  // •
            1 => "\u{25E6}",  // ◦
            _ => "\u{2023}",  // ‣
        };
        anyos_std::format!("{}{} {}", prefix, bullet, stripped)
    };
    let wrapped = word_wrap(&prefixed, WRAP_WIDTH);
    let label = anyui::Label::new(&wrapped);
    label.set_font(FONT_REGULAR);
    label.set_font_size(14);
    label.set_text_color(TEXT_BODY);
    label.set_padding(16, 1, 16, 1);
    label.set_auto_size(true);
    panel.add(&label);
}

fn add_blockquote(panel: &anyui::StackPanel, text: &str, depth: usize) {
    let stripped = strip_inline(text);
    // Build bar prefix: one │ per nesting level
    let mut prefix = String::from("  ");
    for _ in 0..depth {
        prefix.push('\u{2502}');
        prefix.push(' ');
    }
    let prefixed = anyos_std::format!("{}{}", prefix, stripped);
    let wrapped = word_wrap(&prefixed, WRAP_WIDTH - 4 * depth);

    let container = anyui::View::new();
    container.set_color(BG_QUOTE);
    let left_margin = 16 + (depth.saturating_sub(1) * 8) as i32;
    container.set_margin(left_margin, 2, 16, 2);
    container.set_padding(8, 4, 8, 4);
    container.set_auto_size(true);

    let label = anyui::Label::new(&wrapped);
    label.set_font(FONT_REGULAR);
    label.set_font_size(14);
    label.set_text_color(TEXT_QUOTE);
    label.set_auto_size(true);
    container.add(&label);

    panel.add(&container);
}

fn add_horizontal_rule(panel: &anyui::StackPanel) {
    add_spacing(panel, 6);
    let div = anyui::Divider::new();
    div.set_size(800, 1);
    div.set_color(0xFF505050);
    div.set_margin(16, 0, 16, 0);
    panel.add(&div);
    add_spacing(panel, 6);
}

fn is_table_separator(line: &str) -> bool {
    let trimmed = line.trim();
    if !trimmed.starts_with('|') { return false; }
    // Check if all cells are separator cells like ---, :---, ---:, :---:
    for cell in trimmed.split('|') {
        let c = cell.trim();
        if c.is_empty() { continue; }
        let stripped = c.trim_start_matches(':').trim_end_matches(':');
        if stripped.is_empty() || !stripped.as_bytes().iter().all(|&b| b == b'-') {
            return false;
        }
    }
    true
}

fn parse_table_row(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    // Strip leading/trailing |
    let inner = if trimmed.starts_with('|') && trimmed.ends_with('|') {
        &trimmed[1..trimmed.len()-1]
    } else if trimmed.starts_with('|') {
        &trimmed[1..]
    } else {
        trimmed
    };
    inner.split('|').map(|cell| String::from(strip_inline(cell.trim()))).collect()
}

fn add_table(panel: &anyui::StackPanel, header: &[String], rows: &[Vec<String>]) {
    let num_cols = header.len();
    if num_cols == 0 { return; }

    // Calculate column widths (char-based, minimum 6)
    let mut col_widths: Vec<usize> = header.iter().map(|h| h.len().max(6)).collect();
    for row in rows {
        for (c, cell) in row.iter().enumerate() {
            if c < col_widths.len() {
                col_widths[c] = col_widths[c].max(cell.len());
            }
        }
    }

    // Build a text-based table
    let mut table_text = String::new();

    // Header row
    for (c, h) in header.iter().enumerate() {
        if c > 0 { table_text.push_str(" \u{2502} "); }
        table_text.push_str(h);
        let pad = col_widths.get(c).copied().unwrap_or(0).saturating_sub(h.len());
        for _ in 0..pad { table_text.push(' '); }
    }
    table_text.push('\n');

    // Separator
    for (c, w) in col_widths.iter().enumerate() {
        if c > 0 { table_text.push_str("\u{2500}\u{253C}\u{2500}"); }
        for _ in 0..*w { table_text.push('\u{2500}'); }
    }
    table_text.push('\n');

    // Data rows
    for row in rows {
        for c in 0..num_cols {
            if c > 0 { table_text.push_str(" \u{2502} "); }
            let cell = row.get(c).map(|s| s.as_str()).unwrap_or("");
            table_text.push_str(cell);
            let pad = col_widths.get(c).copied().unwrap_or(0).saturating_sub(cell.len());
            for _ in 0..pad { table_text.push(' '); }
        }
        table_text.push('\n');
    }

    // Render as a code-like block
    let container = anyui::View::new();
    container.set_color(BG_CODE_BLOCK);
    container.set_margin(16, 4, 16, 4);
    container.set_padding(12, 8, 12, 8);
    container.set_auto_size(true);

    let label = anyui::Label::new(table_text.trim_end());
    label.set_font(FONT_MONO);
    label.set_font_size(13);
    label.set_text_color(TEXT_CODE);
    label.set_auto_size(true);
    container.add(&label);

    panel.add(&container);
}

fn is_hr_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.len() < 3 { return false; }
    let bytes = trimmed.as_bytes();
    let ch = bytes[0];
    if ch != b'-' && ch != b'*' && ch != b'_' { return false; }
    bytes.iter().all(|&b| b == ch || b == b' ')
}

fn heading_level(line: &str) -> Option<(usize, &str)> {
    let bytes = line.as_bytes();
    let mut level = 0;
    while level < bytes.len() && bytes[level] == b'#' {
        level += 1;
    }
    if level >= 1 && level <= 6 && level < bytes.len() && bytes[level] == b' ' {
        Some((level, &line[level + 1..]))
    } else {
        None
    }
}

/// Returns (ordered, text, indent_level) where indent_level is number of leading spaces / 2.
fn list_item_text(line: &str) -> Option<(bool, &str, usize)> {
    let indent = line.len() - line.trim_start().len();
    let indent_level = indent / 2;
    let trimmed = line.trim_start();
    // Unordered: - item, * item, + item
    if trimmed.len() >= 2 && (trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ")) {
        return Some((false, &trimmed[2..], indent_level));
    }
    // Ordered: 1. item, 12. item
    let bytes = trimmed.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i] >= b'0' && bytes[i] <= b'9' {
        i += 1;
    }
    if i > 0 && i + 1 < bytes.len() && bytes[i] == b'.' && bytes[i + 1] == b' ' {
        return Some((true, &trimmed[i + 2..], indent_level));
    }
    None
}

/// Returns (text, nesting_depth) for blockquote lines.
fn blockquote_text(line: &str) -> Option<(&str, usize)> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('>') {
        return None;
    }
    let mut depth = 0usize;
    let bytes = trimmed.as_bytes();
    let mut pos = 0;
    while pos < bytes.len() && bytes[pos] == b'>' {
        depth += 1;
        pos += 1;
        // Skip optional space after >
        if pos < bytes.len() && bytes[pos] == b' ' {
            pos += 1;
        }
    }
    Some((&trimmed[pos..], depth))
}

fn render_markdown(md: &str, panel: &anyui::StackPanel) {
    let lines: Vec<&str> = md.split('\n').collect();
    let total = lines.len();
    let mut i = 0;
    let mut para_buf = String::new();
    let mut ordered_counter = 0usize;

    while i < total {
        let line = lines[i];

        // Fenced code block
        if line.trim_start().starts_with("```") {
            // Flush paragraph
            if !para_buf.is_empty() {
                add_paragraph(panel, &para_buf);
                para_buf.clear();
            }
            i += 1;
            let mut code_lines: Vec<&str> = Vec::new();
            while i < total && !lines[i].trim_start().starts_with("```") {
                code_lines.push(lines[i]);
                i += 1;
            }
            if i < total { i += 1; } // skip closing ```
            add_code_block(panel, &code_lines);
            continue;
        }

        // Indented code block (4 spaces or 1 tab)
        if (line.starts_with("    ") || line.starts_with("\t")) && para_buf.is_empty() {
            let mut code_lines: Vec<&str> = Vec::new();
            while i < total {
                let cl = lines[i];
                if cl.starts_with("    ") {
                    code_lines.push(&cl[4..]);
                    i += 1;
                } else if cl.starts_with("\t") {
                    code_lines.push(&cl[1..]);
                    i += 1;
                } else if cl.trim().is_empty() {
                    // Blank line inside indented block — include only if more indented lines follow
                    let mut has_more = false;
                    let mut j = i + 1;
                    while j < total {
                        if lines[j].starts_with("    ") || lines[j].starts_with("\t") {
                            has_more = true;
                            break;
                        }
                        if !lines[j].trim().is_empty() { break; }
                        j += 1;
                    }
                    if has_more {
                        code_lines.push("");
                        i += 1;
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }
            add_code_block(panel, &code_lines);
            continue;
        }

        // Empty line → paragraph break
        if line.trim().is_empty() {
            if !para_buf.is_empty() {
                add_paragraph(panel, &para_buf);
                para_buf.clear();
            }
            ordered_counter = 0;
            add_spacing(panel, 4);
            i += 1;
            continue;
        }

        // Heading
        if let Some((level, text)) = heading_level(line) {
            if !para_buf.is_empty() {
                add_paragraph(panel, &para_buf);
                para_buf.clear();
            }
            add_heading(panel, text, level);
            i += 1;
            continue;
        }

        // Horizontal rule
        if is_hr_line(line) {
            if !para_buf.is_empty() {
                add_paragraph(panel, &para_buf);
                para_buf.clear();
            }
            add_horizontal_rule(panel);
            i += 1;
            continue;
        }

        // Blockquote
        if let Some((text, depth)) = blockquote_text(line) {
            if !para_buf.is_empty() {
                add_paragraph(panel, &para_buf);
                para_buf.clear();
            }
            add_blockquote(panel, text, depth);
            i += 1;
            continue;
        }

        // List item
        if let Some((ordered, text, indent_level)) = list_item_text(line) {
            if !para_buf.is_empty() {
                add_paragraph(panel, &para_buf);
                para_buf.clear();
            }
            if ordered {
                ordered_counter += 1;
            }
            add_list_item(panel, text, ordered, ordered_counter, indent_level);
            i += 1;
            continue;
        }

        // Table: line with | and next line is separator
        if line.contains('|') && i + 1 < total && is_table_separator(lines[i + 1]) {
            if !para_buf.is_empty() {
                add_paragraph(panel, &para_buf);
                para_buf.clear();
            }
            let header = parse_table_row(line);
            i += 2; // skip header + separator
            let mut rows: Vec<Vec<String>> = Vec::new();
            while i < total && lines[i].contains('|') && !lines[i].trim().is_empty() {
                rows.push(parse_table_row(lines[i]));
                i += 1;
            }
            add_table(panel, &header, &rows);
            continue;
        }

        // Regular paragraph text — accumulate
        if !para_buf.is_empty() {
            para_buf.push(' ');
        }
        para_buf.push_str(line);
        i += 1;
    }

    // Flush remaining paragraph
    if !para_buf.is_empty() {
        add_paragraph(panel, &para_buf);
    }

    // Bottom spacing
    add_spacing(panel, 20);
}

// ── File operations ──────────────────────────────────────────────────────────

fn open_file(path: &str) {
    let s = app();

    // Check if already open → switch to it
    for (i, f) in s.files.iter().enumerate() {
        if f.path == path {
            switch_tab(i);
            return;
        }
    }

    // Read file
    let content = match anyos_std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => {
            anyos_std::println!("mdview: failed to read {}", path);
            return;
        }
    };

    // Hide current active panel and source editor
    if !s.files.is_empty() && s.active < s.files.len() {
        s.files[s.active].panel.set_visible(false);
        if let Some(ref editor) = s.files[s.active].source_editor {
            editor.set_visible(false);
        }
    }

    // Create content panel
    let panel = anyui::StackPanel::vertical();
    panel.set_dock(anyui::DOCK_TOP);
    panel.set_auto_size(true);
    panel.set_padding(0, 8, 0, 8);

    // Render markdown into the panel
    render_markdown(&content, &panel);

    // Add panel to scroll view
    s.scroll.add(&panel);

    // Track the file
    let idx = s.files.len();
    s.files.push(OpenFile {
        path: String::from(path),
        content,
        panel,
        source_editor: None,
        showing_source: false,
        modified: false,
    });
    s.active = idx;

    // Update UI
    let labels = tab_labels(&s.files);
    s.tab_bar.set_text(&labels);
    s.tab_bar.set_state(idx as u32);
    s.tab_bar.set_visible(true);

    update_status();
}

fn close_tab(index: usize) {
    let s = app();
    if index >= s.files.len() { return; }

    // Remove panel and source editor from scroll view
    s.files[index].panel.remove();
    if let Some(ref editor) = s.files[index].source_editor {
        editor.remove();
    }
    s.files.remove(index);

    if s.files.is_empty() {
        s.active = 0;
        s.tab_bar.set_visible(false);
        s.path_label.set_text("No file open");
        s.status_label.set_text("Ready");
        return;
    }

    // Adjust active index
    if s.active >= s.files.len() {
        s.active = s.files.len() - 1;
    }

    // Show the new active panel
    for (i, f) in s.files.iter().enumerate() {
        f.panel.set_visible(i == s.active);
    }

    let labels = tab_labels(&s.files);
    s.tab_bar.set_text(&labels);
    s.tab_bar.set_state(s.active as u32);
    update_status();
}

fn switch_tab(index: usize) {
    let s = app();
    if index >= s.files.len() { return; }

    // Hide all panels and source editors, show target
    for (i, f) in s.files.iter().enumerate() {
        if i == index {
            if f.showing_source {
                f.panel.set_visible(false);
                if let Some(ref editor) = f.source_editor {
                    editor.set_visible(true);
                }
            } else {
                f.panel.set_visible(true);
                if let Some(ref editor) = f.source_editor {
                    editor.set_visible(false);
                }
            }
        } else {
            f.panel.set_visible(false);
            if let Some(ref editor) = f.source_editor {
                editor.set_visible(false);
            }
        }
    }
    s.active = index;
    s.tab_bar.set_state(index as u32);
    update_status();
}

/// Re-render the markdown panel for the active file from its `content`.
fn rerender_active() {
    let s = app();
    if s.files.is_empty() { return; }
    let idx = s.active;

    // Remove old rendered panel
    s.files[idx].panel.remove();

    // Create new rendered panel
    let panel = anyui::StackPanel::vertical();
    panel.set_dock(anyui::DOCK_TOP);
    panel.set_auto_size(true);
    panel.set_padding(0, 8, 0, 8);
    render_markdown(&s.files[idx].content, &panel);
    s.scroll.add(&panel);

    s.files[idx].panel = panel;
}

/// Read editor text back into `content`, mark modified if changed.
fn sync_editor_to_content() {
    let s = app();
    if s.files.is_empty() { return; }
    let idx = s.active;
    let file = &mut s.files[idx];
    if let Some(ref editor) = file.source_editor {
        let mut buf = [0u8; 256 * 1024]; // 256 KB buffer
        let len = editor.get_text(&mut buf) as usize;
        let new_content = core::str::from_utf8(&buf[..len]).unwrap_or("");
        if new_content != file.content.as_str() {
            file.content = String::from(new_content);
            file.modified = true;
        }
    }
}

fn toggle_source() {
    let s = app();
    if s.files.is_empty() { return; }
    let idx = s.active;

    if s.files[idx].showing_source {
        // Sync editor content back and re-render
        sync_editor_to_content();
        if let Some(ref editor) = s.files[idx].source_editor {
            editor.set_visible(false);
        }
        // Re-render markdown from (possibly updated) content
        rerender_active();
        s.files[idx].panel.set_visible(true);
        s.files[idx].showing_source = false;
        update_tab_labels();
    } else {
        // Switch to source view
        s.files[idx].panel.set_visible(false);

        if s.files[idx].source_editor.is_none() {
            // Create source editor on first use
            let editor = anyui::TextEditor::new(900, 600);
            editor.set_dock(anyui::DOCK_FILL);
            editor.set_text(&s.files[idx].content);
            editor.set_show_line_numbers(true);
            editor.set_editor_font(FONT_MONO, 13);
            s.scroll.add(&editor);
            s.files[idx].source_editor = Some(editor);
        } else {
            s.files[idx].source_editor.as_ref().unwrap().set_visible(true);
        }
        s.files[idx].showing_source = true;
    }
}

fn update_tab_labels() {
    let s = app();
    let labels = tab_labels(&s.files);
    s.tab_bar.set_text(&labels);
}

fn save_file() {
    let s = app();
    if s.files.is_empty() { return; }
    let idx = s.active;

    // If source editor is visible, sync content first
    if s.files[idx].showing_source {
        sync_editor_to_content();
    }

    let path = s.files[idx].path.clone();
    let content = s.files[idx].content.as_bytes();
    if anyos_std::fs::write_bytes(&path, content).is_ok() {
        s.files[idx].modified = false;
        update_tab_labels();
        update_status();
    }
}

fn save_file_as() {
    let s = app();
    if s.files.is_empty() { return; }
    let idx = s.active;

    // If source editor is visible, sync content first
    if s.files[idx].showing_source {
        sync_editor_to_content();
    }

    let default_name = basename(&s.files[idx].path);
    if let Some(new_path) = anyui::FileDialog::save_file(default_name) {
        let content = s.files[idx].content.as_bytes();
        if anyos_std::fs::write_bytes(&new_path, content).is_ok() {
            s.files[idx].path = new_path;
            s.files[idx].modified = false;
            update_tab_labels();
            update_status();
        }
    }
}

// ── Clipboard operations ─────────────────────────────────────────────────────

/// Copy: in source mode → editor.copy(). In rendered mode → copy full markdown content.
fn clipboard_copy() {
    let s = app();
    if s.files.is_empty() { return; }
    let file = &s.files[s.active];
    if file.showing_source {
        if let Some(ref editor) = file.source_editor {
            if editor.copy() {
                s.status_label.set_text("Copied selection");
            }
        }
    } else {
        // Rendered view: copy entire markdown content
        anyui::clipboard_set(&file.content);
        s.status_label.set_text("Copied markdown content");
    }
}

/// Cut: only works in source mode.
fn clipboard_cut() {
    let s = app();
    if s.files.is_empty() { return; }
    let file = &mut s.files[s.active];
    if file.showing_source {
        if let Some(ref editor) = file.source_editor {
            if editor.cut() {
                file.modified = true;
                update_tab_labels();
                s.status_label.set_text("Cut selection");
            }
        }
    }
}

/// Paste: only works in source mode.
fn clipboard_paste() {
    let s = app();
    if s.files.is_empty() { return; }
    let file = &mut s.files[s.active];
    if file.showing_source {
        if let Some(ref editor) = file.source_editor {
            let n = editor.paste();
            if n > 0 {
                file.modified = true;
                update_tab_labels();
                s.status_label.set_text("Pasted from clipboard");
            }
        }
    }
}

/// Select all: in source mode → editor.select_all(). In rendered mode → copy all.
fn select_all() {
    let s = app();
    if s.files.is_empty() { return; }
    let file = &s.files[s.active];
    if file.showing_source {
        if let Some(ref editor) = file.source_editor {
            editor.select_all();
        }
    } else {
        // Rendered view: copy all content to clipboard as convenience
        anyui::clipboard_set(&file.content);
        s.status_label.set_text("All content copied to clipboard");
    }
}

fn update_status() {
    let s = app();
    if s.files.is_empty() {
        s.path_label.set_text("No file open");
        s.status_label.set_text("Ready");
        return;
    }

    let file = &s.files[s.active];
    let name = basename(&file.path);
    s.path_label.set_text(&file.path);

    let status = anyos_std::format!("{} | {} file(s) open", name, s.files.len());
    s.status_label.set_text(&status);
}

// ── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    if !anyui::init() {
        anyos_std::println!("mdview: failed to load libanyui.so");
        return;
    }

    // Parse command line argument
    let mut args_buf = [0u8; 256];
    let arg_path = anyos_std::process::args(&mut args_buf).trim();

    // Create window
    let win = anyui::Window::new("Markdown Viewer", -1, -1, 900, 600);

    // ── Toolbar ──
    let toolbar = anyui::Toolbar::new();
    toolbar.set_dock(anyui::DOCK_TOP);
    toolbar.set_size(900, 36);
    toolbar.set_color(COLOR_TOOLBAR);
    toolbar.set_padding(4, 4, 4, 4);

    let btn_open = toolbar.add_icon_button("Open");
    btn_open.set_size(60, 28);
    btn_open.set_icon(anyui::ICON_FOLDER_OPEN);

    let btn_source = toolbar.add_icon_button("Source");
    btn_source.set_size(70, 28);
    btn_source.set_icon(anyui::ICON_FILES);

    let btn_save = toolbar.add_icon_button("Save");
    btn_save.set_size(60, 28);
    btn_save.set_icon(anyui::ICON_SAVE);

    let btn_save_as = toolbar.add_icon_button("Save As");
    btn_save_as.set_size(70, 28);
    btn_save_as.set_icon(anyui::ICON_SAVE_ALL);

    toolbar.add_separator();

    let path_label = toolbar.add_label("No file open");
    path_label.set_text_color(TEXT_STATUS);

    win.add(&toolbar);

    // ── TabBar ──
    let tab_bar = anyui::TabBar::new("");
    tab_bar.set_dock(anyui::DOCK_TOP);
    tab_bar.set_size(900, 28);
    tab_bar.set_color(COLOR_TAB_BAR);
    tab_bar.set_visible(false);
    win.add(&tab_bar);

    // ── Status bar ──
    let status_bar = anyui::View::new();
    status_bar.set_dock(anyui::DOCK_BOTTOM);
    status_bar.set_size(900, 24);
    status_bar.set_color(COLOR_STATUS);

    let status_label = anyui::Label::new("Ready");
    status_label.set_position(8, 4);
    status_label.set_text_color(TEXT_STATUS);
    status_label.set_font_size(12);
    status_bar.add(&status_label);

    win.add(&status_bar);

    // ── Scroll view (DOCK_FILL) ──
    let scroll = anyui::ScrollView::new();
    scroll.set_dock(anyui::DOCK_FILL);
    scroll.set_color(COLOR_BG);
    win.add(&scroll);

    // ── Initialize global state ──
    unsafe {
        APP = Some(AppState {
            win,
            tab_bar,
            scroll,
            status_label,
            path_label,
            files: Vec::new(),
            active: 0,
        });
    }

    // ── Wire events ──
    btn_open.on_click(|_| {
        if let Some(path) = anyui::FileDialog::open_file() {
            open_file(&path);
        }
    });

    btn_source.on_click(|_| {
        toggle_source();
    });

    btn_save.on_click(|_| {
        save_file();
    });

    btn_save_as.on_click(|_| {
        save_file_as();
    });

    app().tab_bar.on_active_changed(|e| {
        switch_tab(e.index as usize);
    });

    app().tab_bar.on_tab_close(|e| {
        close_tab(e.index as usize);
    });

    app().win.on_close(|_| { anyui::quit(); });

    // ── Keyboard shortcuts ──
    // Note: Ctrl+C/V/X/A are handled by TextEditor when it has focus in source mode.
    // The window on_key_down only receives keys NOT consumed by the focused control,
    // so these fire in rendered view mode or when the editor doesn't consume them.
    app().win.on_key_down(|ke| {
        if ke.ctrl() && ke.shift() {
            match ke.char_code {
                0x53 | 0x73 => save_file_as(),          // Ctrl+Shift+S
                _ => {}
            }
        } else if ke.ctrl() {
            match ke.char_code {
                0x63 => clipboard_copy(),                // Ctrl+C (rendered view)
                0x78 => clipboard_cut(),                 // Ctrl+X (rendered view)
                0x76 => clipboard_paste(),               // Ctrl+V (rendered view)
                0x61 => select_all(),                    // Ctrl+A (rendered view)
                0x73 => save_file(),                     // Ctrl+S
                0x6F => {                                // Ctrl+O
                    if let Some(path) = anyui::FileDialog::open_file() {
                        open_file(&path);
                    }
                }
                0x65 => toggle_source(),                 // Ctrl+E (toggle editor)
                0x77 => {                                // Ctrl+W (close tab)
                    let idx = app().active;
                    close_tab(idx);
                }
                _ => {}
            }
        }
    });

    // ── Open file from command line ──
    if !arg_path.is_empty() {
        open_file(arg_path);
    }

    // ── Run event loop ──
    anyui::run();
}
