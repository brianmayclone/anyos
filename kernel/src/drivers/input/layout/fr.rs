//! French AZERTY keyboard layout (fr-FR).
//!
//! AZERTY swaps A↔Q and W↔Z compared to QWERTY.
//! Numbers are on the Shift layer; unshifted number row has symbols/accents.

use super::KeyboardLayout;

pub static LAYOUT_FR: KeyboardLayout = KeyboardLayout {
    normal: {
        let mut m = ['\0'; 128];
        m[0x02] = '&';                // 1 key
        m[0x03] = '\u{00E9}';         // 2 key = é
        m[0x04] = '"';                 // 3 key
        m[0x05] = '\'';               // 4 key
        m[0x06] = '(';                 // 5 key
        m[0x07] = '-';                 // 6 key
        m[0x08] = '\u{00E8}';         // 7 key = è
        m[0x09] = '_';                 // 8 key
        m[0x0A] = '\u{00E7}';         // 9 key = ç
        m[0x0B] = '\u{00E0}';         // 0 key = à
        m[0x0C] = ')';                 // key right of 0
        m[0x0D] = '=';
        m[0x10] = 'a'; // AZERTY: A on Q key
        m[0x11] = 'z'; // AZERTY: Z on W key
        m[0x12] = 'e'; m[0x13] = 'r'; m[0x14] = 't'; m[0x15] = 'y';
        m[0x16] = 'u'; m[0x17] = 'i'; m[0x18] = 'o'; m[0x19] = 'p';
        m[0x1A] = '^';                // circumflex (dead key)
        m[0x1B] = '$';
        m[0x1E] = 'q'; // AZERTY: Q on A key
        m[0x1F] = 's'; m[0x20] = 'd'; m[0x21] = 'f';
        m[0x22] = 'g'; m[0x23] = 'h'; m[0x24] = 'j'; m[0x25] = 'k';
        m[0x26] = 'l'; m[0x27] = 'm';
        m[0x28] = '\u{00F9}';         // ù
        m[0x29] = '\u{00B2}';         // ² (key left of 1)
        m[0x2B] = '*';
        m[0x2C] = 'w'; // AZERTY: W on Z key
        m[0x2D] = 'x'; m[0x2E] = 'c'; m[0x2F] = 'v';
        m[0x30] = 'b'; m[0x31] = 'n';
        m[0x32] = ','; // AZERTY: comma on M key position
        m[0x33] = ';';
        m[0x34] = ':'; m[0x35] = '!';
        m[0x56] = '<'; // ISO key
        m
    },
    shift: {
        let mut m = ['\0'; 128];
        m[0x02] = '1'; m[0x03] = '2'; m[0x04] = '3'; m[0x05] = '4';
        m[0x06] = '5'; m[0x07] = '6'; m[0x08] = '7'; m[0x09] = '8';
        m[0x0A] = '9'; m[0x0B] = '0';
        m[0x0C] = '\u{00B0}'; // Shift+) = °
        m[0x0D] = '+';
        m[0x10] = 'A'; m[0x11] = 'Z'; m[0x12] = 'E'; m[0x13] = 'R';
        m[0x14] = 'T'; m[0x15] = 'Y'; m[0x16] = 'U'; m[0x17] = 'I';
        m[0x18] = 'O'; m[0x19] = 'P';
        m[0x1A] = '\u{00A8}'; // Shift+^ = ¨ (diaeresis)
        m[0x1B] = '\u{00A3}'; // Shift+$ = £
        m[0x1E] = 'Q'; m[0x1F] = 'S'; m[0x20] = 'D'; m[0x21] = 'F';
        m[0x22] = 'G'; m[0x23] = 'H'; m[0x24] = 'J'; m[0x25] = 'K';
        m[0x26] = 'L'; m[0x27] = 'M';
        m[0x28] = '%';
        m[0x2B] = '\u{00B5}'; // Shift+* = µ
        m[0x2C] = 'W'; m[0x2D] = 'X'; m[0x2E] = 'C'; m[0x2F] = 'V';
        m[0x30] = 'B'; m[0x31] = 'N';
        m[0x32] = '?'; // Shift+,
        m[0x33] = '.';
        m[0x34] = '/'; m[0x35] = '\u{00A7}'; // §
        m[0x56] = '>'; // Shift+<
        m
    },
    altgr: {
        let mut m = ['\0'; 128];
        m[0x03] = '~';         // AltGr+é = ~
        m[0x04] = '#';         // AltGr+3
        m[0x05] = '{';         // AltGr+4
        m[0x06] = '[';         // AltGr+5
        m[0x07] = '|';         // AltGr+6
        m[0x08] = '`';         // AltGr+7
        m[0x09] = '\\';        // AltGr+8
        m[0x0A] = '^';         // AltGr+9
        m[0x0B] = '@';         // AltGr+0
        m[0x0C] = ']';         // AltGr+)
        m[0x0D] = '}';         // AltGr+=
        m[0x12] = '\u{20AC}';  // AltGr+E = €
        m[0x56] = '|';         // AltGr+<
        m
    },
    shift_altgr: ['\0'; 128],
};
