//! Polish Programmer's keyboard layout (pl-PL).
//!
//! Base layer is identical to US QWERTY.
//! Polish characters are available on the AltGr layer.

use super::KeyboardLayout;

pub static LAYOUT_PL: KeyboardLayout = KeyboardLayout {
    normal: {
        let mut m = ['\0'; 128];
        m[0x02] = '1'; m[0x03] = '2'; m[0x04] = '3'; m[0x05] = '4';
        m[0x06] = '5'; m[0x07] = '6'; m[0x08] = '7'; m[0x09] = '8';
        m[0x0A] = '9'; m[0x0B] = '0'; m[0x0C] = '-'; m[0x0D] = '=';
        m[0x10] = 'q'; m[0x11] = 'w'; m[0x12] = 'e'; m[0x13] = 'r';
        m[0x14] = 't'; m[0x15] = 'y'; m[0x16] = 'u'; m[0x17] = 'i';
        m[0x18] = 'o'; m[0x19] = 'p'; m[0x1A] = '['; m[0x1B] = ']';
        m[0x1E] = 'a'; m[0x1F] = 's'; m[0x20] = 'd'; m[0x21] = 'f';
        m[0x22] = 'g'; m[0x23] = 'h'; m[0x24] = 'j'; m[0x25] = 'k';
        m[0x26] = 'l'; m[0x27] = ';'; m[0x28] = '\'';
        m[0x29] = '`'; m[0x2B] = '\\';
        m[0x2C] = 'z'; m[0x2D] = 'x'; m[0x2E] = 'c'; m[0x2F] = 'v';
        m[0x30] = 'b'; m[0x31] = 'n'; m[0x32] = 'm'; m[0x33] = ',';
        m[0x34] = '.'; m[0x35] = '/';
        m
    },
    shift: {
        let mut m = ['\0'; 128];
        m[0x02] = '!'; m[0x03] = '@'; m[0x04] = '#'; m[0x05] = '$';
        m[0x06] = '%'; m[0x07] = '^'; m[0x08] = '&'; m[0x09] = '*';
        m[0x0A] = '('; m[0x0B] = ')'; m[0x0C] = '_'; m[0x0D] = '+';
        m[0x10] = 'Q'; m[0x11] = 'W'; m[0x12] = 'E'; m[0x13] = 'R';
        m[0x14] = 'T'; m[0x15] = 'Y'; m[0x16] = 'U'; m[0x17] = 'I';
        m[0x18] = 'O'; m[0x19] = 'P'; m[0x1A] = '{'; m[0x1B] = '}';
        m[0x1E] = 'A'; m[0x1F] = 'S'; m[0x20] = 'D'; m[0x21] = 'F';
        m[0x22] = 'G'; m[0x23] = 'H'; m[0x24] = 'J'; m[0x25] = 'K';
        m[0x26] = 'L'; m[0x27] = ':'; m[0x28] = '"';
        m[0x29] = '~'; m[0x2B] = '|';
        m[0x2C] = 'Z'; m[0x2D] = 'X'; m[0x2E] = 'C'; m[0x2F] = 'V';
        m[0x30] = 'B'; m[0x31] = 'N'; m[0x32] = 'M'; m[0x33] = '<';
        m[0x34] = '>'; m[0x35] = '?';
        m
    },
    altgr: {
        let mut m = ['\0'; 128];
        m[0x1E] = '\u{0105}'; // AltGr+A = ą (a-ogonek)
        m[0x2E] = '\u{0107}'; // AltGr+C = ć (c-acute)
        m[0x12] = '\u{0119}'; // AltGr+E = ę (e-ogonek)
        m[0x26] = '\u{0142}'; // AltGr+L = ł (l-stroke)
        m[0x31] = '\u{0144}'; // AltGr+N = ń (n-acute)
        m[0x18] = '\u{00F3}'; // AltGr+O = ó (o-acute)
        m[0x1F] = '\u{015B}'; // AltGr+S = ś (s-acute)
        m[0x2D] = '\u{017A}'; // AltGr+X = ź (z-acute)
        m[0x2C] = '\u{017C}'; // AltGr+Z = ż (z-dot-above)
        m
    },
    shift_altgr: {
        let mut m = ['\0'; 128];
        m[0x1E] = '\u{0104}'; // Shift+AltGr+A = Ą
        m[0x2E] = '\u{0106}'; // Shift+AltGr+C = Ć
        m[0x12] = '\u{0118}'; // Shift+AltGr+E = Ę
        m[0x26] = '\u{0141}'; // Shift+AltGr+L = Ł
        m[0x31] = '\u{0143}'; // Shift+AltGr+N = Ń
        m[0x18] = '\u{00D3}'; // Shift+AltGr+O = Ó
        m[0x1F] = '\u{015A}'; // Shift+AltGr+S = Ś
        m[0x2D] = '\u{0179}'; // Shift+AltGr+X = Ź
        m[0x2C] = '\u{017B}'; // Shift+AltGr+Z = Ż
        m
    },
};
