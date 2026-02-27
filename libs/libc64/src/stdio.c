/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * libc64 — x86_64 stdio implementation.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <errno.h>

static FILE _stdin  = { .fd = 0, .flags = 0, .eof = 0, .error = 0, .ungot = -1 };
static FILE _stdout = { .fd = 1, .flags = 1, .eof = 0, .error = 0, .ungot = -1 };
static FILE _stderr = { .fd = 2, .flags = 1, .eof = 0, .error = 0, .ungot = -1 };

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
    f->ungot = -1;
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
    off_t ret = lseek(stream->fd, offset, whence);
    if (ret < 0) return -1;
    stream->eof = 0;
    return 0;
}

long ftell(FILE *stream) {
    if (!stream) return -1;
    return (long)lseek(stream->fd, 0, SEEK_CUR);
}

void rewind(FILE *stream) {
    if (stream) { fseek(stream, 0, SEEK_SET); stream->error = 0; }
}

int feof(FILE *stream) { return stream ? stream->eof : 0; }
int ferror(FILE *stream) { return stream ? stream->error : 0; }
void clearerr(FILE *stream) { if (stream) { stream->eof = 0; stream->error = 0; } }

int fflush(FILE *stream) {
    (void)stream;
    return 0;
}

int fgetc(FILE *stream) {
    if (stream->ungot >= 0) {
        int c = stream->ungot;
        stream->ungot = -1;
        return c;
    }
    unsigned char c;
    if (fread(&c, 1, 1, stream) == 1) return c;
    return EOF;
}

