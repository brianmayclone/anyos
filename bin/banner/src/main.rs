#![no_std]
#![no_main]

anyos_std::entry!(main);

/// Each character is 7 rows tall, stored as 7 byte-strings.
/// Characters are 5 columns wide, padded with spaces.
struct Glyph {
    rows: [&'static [u8]; 7],
}

static FONT: [Glyph; 38] = [
    // A
    Glyph { rows: [
        b"  #  ",
        b" # # ",
        b"#   #",
        b"#####",
        b"#   #",
        b"#   #",
        b"#   #",
    ]},
    // B
    Glyph { rows: [
        b"#### ",
        b"#   #",
        b"#   #",
        b"#### ",
        b"#   #",
        b"#   #",
        b"#### ",
    ]},
    // C
    Glyph { rows: [
        b" ### ",
        b"#   #",
        b"#    ",
        b"#    ",
        b"#    ",
        b"#   #",
        b" ### ",
    ]},
    // D
    Glyph { rows: [
        b"#### ",
        b"#   #",
        b"#   #",
        b"#   #",
        b"#   #",
        b"#   #",
        b"#### ",
    ]},
    // E
    Glyph { rows: [
        b"#####",
        b"#    ",
        b"#    ",
        b"#### ",
        b"#    ",
        b"#    ",
        b"#####",
    ]},
    // F
    Glyph { rows: [
        b"#####",
        b"#    ",
        b"#    ",
        b"#### ",
        b"#    ",
        b"#    ",
        b"#    ",
    ]},
    // G
    Glyph { rows: [
        b" ### ",
        b"#   #",
        b"#    ",
        b"# ###",
        b"#   #",
        b"#   #",
        b" ### ",
    ]},
    // H
    Glyph { rows: [
        b"#   #",
        b"#   #",
        b"#   #",
        b"#####",
        b"#   #",
        b"#   #",
        b"#   #",
    ]},
    // I
    Glyph { rows: [
        b"#####",
        b"  #  ",
        b"  #  ",
        b"  #  ",
        b"  #  ",
        b"  #  ",
        b"#####",
    ]},
    // J
    Glyph { rows: [
        b"#####",
        b"    #",
        b"    #",
        b"    #",
        b"    #",
        b"#   #",
        b" ### ",
    ]},
    // K
    Glyph { rows: [
        b"#   #",
        b"#  # ",
        b"# #  ",
        b"##   ",
        b"# #  ",
        b"#  # ",
        b"#   #",
    ]},
    // L
    Glyph { rows: [
        b"#    ",
        b"#    ",
        b"#    ",
        b"#    ",
        b"#    ",
        b"#    ",
        b"#####",
    ]},
    // M
    Glyph { rows: [
        b"#   #",
        b"## ##",
        b"# # #",
        b"#   #",
        b"#   #",
        b"#   #",
        b"#   #",
    ]},
    // N
    Glyph { rows: [
        b"#   #",
        b"##  #",
        b"# # #",
        b"#  ##",
        b"#   #",
        b"#   #",
        b"#   #",
    ]},
    // O
    Glyph { rows: [
        b" ### ",
        b"#   #",
        b"#   #",
        b"#   #",
        b"#   #",
        b"#   #",
        b" ### ",
    ]},
    // P
    Glyph { rows: [
        b"#### ",
        b"#   #",
        b"#   #",
        b"#### ",
        b"#    ",
        b"#    ",
        b"#    ",
    ]},
    // Q
    Glyph { rows: [
        b" ### ",
        b"#   #",
        b"#   #",
        b"#   #",
        b"# # #",
        b"#  # ",
        b" ## #",
    ]},
    // R
    Glyph { rows: [
        b"#### ",
        b"#   #",
        b"#   #",
        b"#### ",
        b"# #  ",
        b"#  # ",
        b"#   #",
    ]},
    // S
    Glyph { rows: [
        b" ### ",
        b"#   #",
        b"#    ",
        b" ### ",
        b"    #",
        b"#   #",
        b" ### ",
    ]},
    // T
    Glyph { rows: [
        b"#####",
        b"  #  ",
        b"  #  ",
        b"  #  ",
        b"  #  ",
        b"  #  ",
        b"  #  ",
    ]},
    // U
    Glyph { rows: [
        b"#   #",
        b"#   #",
        b"#   #",
        b"#   #",
        b"#   #",
        b"#   #",
        b" ### ",
    ]},
    // V
    Glyph { rows: [
        b"#   #",
        b"#   #",
        b"#   #",
        b"#   #",
        b" # # ",
        b" # # ",
        b"  #  ",
    ]},
    // W
    Glyph { rows: [
        b"#   #",
        b"#   #",
        b"#   #",
        b"#   #",
        b"# # #",
        b"## ##",
        b"#   #",
    ]},
    // X
    Glyph { rows: [
        b"#   #",
        b"#   #",
        b" # # ",
        b"  #  ",
        b" # # ",
        b"#   #",
        b"#   #",
    ]},
    // Y
    Glyph { rows: [
        b"#   #",
        b"#   #",
        b" # # ",
        b"  #  ",
        b"  #  ",
        b"  #  ",
        b"  #  ",
    ]},
    // Z
    Glyph { rows: [
        b"#####",
        b"    #",
        b"   # ",
        b"  #  ",
        b" #   ",
        b"#    ",
        b"#####",
    ]},
    // 0
    Glyph { rows: [
        b" ### ",
        b"#   #",
        b"#  ##",
        b"# # #",
        b"##  #",
        b"#   #",
        b" ### ",
    ]},
    // 1
    Glyph { rows: [
        b"  #  ",
        b" ##  ",
        b"  #  ",
        b"  #  ",
        b"  #  ",
        b"  #  ",
        b"#####",
    ]},
    // 2
    Glyph { rows: [
        b" ### ",
        b"#   #",
        b"    #",
        b"  ## ",
        b" #   ",
        b"#    ",
        b"#####",
    ]},
    // 3
    Glyph { rows: [
        b" ### ",
        b"#   #",
        b"    #",
        b"  ## ",
        b"    #",
        b"#   #",
        b" ### ",
    ]},
    // 4
    Glyph { rows: [
        b"   # ",
        b"  ## ",
        b" # # ",
        b"#  # ",
        b"#####",
        b"   # ",
        b"   # ",
    ]},
    // 5
    Glyph { rows: [
        b"#####",
        b"#    ",
        b"#### ",
        b"    #",
        b"    #",
        b"#   #",
        b" ### ",
    ]},
    // 6
    Glyph { rows: [
        b" ### ",
        b"#    ",
        b"#    ",
        b"#### ",
        b"#   #",
        b"#   #",
        b" ### ",
    ]},
    // 7
    Glyph { rows: [
        b"#####",
        b"    #",
        b"   # ",
        b"  #  ",
        b"  #  ",
        b"  #  ",
        b"  #  ",
    ]},
    // 8
    Glyph { rows: [
        b" ### ",
        b"#   #",
        b"#   #",
        b" ### ",
        b"#   #",
        b"#   #",
        b" ### ",
    ]},
    // 9
    Glyph { rows: [
        b" ### ",
        b"#   #",
        b"#   #",
        b" ####",
        b"    #",
        b"    #",
        b" ### ",
    ]},
    // space
    Glyph { rows: [
        b"     ",
        b"     ",
        b"     ",
        b"     ",
        b"     ",
        b"     ",
        b"     ",
    ]},
    // ! (exclamation)
    Glyph { rows: [
        b"  #  ",
        b"  #  ",
        b"  #  ",
        b"  #  ",
        b"  #  ",
        b"     ",
        b"  #  ",
    ]},
];

fn glyph_for(ch: u8) -> &'static Glyph {
    match ch {
        b'A'..=b'Z' => &FONT[(ch - b'A') as usize],
        b'a'..=b'z' => &FONT[(ch - b'a') as usize],
        b'0'..=b'9' => &FONT[26 + (ch - b'0') as usize],
        b'!' => &FONT[37],
        _ => &FONT[36], // space
    }
}

