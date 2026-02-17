//! German QWERTZ keyboard layout (de-DE).

use super::KeyboardLayout;

pub static LAYOUT_DE: KeyboardLayout = KeyboardLayout {
    normal: {
        let mut m = ['\0'; 128];
        m[0x02] = '1'; m[0x03] = '2'; m[0x04] = '3'; m[0x05] = '4';
        m[0x06] = '5'; m[0x07] = '6'; m[0x08] = '7'; m[0x09] = '8';
        m[0x0A] = '9'; m[0x0B] = '0';
        m[0x0C] = '\u{00DF}'; // ß
        m[0x0D] = '\u{00B4}'; // ´ (acute accent)
        m[0x10] = 'q'; m[0x11] = 'w'; m[0x12] = 'e'; m[0x13] = 'r';
        m[0x14] = 't'; m[0x15] = 'z'; // QWERTZ: Z/Y swapped
        m[0x16] = 'u'; m[0x17] = 'i';
        m[0x18] = 'o'; m[0x19] = 'p';
        m[0x1A] = '\u{00FC}'; // ü
        m[0x1B] = '+';
        m[0x1E] = 'a'; m[0x1F] = 's'; m[0x20] = 'd'; m[0x21] = 'f';
        m[0x22] = 'g'; m[0x23] = 'h'; m[0x24] = 'j'; m[0x25] = 'k';
        m[0x26] = 'l';
        m[0x27] = '\u{00F6}'; // ö
        m[0x28] = '\u{00E4}'; // ä
        m[0x29] = '^';         // circumflex (dead key position)
        m[0x2B] = '#';
        m[0x2C] = 'y'; // QWERTZ: Z/Y swapped
        m[0x2D] = 'x'; m[0x2E] = 'c'; m[0x2F] = 'v';
        m[0x30] = 'b'; m[0x31] = 'n'; m[0x32] = 'm'; m[0x33] = ',';
        m[0x34] = '.'; m[0x35] = '-';
        m[0x56] = '<'; // ISO key (between left shift and Y)
        m
    },
    shift: {
        let mut m = ['\0'; 128];
        m[0x02] = '!'; m[0x03] = '"';
        m[0x04] = '\u{00A7}'; // §
        m[0x05] = '$'; m[0x06] = '%'; m[0x07] = '&'; m[0x08] = '/';
        m[0x09] = '('; m[0x0A] = ')'; m[0x0B] = '=';
        m[0x0C] = '?'; // Shift+ß
        m[0x0D] = '`'; // Shift+´
        m[0x10] = 'Q'; m[0x11] = 'W'; m[0x12] = 'E'; m[0x13] = 'R';
        m[0x14] = 'T'; m[0x15] = 'Z'; m[0x16] = 'U'; m[0x17] = 'I';
        m[0x18] = 'O'; m[0x19] = 'P';
        m[0x1A] = '\u{00DC}'; // Ü
        m[0x1B] = '*';
        m[0x1E] = 'A'; m[0x1F] = 'S'; m[0x20] = 'D'; m[0x21] = 'F';
        m[0x22] = 'G'; m[0x23] = 'H'; m[0x24] = 'J'; m[0x25] = 'K';
        m[0x26] = 'L';
        m[0x27] = '\u{00D6}'; // Ö
        m[0x28] = '\u{00C4}'; // Ä
        m[0x29] = '\u{00B0}'; // ° (degree sign)
        m[0x2B] = '\'';       // Shift+#
        m[0x2C] = 'Y'; m[0x2D] = 'X'; m[0x2E] = 'C'; m[0x2F] = 'V';
        m[0x30] = 'B'; m[0x31] = 'N'; m[0x32] = 'M'; m[0x33] = ';';
        m[0x34] = ':'; m[0x35] = '_';
        m[0x56] = '>'; // Shift+<
        m
    },
    altgr: {
        let mut m = ['\0'; 128];
        m[0x03] = '\u{00B2}'; // AltGr+2 = ²
        m[0x04] = '\u{00B3}'; // AltGr+3 = ³
        m[0x08] = '{';         // AltGr+7
        m[0x09] = '[';         // AltGr+8
        m[0x0A] = ']';         // AltGr+9
        m[0x0B] = '}';         // AltGr+0
        m[0x0C] = '\\';        // AltGr+ß
        m[0x10] = '@';         // AltGr+Q
        m[0x12] = '\u{20AC}';  // AltGr+E = €
        m[0x1B] = '~';         // AltGr++
        m[0x32] = '\u{00B5}';  // AltGr+M = µ
        m[0x56] = '|';         // AltGr+<
        m
    },
    shift_altgr: ['\0'; 128],
};
