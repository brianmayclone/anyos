/*
 * Copyright (c) 2024-2026 Christian Moeller
 * Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
 *
 * This project is open source and community-driven.
 * Contributions are welcome! See README.md for details.
 *
 * SPDX-License-Identifier: MIT
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <errno.h>

/* Static FILE objects for stdin/stdout/stderr */
static FILE _stdin  = { .fd = 0, .flags = 0, .eof = 0, .error = 0 };
static FILE _stdout = { .fd = 1, .flags = 1, .eof = 0, .error = 0 };
static FILE _stderr = { .fd = 2, .flags = 1, .eof = 0, .error = 0 };

FILE *stdin  = &_stdin;
FILE *stdout = &_stdout;
FILE *stderr = &_stderr;

int errno = 0;

FILE *fopen(const char *path, const char *mode) {
    int flags = 0;
    if (strcmp(mode, "r") == 0) flags = O_RDONLY;
    else if (strcmp(mode, "w") == 0) flags = O_WRONLY | O_CREAT | O_TRUNC;
    else if (strcmp(mode, "a") == 0) flags = O_WRONLY | O_CREAT | O_APPEND;
    else if (strcmp(mode, "rb") == 0) flags = O_RDONLY;
    else if (strcmp(mode, "wb") == 0) flags = O_WRONLY | O_CREAT | O_TRUNC;
    else if (strcmp(mode, "r+") == 0 || strcmp(mode, "r+b") == 0) flags = O_RDWR;
    else if (strcmp(mode, "w+") == 0 || strcmp(mode, "w+b") == 0) flags = O_RDWR | O_CREAT | O_TRUNC;
    else flags = O_RDONLY;

    int fd = open(path, flags);
    if (fd < 0) return NULL;

    FILE *f = calloc(1, sizeof(FILE));
    if (!f) { close(fd); return NULL; }
    f->fd = fd;
    f->flags = (flags & O_WRONLY) || (flags & O_RDWR) ? 1 : 0;
    return f;
}

int fclose(FILE *stream) {
    if (!stream) return EOF;
    fflush(stream);
    int ret = close(stream->fd);
    if (stream != stdin && stream != stdout && stream != stderr)
        free(stream);
    return ret < 0 ? EOF : 0;
}

size_t fread(void *ptr, size_t size, size_t nmemb, FILE *stream) {
    if (!stream || size == 0 || nmemb == 0) return 0;
    size_t total = size * nmemb;
    ssize_t n = read(stream->fd, ptr, total);
    if (n <= 0) {
        if (n == 0) stream->eof = 1;
        else stream->error = 1;
        return 0;
    }
    return n / size;
}

size_t fwrite(const void *ptr, size_t size, size_t nmemb, FILE *stream) {
    if (!stream || size == 0 || nmemb == 0) return 0;
    size_t total = size * nmemb;
    ssize_t n = write(stream->fd, ptr, total);
    if (n < 0) { stream->error = 1; return 0; }
    return n / size;
}

int fseek(FILE *stream, long offset, int whence) {
    if (!stream) return -1;
    int ret = lseek(stream->fd, offset, whence);
    if (ret < 0) return -1;
    stream->eof = 0;
    return 0;
}

long ftell(FILE *stream) {
    if (!stream) return -1;
    return lseek(stream->fd, 0, SEEK_CUR);
}

void rewind(FILE *stream) {
    if (stream) {
        fseek(stream, 0, SEEK_SET);
        stream->error = 0;
    }
}

int feof(FILE *stream) { return stream ? stream->eof : 0; }
int ferror(FILE *stream) { return stream ? stream->error : 0; }
void clearerr(FILE *stream) { if (stream) { stream->eof = 0; stream->error = 0; } }

int fflush(FILE *stream) {
    (void)stream;
    return 0; /* unbuffered for now */
}

int fgetc(FILE *stream) {
    unsigned char c;
    if (fread(&c, 1, 1, stream) == 1) return c;
    return EOF;
}

int fputc(int c, FILE *stream) {
    unsigned char ch = (unsigned char)c;
    if (fwrite(&ch, 1, 1, stream) == 1) return c;
    return EOF;
}

char *fgets(char *s, int size, FILE *stream) {
    if (size <= 0) return NULL;
    int i = 0;
    while (i < size - 1) {
        int c = fgetc(stream);
        if (c == EOF) { if (i == 0) return NULL; break; }
        s[i++] = c;
        if (c == '\n') break;
    }
    s[i] = '\0';
    return s;
}

int fputs(const char *s, FILE *stream) {
    size_t len = strlen(s);
    return fwrite(s, 1, len, stream) == len ? 0 : EOF;
}