/// Width of one rendered character: 5 glyph cols * 2 (doubled) + 1 gap = 11
const CHAR_WIDTH: u32 = 11;

fn render_line(text: &[u8], fill: u8) {
    for row in 0..7 {
        for &ch in text.iter() {
            let g = glyph_for(ch);
            for &pixel in g.rows[row] {
                if pixel == b'#' {
                    let buf = [fill, fill];
                    anyos_std::fs::write(1, &buf);
                } else {
                    anyos_std::fs::write(1, b"  ");
                }
            }
            anyos_std::fs::write(1, b" "); // gap between characters
        }
        anyos_std::println!("");
    }
}

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"wf");

    let width = args.opt_u32(b'w', 80);
    let fill = match args.opt(b'f') {
        Some(s) if !s.is_empty() => s.as_bytes()[0],
        _ => b'#',
    };

    if args.pos_count == 0 {
        anyos_std::println!("usage: banner [-w width] [-f char] text ...");
        anyos_std::println!("  -w width   output width in columns (default 80)");
        anyos_std::println!("  -f char    fill character (default #)");
        return;
    }

    // Join all positional args into a single text buffer (with spaces)
    let mut text_buf = [0u8; 256];
    let mut len: usize = 0;
    for i in 0..args.pos_count {
        if i > 0 && len < text_buf.len() {
            text_buf[len] = b' ';
            len += 1;
        }
        for &b in args.positional[i].as_bytes() {
            if len >= text_buf.len() { break; }
            text_buf[len] = b;
            len += 1;
        }
    }
    let text = &text_buf[..len];

    // Calculate how many characters fit per line
    let chars_per_line = if width / CHAR_WIDTH > 0 {
        (width / CHAR_WIDTH) as usize
    } else {
        1
    };

    // Render text in chunks that fit within the width
    let mut pos: usize = 0;
    while pos < text.len() {
        let end = if pos + chars_per_line > text.len() {
            text.len()
        } else {
            pos + chars_per_line
        };
        render_line(&text[pos..end], fill);
        pos = end;
        if pos < text.len() {
            anyos_std::println!(""); // blank line between wrapped rows
        }
    }
}