int ungetc(int c, FILE *stream) {
    if (c == EOF || !stream) return EOF;
    stream->ungot = (unsigned char)c;
    stream->eof = 0;
    return c;
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

/* --- Float formatting helpers for %f, %e, %g --- */

/* Union for IEEE 754 double bit access. */
typedef union { double d; unsigned long long u; } _double_bits;

static int _is_nan(double v) { _double_bits b; b.d = v; return ((b.u >> 52) & 0x7FF) == 0x7FF && (b.u & 0x000FFFFFFFFFFFFFULL); }
static int _is_inf(double v) { _double_bits b; b.d = v; return ((b.u >> 52) & 0x7FF) == 0x7FF && (b.u & 0x000FFFFFFFFFFFFFULL) == 0; }
static int _is_neg(double v) { _double_bits b; b.d = v; return (b.u >> 63) != 0; }

/* Format a double as %f (fixed-point). Returns chars written. */
static int _put_float_f(char *buf, size_t pos, size_t max, double val, int prec, int width, int zero_pad) {
    int n = 0;
    if (_is_nan(val)) return _put_string(buf, pos, max, "nan");
    if (_is_inf(val)) return _put_string(buf, pos, max, _is_neg(val) ? "-inf" : "inf");
    if (_is_neg(val)) { n += _put_char(buf, pos + n, max, '-'); val = -val; }

    if (prec < 0) prec = 6;

    /* Separate integer and fractional parts. */
    unsigned long long int_part = (unsigned long long)val;
    double frac = val - (double)int_part;

    /* Round the fractional part. */
    double round_add = 0.5;
    for (int i = 0; i < prec; i++) round_add /= 10.0;
    frac += round_add;
    if (frac >= 1.0) { int_part++; frac -= 1.0; }

    /* Integer digits. */
    char itmp[24]; int ilen = 0;
    if (int_part == 0) itmp[ilen++] = '0';
    else { unsigned long long iv = int_part; while (iv) { itmp[ilen++] = '0' + (char)(iv % 10); iv /= 10; } }

    /* Width padding (before sign which is already written). */
    int total_len = ilen + (prec > 0 ? 1 + prec : 0);
    if (width > total_len + n) {
        int pad = width - total_len - n;
        char pc = zero_pad ? '0' : ' ';
        while (pad-- > 0) n += _put_char(buf, pos + n, max, pc);
    }

    while (ilen > 0) n += _put_char(buf, pos + n, max, itmp[--ilen]);

    if (prec > 0) {
        n += _put_char(buf, pos + n, max, '.');
        for (int i = 0; i < prec; i++) {
            frac *= 10.0;
            int digit = (int)frac;
            if (digit > 9) digit = 9;
            n += _put_char(buf, pos + n, max, '0' + digit);
            frac -= digit;
        }
    }
    return n;
}

/* Format a double as %e (scientific notation). */
static int _put_float_e(char *buf, size_t pos, size_t max, double val, int prec, int uppercase) {
    int n = 0;
    if (_is_nan(val)) return _put_string(buf, pos, max, uppercase ? "NAN" : "nan");
    if (_is_inf(val)) return _put_string(buf, pos, max, _is_neg(val) ? (uppercase ? "-INF" : "-inf") : (uppercase ? "INF" : "inf"));
    if (_is_neg(val)) { n += _put_char(buf, pos + n, max, '-'); val = -val; }

    if (prec < 0) prec = 6;

    int exponent = 0;
    if (val != 0.0) {
        while (val >= 10.0) { val /= 10.0; exponent++; }
        while (val < 1.0) { val *= 10.0; exponent--; }
    }

    /* val is now in [1.0, 10.0) — format as %f with that mantissa */
    n += _put_float_f(buf, pos + n, max, val, prec, 0, 0);
    n += _put_char(buf, pos + n, max, uppercase ? 'E' : 'e');
    n += _put_char(buf, pos + n, max, exponent >= 0 ? '+' : '-');
    if (exponent < 0) exponent = -exponent;
    if (exponent < 10) n += _put_char(buf, pos + n, max, '0');
    /* Print exponent digits. */
    char etmp[8]; int elen = 0;
    if (exponent == 0) etmp[elen++] = '0';
    else { while (exponent) { etmp[elen++] = '0' + (exponent % 10); exponent /= 10; } }
    while (elen > 0) n += _put_char(buf, pos + n, max, etmp[--elen]);
    return n;
}

/* Format a double as %g (shortest of %f or %e, trailing zeros stripped). */
static int _put_float_g(char *buf, size_t pos, size_t max, double val, int prec, int uppercase) {
    if (prec < 0) prec = 6;
    if (prec == 0) prec = 1;

    if (_is_nan(val) || _is_inf(val))
        return _put_float_e(buf, pos, max, val, prec, uppercase);

    double aval = val < 0 ? -val : val;
    int exponent = 0;
    if (aval != 0.0) {
        double tmp = aval;
        while (tmp >= 10.0) { tmp /= 10.0; exponent++; }
        while (tmp < 1.0) { tmp *= 10.0; exponent--; }
    }

    /* Use %e if exponent < -4 or >= precision. */
    char tmp_buf[128];
    int len;
    if (exponent < -4 || exponent >= prec)
        len = _put_float_e(tmp_buf, 0, sizeof(tmp_buf) - 1, val, prec - 1, uppercase);
    else
        len = _put_float_f(tmp_buf, 0, sizeof(tmp_buf) - 1, val, prec - 1 - exponent, 0, 0);
    tmp_buf[len] = '\0';

    /* Strip trailing zeros after decimal point. */
    int has_dot = 0;
    for (int i = 0; i < len; i++) { if (tmp_buf[i] == '.') { has_dot = 1; break; } if (tmp_buf[i] == 'e' || tmp_buf[i] == 'E') break; }
    if (has_dot) {
        int e_pos = len;
        for (int i = 0; i < len; i++) if (tmp_buf[i] == 'e' || tmp_buf[i] == 'E') { e_pos = i; break; }
        int trail = e_pos - 1;
        while (trail > 0 && tmp_buf[trail] == '0') trail--;
        if (tmp_buf[trail] == '.') trail--;
        /* Rebuild: prefix + exponent part. */
        if (e_pos < len) {
            int elen = len - e_pos;
            for (int i = 0; i < elen; i++) tmp_buf[trail + 1 + i] = tmp_buf[e_pos + i];
            len = trail + 1 + (len - e_pos);
        } else {
            len = trail + 1;
        }
        tmp_buf[len] = '\0';
    }

    int n = 0;
    for (int i = 0; i < len; i++) n += _put_char(buf, pos + n, max, tmp_buf[i]);
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
        format++;

        int zero_pad = 0, left_align = 0;
        while (*format == '0' || *format == '-') {
            if (*format == '0') zero_pad = 1;
            if (*format == '-') left_align = 1;
            format++;
        }
        (void)left_align;

        int width = 0;
        if (*format == '*') { width = va_arg(ap, int); format++; }
        else { while (*format >= '0' && *format <= '9') { width = width * 10 + (*format - '0'); format++; } }

        int precision = -1;
        if (*format == '.') {
            format++;
            precision = 0;
            if (*format == '*') { precision = va_arg(ap, int); format++; }
            else { while (*format >= '0' && *format <= '9') { precision = precision * 10 + (*format - '0'); format++; } }
        }

        int is_long = 0, is_longlong = 0;
        if (*format == 'l') { is_long = 1; format++; if (*format == 'l') { is_longlong = 1; format++; } }
        else if (*format == 'h') { format++; if (*format == 'h') format++; }
        else if (*format == 'z') { is_long = 1; format++; }

        size_t max = size ? size - 1 : 0;

        switch (*format) {
            case 'd': case 'i': {
                long long val = is_longlong ? va_arg(ap, long long) : is_long ? va_arg(ap, long) : va_arg(ap, int);
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
                unsigned long long val = is_longlong ? va_arg(ap, unsigned long long) : is_long ? va_arg(ap, unsigned long) : va_arg(ap, unsigned int);
                int iw = width, izp = zero_pad;
                if (precision >= 0) { iw = precision; izp = 1; }
                pos += _put_uint(str, pos, max, (unsigned long)val, 10, 0, iw, izp);
                break;
            }
            case 'x': {
                unsigned long long val = is_longlong ? va_arg(ap, unsigned long long) : is_long ? va_arg(ap, unsigned long) : va_arg(ap, unsigned int);
                int iw = width, izp = zero_pad;
                if (precision >= 0) { iw = precision; izp = 1; }
                pos += _put_uint(str, pos, max, (unsigned long)val, 16, 0, iw, izp);
                break;
            }
            case 'X': {
                unsigned long long val = is_longlong ? va_arg(ap, unsigned long long) : is_long ? va_arg(ap, unsigned long) : va_arg(ap, unsigned int);
                int iw = width, izp = zero_pad;
                if (precision >= 0) { iw = precision; izp = 1; }
                pos += _put_uint(str, pos, max, (unsigned long)val, 16, 1, iw, izp);
                break;
            }
            case 'o': {
                unsigned long long val = is_longlong ? va_arg(ap, unsigned long long) : is_long ? va_arg(ap, unsigned long) : va_arg(ap, unsigned int);
                int iw = width, izp = zero_pad;
                if (precision >= 0) { iw = precision; izp = 1; }
                pos += _put_uint(str, pos, max, (unsigned long)val, 8, 0, iw, izp);
                break;
            }
            case 'p': {
                unsigned long val = (unsigned long)va_arg(ap, void *);
                pos += _put_string(str, pos, max, "0x");
                pos += _put_uint(str, pos, max, val, 16, 0, 16, 1);
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
            case 'f': case 'F': {
                double val = va_arg(ap, double);
                pos += _put_float_f(str, pos, max, val, precision, width, zero_pad);
                break;
            }
            case 'e': case 'E': {
                double val = va_arg(ap, double);
                pos += _put_float_e(str, pos, max, val, precision, *format == 'E');
                break;
            }
            case 'g': case 'G': {
                double val = va_arg(ap, double);
                pos += _put_float_g(str, pos, max, val, precision, *format == 'G');
                break;
            }
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
    va_list ap; va_start(ap, format);
    int ret = vsnprintf(str, size, format, ap);
    va_end(ap); return ret;
}

int sprintf(char *str, const char *format, ...) {
    va_list ap; va_start(ap, format);
    int ret = vsprintf(str, format, ap);
    va_end(ap); return ret;
}

int vfprintf(FILE *stream, const char *format, va_list ap) {
    char buf[4096];
    int n = vsnprintf(buf, sizeof(buf), format, ap);
    if (n > 0) fwrite(buf, 1, n < (int)sizeof(buf) ? n : (int)sizeof(buf) - 1, stream);
    return n;
}

int vprintf(const char *format, va_list ap) { return vfprintf(stdout, format, ap); }

int fprintf(FILE *stream, const char *format, ...) {
    va_list ap; va_start(ap, format);
    int ret = vfprintf(stream, format, ap);
    va_end(ap); return ret;
}

int printf(const char *format, ...) {
    va_list ap; va_start(ap, format);
    int ret = vprintf(format, ap);
    va_end(ap); return ret;
}

int sscanf(const char *str, const char *format, ...) {
    va_list ap; va_start(ap, format);
    int count = 0;
    while (*format && *str) {
        if (*format == '%') {
            format++;
            if (*format == 'd') {
                int *val = va_arg(ap, int *);
                int neg = 0, n = 0, has = 0;
                while (*str == ' ') str++;
                if (*str == '-') { neg = 1; str++; }
                while (*str >= '0' && *str <= '9') { n = n * 10 + (*str - '0'); str++; has = 1; }
                if (has) { *val = neg ? -n : n; count++; } else break;
            } else if (*format == 's') {
                char *s = va_arg(ap, char *);
                while (*str == ' ') str++;
                while (*str && *str != ' ' && *str != '\n') *s++ = *str++;
                *s = '\0'; count++;
            } else break;
            format++;
        } else if (*format == *str) { format++; str++; }
        else break;
    }
    va_end(ap); return count;
}

int fscanf(FILE *stream, const char *format, ...) {
    va_list ap; va_start(ap, format);
    int count = 0;
    while (*format) {
        if (*format == '%') {
            format++;
            if (*format == 'i' || *format == 'd') {
                int *val = va_arg(ap, int *);
                int c, neg = 0, n = 0, has = 0;
                while ((c = fgetc(stream)) != EOF && (c == ' ' || c == '\t' || c == '\n'));
                if (c == EOF) break;
                if (c == '-') neg = 1;
                else if (c == '+') { }
                else if (c >= '0' && c <= '9') { n = c - '0'; has = 1; }
                else { ungetc(c, stream); break; }
                while ((c = fgetc(stream)) != EOF && c >= '0' && c <= '9') { n = n * 10 + (c - '0'); has = 1; }
                if (c != EOF) ungetc(c, stream);
                if (has) { *val = neg ? -n : n; count++; } else break;
            } else if (*format == 's') {
                char *s = va_arg(ap, char *);
                int c;
                while ((c = fgetc(stream)) != EOF && (c == ' ' || c == '\t' || c == '\n'));
                if (c == EOF) break;
                *s++ = c;
                while ((c = fgetc(stream)) != EOF && c != ' ' && c != '\t' && c != '\n') *s++ = c;
                *s = '\0';
                if (c != EOF) ungetc(c, stream);
                count++;
            } else break;
            format++;
        } else if (*format == ' ' || *format == '\t' || *format == '\n') {
            int c;
            while ((c = fgetc(stream)) != EOF && (c == ' ' || c == '\t' || c == '\n'));
            if (c != EOF) ungetc(c, stream);
            format++;
        } else {
            int c = fgetc(stream);
            if (c != *format) { if (c != EOF) ungetc(c, stream); break; }
            format++;
        }
    }
    va_end(ap); return count;
}

extern long _syscall(long num, long a1, long a2, long a3, long a4, long a5);

int remove(const char *pathname) { return unlink(pathname); }

int rename(const char *oldpath, const char *newpath) {
    if (!oldpath || !newpath) { errno = EINVAL; return -1; }
    long r = _syscall(99 /*SYS_RENAME*/, (long)oldpath, (long)newpath, 0, 0, 0);
    if (r < 0) { errno = (int)-r; return -1; }
    return 0;
}

FILE *tmpfile(void) {
    char tmpl[] = "/tmp/tmpXXXXXX";
    int fd = mkstemp(tmpl);
    if (fd < 0) return NULL;
    /* Unlink immediately so file is deleted when closed */
    unlink(tmpl);
    return fdopen(fd, "w+");
}

FILE *fdopen(int fd, const char *mode) {
    if (fd < 0) return NULL;
    FILE *f = calloc(1, sizeof(FILE));
    if (!f) return NULL;
    f->fd = fd;
    f->flags = (mode[0] == 'w' || mode[0] == 'a' || (mode[0] == 'r' && mode[1] == '+')) ? 1 : 0;
    f->ungot = -1;
    return f;
}

int fileno(FILE *stream) { return stream ? stream->fd : -1; }
int setvbuf(FILE *stream, char *buf, int mode, size_t size) { (void)stream; (void)buf; (void)mode; (void)size; return 0; }

FILE *freopen(const char *path, const char *mode, FILE *stream) {
    (void)path; (void)mode;
    return stream;
}

void __assert_fail(const char *expr, const char *file, int line) {
    fprintf(stderr, "Assertion failed: %s at %s:%d\n", expr, file, line);
    abort();
}

void perror(const char *s) {
    if (s && *s) { fputs(s, stderr); fputs(": ", stderr); }
    fputs(strerror(errno), stderr);
    fputc('\n', stderr);
}