int getc(FILE *stream) { return fgetc(stream); }
int putc(int c, FILE *stream) { return fputc(c, stream); }
int getchar(void) { return fgetc(stdin); }
int putchar(int c) { return fputc(c, stdout); }
int puts(const char *s) { fputs(s, stdout); fputc('\n', stdout); return 0; }

/* --- printf implementation --- */

static int _put_char(char *buf, size_t pos, size_t max, char c) {
    if (pos < max) buf[pos] = c;
    return 1;
}

static int _put_string(char *buf, size_t pos, size_t max, const char *s) {
    int n = 0;
    while (*s) { n += _put_char(buf, pos + n, max, *s++); }
    return n;
}

static int _put_uint(char *buf, size_t pos, size_t max, unsigned long val, int base, int uppercase, int width, int zero_pad) {
    char tmp[32];
    int i = 0;
    const char *digits = uppercase ? "0123456789ABCDEF" : "0123456789abcdef";

    if (val == 0) tmp[i++] = '0';
    else {
        while (val) {
            tmp[i++] = digits[val % base];
            val /= base;
        }
    }

    int n = 0;
    int pad = width > i ? width - i : 0;
    char pad_char = zero_pad ? '0' : ' ';
    while (pad-- > 0) n += _put_char(buf, pos + n, max, pad_char);
    while (i > 0) n += _put_char(buf, pos + n, max, tmp[--i]);
    return n;
}

int vsnprintf(char *str, size_t size, const char *format, va_list ap) {
    size_t pos = 0;

    while (*format) {
        if (*format != '%') {
            pos += _put_char(str, pos, size ? size - 1 : 0, *format++);
            continue;
        }
        format++; /* skip '%' */

        /* Flags */
        int zero_pad = 0, left_align = 0;
        while (*format == '0' || *format == '-') {
            if (*format == '0') zero_pad = 1;
            if (*format == '-') left_align = 1;
            format++;
        }

        /* Width */
        int width = 0;
        if (*format == '*') {
            width = va_arg(ap, int);
            format++;
        } else {
            while (*format >= '0' && *format <= '9') {
                width = width * 10 + (*format - '0');
                format++;
            }
        }

        /* Precision (skip) */
        int precision = -1;
        if (*format == '.') {
            format++;
            precision = 0;
            if (*format == '*') {
                precision = va_arg(ap, int);
                format++;
            } else {
                while (*format >= '0' && *format <= '9') {
                    precision = precision * 10 + (*format - '0');
                    format++;
                }
            }
        }

        /* Length modifier */
        int is_long = 0;
        if (*format == 'l') { is_long = 1; format++; if (*format == 'l') format++; }
        else if (*format == 'h') { format++; if (*format == 'h') format++; }
        else if (*format == 'z') { format++; }

        size_t max = size ? size - 1 : 0;

        /* For integer types, precision means minimum digits (zero-padded) */
        switch (*format) {
            case 'd': case 'i': {
                long val = is_long ? va_arg(ap, long) : va_arg(ap, int);
                if (val < 0) {
                    pos += _put_char(str, pos, max, '-');
                    val = -val;
                    if (width > 0) width--;
                }
                int iw = width, izp = zero_pad;
                if (precision >= 0) { iw = precision; izp = 1; }
                pos += _put_uint(str, pos, max, (unsigned long)val, 10, 0, iw, izp);
                break;
            }
            case 'u': {
                unsigned long val = is_long ? va_arg(ap, unsigned long) : va_arg(ap, unsigned int);
                int iw = width, izp = zero_pad;
                if (precision >= 0) { iw = precision; izp = 1; }
                pos += _put_uint(str, pos, max, val, 10, 0, iw, izp);
                break;
            }
            case 'x': {
                unsigned long val = is_long ? va_arg(ap, unsigned long) : va_arg(ap, unsigned int);
                int iw = width, izp = zero_pad;
                if (precision >= 0) { iw = precision; izp = 1; }
                pos += _put_uint(str, pos, max, val, 16, 0, iw, izp);
                break;
            }
            case 'X': {
                unsigned long val = is_long ? va_arg(ap, unsigned long) : va_arg(ap, unsigned int);
                int iw = width, izp = zero_pad;
                if (precision >= 0) { iw = precision; izp = 1; }
                pos += _put_uint(str, pos, max, val, 16, 1, iw, izp);
                break;
            }
            case 'o': {
                unsigned long val = is_long ? va_arg(ap, unsigned long) : va_arg(ap, unsigned int);
                int iw = width, izp = zero_pad;
                if (precision >= 0) { iw = precision; izp = 1; }
                pos += _put_uint(str, pos, max, val, 8, 0, iw, izp);
                break;
            }
            case 'p': {
                unsigned long val = (unsigned long)va_arg(ap, void *);
                pos += _put_string(str, pos, max, "0x");
                pos += _put_uint(str, pos, max, val, 16, 0, 8, 1);
                break;
            }
            case 's': {
                const char *s = va_arg(ap, const char *);
                if (!s) s = "(null)";
                if (precision >= 0) {
                    int len = 0;
                    while (len < precision && s[len]) len++;
                    for (int k = 0; k < len; k++)
                        pos += _put_char(str, pos, max, s[k]);
                } else {
                    pos += _put_string(str, pos, max, s);
                }
                break;
            }
            case 'c': {
                char c = (char)va_arg(ap, int);
                pos += _put_char(str, pos, max, c);
                break;
            }
            case '%':
                pos += _put_char(str, pos, max, '%');
                break;
            case 'n': {
                int *n = va_arg(ap, int *);
                if (n) *n = (int)pos;
                break;
            }
            default:
                pos += _put_char(str, pos, max, '%');
                pos += _put_char(str, pos, max, *format);
                break;
        }
        format++;
    }

    if (size > 0) str[pos < size - 1 ? pos : size - 1] = '\0';
    return (int)pos;
}

