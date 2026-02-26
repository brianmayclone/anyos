/*
 * Copyright (c) 2024-2026 Christian Moeller
 * Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
 *
 * This project is open source and community-driven.
 * Contributions are welcome! See README.md for details.
 *
 * SPDX-License-Identifier: MIT
 */

#include <string.h>
#include <strings.h>
#include <stdlib.h>
#include <ctype.h>

void *memcpy(void *dest, const void *src, size_t n) {
    /* rep movsb — ERMS-accelerated on modern CPUs (Ivy Bridge+). */
    void *ret = dest;
    __asm__ __volatile__(
        "rep movsb"
        : "+D"(dest), "+S"(src), "+c"(n)
        :
        : "memory"
    );
    return ret;
}

void *memmove(void *dest, const void *src, size_t n) {
    void *ret = dest;
    if (dest <= src || (char *)dest >= (char *)src + n) {
        __asm__ __volatile__(
            "rep movsb"
            : "+D"(dest), "+S"(src), "+c"(n)
            :
            : "memory"
        );
    } else {
        dest = (char *)dest + n - 1;
        src  = (const char *)src + n - 1;
        __asm__ __volatile__(
            "std\n\t"
            "rep movsb\n\t"
            "cld"
            : "+D"(dest), "+S"(src), "+c"(n)
            :
            : "memory"
        );
    }
    return ret;
}

void *memset(void *s, int c, size_t n) {
    void *ret = s;
    /* Broadcast byte to dword: 0xAB → 0xABABABAB */
    unsigned int fill = (unsigned char)c;
    fill |= fill << 8;
    fill |= fill << 16;

    size_t dwords = n >> 2;
    size_t tail   = n & 3;
    __asm__ __volatile__(
        "rep stosl"
        : "+D"(s), "+c"(dwords)
        : "a"(fill)
        : "memory"
    );
    if (tail) {
        __asm__ __volatile__(
            "rep stosb"
            : "+D"(s), "+c"(tail)
            : "a"(fill)
            : "memory"
        );
    }
    return ret;
}

int memcmp(const void *s1, const void *s2, size_t n) {
    const unsigned char *a = s1, *b = s2;
    while (n--) {
        if (*a != *b) return *a - *b;
        a++; b++;
    }
    return 0;
}

void *memchr(const void *s, int c, size_t n) {
    const unsigned char *p = s;
    while (n--) {
        if (*p == (unsigned char)c) return (void *)p;
        p++;
    }
    return NULL;
}

size_t strlen(const char *s) {
    const char *p = s;
    while (*p) p++;
    return p - s;
}

int strcmp(const char *s1, const char *s2) {
    while (*s1 && *s1 == *s2) { s1++; s2++; }
    return *(unsigned char *)s1 - *(unsigned char *)s2;
}

int strncmp(const char *s1, const char *s2, size_t n) {
    while (n && *s1 && *s1 == *s2) { s1++; s2++; n--; }
    if (n == 0) return 0;
    return *(unsigned char *)s1 - *(unsigned char *)s2;
}

char *strcpy(char *dest, const char *src) {
    char *d = dest;
    while ((*d++ = *src++));
    return dest;
}

char *strncpy(char *dest, const char *src, size_t n) {
    char *d = dest;
    while (n > 0) {
        n--;
        if ((*d++ = *src++) == '\0') break;
    }
    while (n > 0) { *d++ = '\0'; n--; }
    return dest;
}

char *strcat(char *dest, const char *src) {
    char *d = dest + strlen(dest);
    while ((*d++ = *src++));
    return dest;
}

char *strncat(char *dest, const char *src, size_t n) {
    char *d = dest + strlen(dest);
    while (n-- && (*d = *src++)) d++;
    *d = '\0';
    return dest;
}

char *strchr(const char *s, int c) {
    while (*s) {
        if (*s == (char)c) return (char *)s;
        s++;
    }
    return (c == '\0') ? (char *)s : NULL;
}

char *strrchr(const char *s, int c) {
    const char *last = NULL;
    while (*s) {
        if (*s == (char)c) last = s;
        s++;
    }
    if (c == '\0') return (char *)s;
    return (char *)last;
}

char *strstr(const char *haystack, const char *needle) {
    size_t nlen = strlen(needle);
    if (nlen == 0) return (char *)haystack;
    while (*haystack) {
        if (strncmp(haystack, needle, nlen) == 0) return (char *)haystack;
        haystack++;
    }
    return NULL;
}

char *strdup(const char *s) {
    size_t len = strlen(s) + 1;
    char *d = malloc(len);
    if (d) memcpy(d, s, len);
    return d;
}

char *strndup(const char *s, size_t n) {
    size_t len = strlen(s);
    if (len > n) len = n;
    char *d = malloc(len + 1);
    if (d) { memcpy(d, s, len); d[len] = '\0'; }
    return d;
}

static const char *_strerror_msgs[] = {
    "Success", "Operation not permitted", "No such file or directory",
    "No such process", "Interrupted", "I/O error"
};

char *strerror(int errnum) {
    if (errnum >= 0 && errnum <= 5) return (char *)_strerror_msgs[errnum];
    return "Unknown error";
}

size_t strspn(const char *s, const char *accept) {
    size_t count = 0;
    while (*s && strchr(accept, *s)) { s++; count++; }
    return count;
}

size_t strcspn(const char *s, const char *reject) {
    size_t count = 0;
    while (*s && !strchr(reject, *s)) { s++; count++; }
    return count;
}

int strcasecmp(const char *s1, const char *s2) {
    while (*s1 && *s2) {
        int c1 = tolower((unsigned char)*s1);
        int c2 = tolower((unsigned char)*s2);
        if (c1 != c2) return c1 - c2;
        s1++; s2++;
    }
    return tolower((unsigned char)*s1) - tolower((unsigned char)*s2);
}

int strncasecmp(const char *s1, const char *s2, size_t n) {
    while (n && *s1 && *s2) {
        int c1 = tolower((unsigned char)*s1);
        int c2 = tolower((unsigned char)*s2);
        if (c1 != c2) return c1 - c2;
        s1++; s2++; n--;
    }
    if (n == 0) return 0;
    return tolower((unsigned char)*s1) - tolower((unsigned char)*s2);
}

char *strcasestr(const char *haystack, const char *needle) {
    size_t nlen = strlen(needle);
    if (nlen == 0) return (char *)haystack;
    while (*haystack) {
        if (strncasecmp(haystack, needle, nlen) == 0) return (char *)haystack;
        haystack++;
    }
    return NULL;
}

char *strchrnul(const char *s, int c) {
    while (*s && *s != (char)c) s++;
    return (char *)s;
}

char *strpbrk(const char *s, const char *accept) {
    while (*s) {
        if (strchr(accept, *s)) return (char *)s;
        s++;
    }
    return NULL;
}

void *memrchr(const void *s, int c, size_t n) {
    const unsigned char *p = (const unsigned char *)s + n;
    while (n--) {
        --p;
        if (*p == (unsigned char)c) return (void *)p;
    }
    return NULL;
}

static char *_strtok_last = NULL;
char *strtok(char *str, const char *delim) {
    if (str) _strtok_last = str;
    if (!_strtok_last) return NULL;
    _strtok_last += strspn(_strtok_last, delim);
    if (*_strtok_last == '\0') { _strtok_last = NULL; return NULL; }
    char *token = _strtok_last;
    _strtok_last += strcspn(_strtok_last, delim);
    if (*_strtok_last) *_strtok_last++ = '\0';
    else _strtok_last = NULL;
    return token;
}
