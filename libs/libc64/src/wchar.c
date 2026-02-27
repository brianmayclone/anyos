/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * libc64 — Wide character / UTF-8 conversion functions.
 * anyOS uses UTF-8 exclusively; wchar_t is 32-bit.
 */

#include <wchar.h>
#include <string.h>
#include <ctype.h>
#include <errno.h>

/* ── UTF-8 ↔ wchar_t conversion ── */

size_t mbrtowc(wchar_t *pwc, const char *s, size_t n, mbstate_t *ps) {
    (void)ps;
    if (!s) return 0;
    if (n == 0) return (size_t)-2;

    unsigned char c = (unsigned char)s[0];

    if (c < 0x80) {
        if (pwc) *pwc = (wchar_t)c;
        return c ? 1 : 0;
    }

    wchar_t wc;
    size_t len;

    if ((c & 0xE0) == 0xC0) {
        wc = c & 0x1F; len = 2;
    } else if ((c & 0xF0) == 0xE0) {
        wc = c & 0x0F; len = 3;
    } else if ((c & 0xF8) == 0xF0) {
        wc = c & 0x07; len = 4;
    } else {
        errno = EILSEQ;
        return (size_t)-1;
    }

    if (n < len) return (size_t)-2;

    for (size_t i = 1; i < len; i++) {
        unsigned char cont = (unsigned char)s[i];
        if ((cont & 0xC0) != 0x80) {
            errno = EILSEQ;
            return (size_t)-1;
        }
        wc = (wc << 6) | (cont & 0x3F);
    }

    if (pwc) *pwc = wc;
    return wc ? len : 0;
}

size_t wcrtomb(char *s, wchar_t wc, mbstate_t *ps) {
    (void)ps;
    if (!s) {
        static char dummy[4];
        return wcrtomb(dummy, L'\0', ps);
    }

    if (wc < 0x80) {
        s[0] = (char)wc;
        return 1;
    } else if (wc < 0x800) {
        s[0] = (char)(0xC0 | (wc >> 6));
        s[1] = (char)(0x80 | (wc & 0x3F));
        return 2;
    } else if (wc < 0x10000) {
        s[0] = (char)(0xE0 | (wc >> 12));
        s[1] = (char)(0x80 | ((wc >> 6) & 0x3F));
        s[2] = (char)(0x80 | (wc & 0x3F));
        return 3;
    } else if (wc < 0x110000) {
        s[0] = (char)(0xF0 | (wc >> 18));
        s[1] = (char)(0x80 | ((wc >> 12) & 0x3F));
        s[2] = (char)(0x80 | ((wc >> 6) & 0x3F));
        s[3] = (char)(0x80 | (wc & 0x3F));
        return 4;
    }

    errno = EILSEQ;
    return (size_t)-1;
}

size_t mbsrtowcs(wchar_t *dst, const char **src, size_t len, mbstate_t *ps) {
    size_t count = 0;
    while (count < len) {
        wchar_t wc;
        size_t r = mbrtowc(&wc, *src, 4, ps);
        if (r == (size_t)-1) return (size_t)-1;
        if (r == 0) { if (dst) dst[count] = L'\0'; *src = NULL; break; }
        if (dst) dst[count] = wc;
        *src += r;
        count++;
    }
    return count;
}

size_t wcsrtombs(char *dst, const wchar_t **src, size_t len, mbstate_t *ps) {
    size_t count = 0;
    char buf[4];
    while (1) {
        wchar_t wc = **src;
        size_t r = wcrtomb(buf, wc, ps);
        if (r == (size_t)-1) return (size_t)-1;
        if (count + r > len) break;
        if (dst) memcpy(dst + count, buf, r);
        count += r;
        if (wc == L'\0') { *src = NULL; break; }
        (*src)++;
    }
    return count;
}

int mbtowc(wchar_t *pwc, const char *s, size_t n) {
    if (!s) return 0;
    size_t r = mbrtowc(pwc, s, n, NULL);
    if (r == (size_t)-1 || r == (size_t)-2) return -1;
    return (int)r;
}

