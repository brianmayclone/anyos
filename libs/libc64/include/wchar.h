/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * wchar.h — Wide character / UTF-8 conversion support.
 * anyOS uses UTF-8 exclusively; wchar_t is 32-bit (Unicode code point).
 */

#ifndef _WCHAR_H
#define _WCHAR_H

#include <stddef.h>
#include <stdint.h>
#include <stdarg.h>

#ifndef __WCHAR_TYPE__
typedef int wchar_t;
#endif
typedef unsigned int wint_t;
typedef int mbstate_t;

#define WEOF ((wint_t)-1)
#define MB_CUR_MAX 4

#ifdef __cplusplus
extern "C" {
#endif

/* ── Multibyte ↔ wide conversion (UTF-8) ── */
size_t mbrtowc(wchar_t *pwc, const char *s, size_t n, mbstate_t *ps);
size_t wcrtomb(char *s, wchar_t wc, mbstate_t *ps);
size_t mbsrtowcs(wchar_t *dst, const char **src, size_t len, mbstate_t *ps);
size_t wcsrtombs(char *dst, const wchar_t **src, size_t len, mbstate_t *ps);
int mbtowc(wchar_t *pwc, const char *s, size_t n);
int wctomb(char *s, wchar_t wc);
size_t mbstowcs(wchar_t *dst, const char *src, size_t n);
size_t wcstombs(char *dst, const wchar_t *src, size_t n);
int mblen(const char *s, size_t n);

/* ── Wide string functions ── */
size_t wcslen(const wchar_t *s);
wchar_t *wcscpy(wchar_t *dst, const wchar_t *src);
wchar_t *wcsncpy(wchar_t *dst, const wchar_t *src, size_t n);
wchar_t *wcscat(wchar_t *dst, const wchar_t *src);
int wcscmp(const wchar_t *s1, const wchar_t *s2);
int wcsncmp(const wchar_t *s1, const wchar_t *s2, size_t n);
wchar_t *wcschr(const wchar_t *s, wchar_t c);
wchar_t *wcsrchr(const wchar_t *s, wchar_t c);
wchar_t *wmemset(wchar_t *s, wchar_t c, size_t n);
wchar_t *wmemcpy(wchar_t *dst, const wchar_t *src, size_t n);

/* ── Wide character classification ── */
int iswspace(wint_t wc);
int iswdigit(wint_t wc);
int iswalpha(wint_t wc);
int iswalnum(wint_t wc);
wint_t towlower(wint_t wc);
wint_t towupper(wint_t wc);

/* ── Wide I/O (simplified) ── */
int swprintf(wchar_t *s, size_t n, const wchar_t *fmt, ...);
int vswprintf(wchar_t *s, size_t n, const wchar_t *fmt, va_list ap);

#ifdef __cplusplus
}
#endif

#endif