int vsprintf(char *str, const char *format, va_list ap) {
    return vsnprintf(str, (size_t)-1, format, ap);
}

int snprintf(char *str, size_t size, const char *format, ...) {
    va_list ap;
    va_start(ap, format);
    int ret = vsnprintf(str, size, format, ap);
    va_end(ap);
    return ret;
}

int sprintf(char *str, const char *format, ...) {
    va_list ap;
    va_start(ap, format);
    int ret = vsprintf(str, format, ap);
    va_end(ap);
    return ret;
}

int vfprintf(FILE *stream, const char *format, va_list ap) {
    char buf[4096];
    int n = vsnprintf(buf, sizeof(buf), format, ap);
    if (n > 0) fwrite(buf, 1, n < (int)sizeof(buf) ? n : (int)sizeof(buf) - 1, stream);
    return n;
}

int vprintf(const char *format, va_list ap) {
    return vfprintf(stdout, format, ap);
}

int fprintf(FILE *stream, const char *format, ...) {
    va_list ap;
    va_start(ap, format);
    int ret = vfprintf(stream, format, ap);
    va_end(ap);
    return ret;
}

int printf(const char *format, ...) {
    va_list ap;
    va_start(ap, format);
    int ret = vprintf(format, ap);
    va_end(ap);
    return ret;
}

int sscanf(const char *str, const char *format, ...) {
    /* Minimal sscanf: only supports %d and %s */
    va_list ap;
    va_start(ap, format);
    int count = 0;

    while (*format && *str) {
        if (*format == '%') {
            format++;
            if (*format == 'd') {
                int *val = va_arg(ap, int *);
                int neg = 0, n = 0, has = 0;
                while (*str == ' ') str++;
                if (*str == '-') { neg = 1; str++; }
                while (*str >= '0' && *str <= '9') {
                    n = n * 10 + (*str - '0');
                    str++; has = 1;
                }
                if (has) { *val = neg ? -n : n; count++; }
                else break;
            } else if (*format == 's') {
                char *s = va_arg(ap, char *);
                while (*str == ' ') str++;
                while (*str && *str != ' ' && *str != '\n') *s++ = *str++;
                *s = '\0';
                count++;
            } else break;
            format++;
        } else if (*format == *str) {
            format++; str++;
        } else break;
    }

    va_end(ap);
    return count;
}

int remove(const char *pathname) {
    return unlink(pathname);
}

int rename(const char *oldpath, const char *newpath) {
    (void)oldpath; (void)newpath;
    errno = ENOSYS;
    return -1;
}

FILE *tmpfile(void) {
    return NULL;
}

FILE *fdopen(int fd, const char *mode) {
    if (fd < 0) return NULL;
    FILE *f = calloc(1, sizeof(FILE));
    if (!f) return NULL;
    f->fd = fd;
    f->flags = (mode[0] == 'w' || mode[0] == 'a' || (mode[0] == 'r' && mode[1] == '+')) ? 1 : 0;
    return f;
}

int fileno(FILE *stream) {
    if (!stream) return -1;
    return stream->fd;
}

int setvbuf(FILE *stream, char *buf, int mode, size_t size) {
    (void)stream; (void)buf; (void)mode; (void)size;
    return 0; /* unbuffered */
}

void __assert_fail(const char *expr, const char *file, int line) {
    fprintf(stderr, "Assertion failed: %s at %s:%d\n", expr, file, line);
    abort();
}