int wctomb(char *s, wchar_t wc) {
    if (!s) return 0;
    size_t r = wcrtomb(s, wc, NULL);
    return r == (size_t)-1 ? -1 : (int)r;
}

size_t mbstowcs(wchar_t *dst, const char *src, size_t n) {
    return mbsrtowcs(dst, &src, n, NULL);
}

size_t wcstombs(char *dst, const wchar_t *src, size_t n) {
    return wcsrtombs(dst, &src, n, NULL);
}

int mblen(const char *s, size_t n) {
    return mbtowc(NULL, s, n);
}

/* ── Wide string functions ── */

size_t wcslen(const wchar_t *s) {
    size_t n = 0;
    while (s[n]) n++;
    return n;
}

wchar_t *wcscpy(wchar_t *dst, const wchar_t *src) {
    wchar_t *d = dst;
    while ((*d++ = *src++) != L'\0');
    return dst;
}

wchar_t *wcsncpy(wchar_t *dst, const wchar_t *src, size_t n) {
    size_t i;
    for (i = 0; i < n && src[i]; i++) dst[i] = src[i];
    for (; i < n; i++) dst[i] = L'\0';
    return dst;
}

wchar_t *wcscat(wchar_t *dst, const wchar_t *src) {
    wchar_t *d = dst + wcslen(dst);
    while ((*d++ = *src++) != L'\0');
    return dst;
}

int wcscmp(const wchar_t *s1, const wchar_t *s2) {
    while (*s1 && *s1 == *s2) { s1++; s2++; }
    return (*s1 > *s2) - (*s1 < *s2);
}

int wcsncmp(const wchar_t *s1, const wchar_t *s2, size_t n) {
    for (size_t i = 0; i < n; i++) {
        if (s1[i] != s2[i] || !s1[i])
            return (s1[i] > s2[i]) - (s1[i] < s2[i]);
    }
    return 0;
}

wchar_t *wcschr(const wchar_t *s, wchar_t c) {
    while (*s) { if (*s == c) return (wchar_t *)s; s++; }
    return c == L'\0' ? (wchar_t *)s : NULL;
}

wchar_t *wcsrchr(const wchar_t *s, wchar_t c) {
    const wchar_t *last = NULL;
    while (*s) { if (*s == c) last = s; s++; }
    return (wchar_t *)(c == L'\0' ? s : last);
}

wchar_t *wmemset(wchar_t *s, wchar_t c, size_t n) {
    for (size_t i = 0; i < n; i++) s[i] = c;
    return s;
}

wchar_t *wmemcpy(wchar_t *dst, const wchar_t *src, size_t n) {
    for (size_t i = 0; i < n; i++) dst[i] = src[i];
    return dst;
}

/* ── Wide character classification (ASCII/Latin-1 subset) ── */

int iswspace(wint_t wc) { return wc == ' ' || (wc >= '\t' && wc <= '\r'); }
int iswdigit(wint_t wc) { return wc >= '0' && wc <= '9'; }
int iswalpha(wint_t wc) {
    return (wc >= 'A' && wc <= 'Z') || (wc >= 'a' && wc <= 'z');
}
int iswalnum(wint_t wc) { return iswalpha(wc) || iswdigit(wc); }

wint_t towlower(wint_t wc) {
    if (wc >= 'A' && wc <= 'Z') return wc + ('a' - 'A');
    return wc;
}

wint_t towupper(wint_t wc) {
    if (wc >= 'a' && wc <= 'z') return wc - ('a' - 'A');
    return wc;
}

/* ── swprintf — simplified wide printf (ASCII subset) ── */
int swprintf(wchar_t *s, size_t n, const wchar_t *fmt, ...) {
    (void)s; (void)n; (void)fmt;
    /* Stub — full implementation would mirror vsnprintf for wchar_t. */
    if (s && n > 0) s[0] = L'\0';
    return 0;
}

int vswprintf(wchar_t *s, size_t n, const wchar_t *fmt, va_list ap) {
    (void)s; (void)n; (void)fmt; (void)ap;
    if (s && n > 0) s[0] = L'\0';
    return 0;
}
