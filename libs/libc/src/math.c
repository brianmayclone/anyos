/*
 * Copyright (c) 2024-2026 Christian Moeller
 * Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
 *
 * This project is open source and community-driven.
 * Contributions are welcome! See README.md for details.
 *
 * SPDX-License-Identifier: MIT
 */

#include <math.h>
#include <stddef.h>

/* ldexp: x * 2^exp — used by TCC for floating-point constant evaluation */
double ldexp(double x, int exp) {
    /* Use repeated multiply/divide by 2 for simplicity */
    if (exp > 0) {
        while (exp-- > 0) x *= 2.0;
    } else {
        while (exp++ < 0) x /= 2.0;
    }
    return x;
}

double frexp(double x, int *exp) {
    *exp = 0;
    if (x == 0.0) return 0.0;
    int neg = 0;
    if (x < 0.0) { neg = 1; x = -x; }
    while (x >= 1.0) { x /= 2.0; (*exp)++; }
    while (x < 0.5) { x *= 2.0; (*exp)--; }
    return neg ? -x : x;
}

double modf(double x, double *iptr) {
    double i = (double)(long)x;
    *iptr = i;
    return x - i;
}

double fabs(double x) { return x < 0.0 ? -x : x; }

double floor(double x) {
    long i = (long)x;
    if (x < 0.0 && x != (double)i) i--;
    return (double)i;
}

double ceil(double x) {
    long i = (long)x;
    if (x > 0.0 && x != (double)i) i++;
    return (double)i;
}

double sqrt(double x) {
    double result;
    __asm__ __volatile__("fsqrt" : "=t"(result) : "0"(x));
    return result;
}

double pow(double base, double exponent) {
    if (exponent == 0.0) return 1.0;
    if (base == 0.0) return 0.0;
    if (base == 1.0) return 1.0;
    /* Integer exponent fast path */
    int neg = 0;
    double e = exponent;
    if (e < 0.0) { neg = 1; e = -e; }
    long iexp = (long)e;
    if ((double)iexp == e) {
        double result = 1.0;
        double b = base;
        while (iexp > 0) {
            if (iexp & 1) result *= b;
            b *= b;
            iexp >>= 1;
        }
        return neg ? 1.0 / result : result;
    }
    /* General case: 2^(exponent * log2(base)) via x87 */
    /* fyl2x computes y * log2(x) */
    double t;
    __asm__ __volatile__(
        "fyl2x"
        : "=t"(t)
        : "0"(base), "u"(exponent)
        : "st(1)"
    );
    /* Now compute 2^t: split into integer + fraction, use f2xm1 */
    double result;
    __asm__ __volatile__(
        "fld %%st(0)\n\t"   /* duplicate t */
        "frndint\n\t"       /* int(t) */
        "fxch %%st(1)\n\t"
        "fsub %%st(1), %%st(0)\n\t" /* frac = t - int(t) */
        "f2xm1\n\t"         /* 2^frac - 1 */
        "fld1\n\t"
        "faddp\n\t"          /* 2^frac */
        "fscale\n\t"         /* 2^frac * 2^int = 2^t */
        "fstp %%st(1)\n\t"   /* pop the integer part */
        : "=t"(result)
        : "0"(t)
    );
    return result;
}

double log(double x) {
    if (x <= 0.0) return -HUGE_VAL;
    /* Simple series approximation: log(x) using log((1+y)/(1-y)) = 2*(y + y^3/3 + y^5/5 + ...) */
    int k = 0;
    while (x > 2.0) { x /= 2.0; k++; }
    while (x < 1.0) { x *= 2.0; k--; }
    double y = (x - 1.0) / (x + 1.0);
    double y2 = y * y;
    double term = y;
    double sum = 0.0;
    for (int i = 0; i < 20; i++) {
        sum += term / (2 * i + 1);
        term *= y2;
    }
    return 2.0 * sum + k * 0.693147180559945309;
}

double log2(double x) {
    return log(x) / 0.693147180559945309;
}

double exp(double x) {
    if (x == 0.0) return 1.0;
    /* Taylor series: e^x = 1 + x + x^2/2! + x^3/3! + ... */
    double term = 1.0;
    double sum = 1.0;
    for (int i = 1; i < 30; i++) {
        term *= x / i;
        sum += term;
    }
    return sum;
}

double log10(double x) {
    return log(x) / 2.302585092994045684;
}

/* ── Trigonometric functions via x87 FPU ── */

double sin(double x) {
    double result;
    __asm__ __volatile__("fsin" : "=t"(result) : "0"(x));
    return result;
}

double cos(double x) {
    double result;
    __asm__ __volatile__("fcos" : "=t"(result) : "0"(x));
    return result;
}

double tan(double x) {
    double result;
    /* fptan pushes 1.0 then tan(x); pop the 1.0 */
    __asm__ __volatile__(
        "fptan\n\t"
        "fstp %%st(0)"
        : "=t"(result)
        : "0"(x)
    );
    return result;
}

double atan(double x) {
    double result;
    /* fpatan computes atan(st1/st0); load 1.0 to get atan(x/1) = atan(x) */
    __asm__ __volatile__(
        "fld1\n\t"
        "fpatan"
        : "=t"(result)
        : "0"(x)
    );
    return result;
}

double atan2(double y, double x) {
    double result;
    /* fpatan computes atan(st1/st0) = atan(y/x) */
    __asm__ __volatile__(
        "fpatan"
        : "=t"(result)
        : "0"(x), "u"(y)
        : "st(1)"
    );
    return result;
}

double asin(double x) {
    /* asin(x) = atan2(x, sqrt(1 - x*x)) */
    return atan2(x, sqrt(1.0 - x * x));
}

double acos(double x) {
    /* acos(x) = atan2(sqrt(1 - x*x), x) */
    return atan2(sqrt(1.0 - x * x), x);
}

double fmod(double x, double y) {
    if (y == 0.0) return NAN;
    double result;
    __asm__ __volatile__(
        "1:\n\t"
        "fprem\n\t"
        "fnstsw %%ax\n\t"
        "testw $0x400, %%ax\n\t"
        "jnz 1b"
        : "=t"(result)
        : "0"(x), "u"(y)
        : "ax", "st(1)"
    );
    return result;
}

/* ── float variants ── */

float fabsf(float x) { return x < 0.0f ? -x : x; }
float sqrtf(float x) { return (float)sqrt((double)x); }
float sinf(float x) { return (float)sin((double)x); }
float cosf(float x) { return (float)cos((double)x); }
float atan2f(float y, float x) { return (float)atan2((double)y, (double)x); }
float fmodf(float x, float y) { return (float)fmod((double)x, (double)y); }
float floorf(float x) { return (float)floor((double)x); }
float ceilf(float x) { return (float)ceil((double)x); }
float powf(float x, float y) { return (float)pow((double)x, (double)y); }
float logf(float x) { return (float)log((double)x); }
float log2f(float x) { return (float)log2((double)x); }
float log10f(float x) { return (float)log10((double)x); }
float expf(float x) { return (float)exp((double)x); }

/* Parse a floating-point number string */
static double _parse_double(const char *nptr, char **endptr) {
    const char *s = nptr;
    while (*s == ' ' || *s == '\t' || *s == '\n') s++;

    int neg = 0;
    if (*s == '-') { neg = 1; s++; }
    else if (*s == '+') s++;

    /* Handle hex float: 0xH.Hp+/-N */
    if (s[0] == '0' && (s[1] == 'x' || s[1] == 'X')) {
        s += 2;
        double result = 0.0;
        int has_digits = 0;
        while (1) {
            int d;
            if (*s >= '0' && *s <= '9') d = *s - '0';
            else if (*s >= 'a' && *s <= 'f') d = *s - 'a' + 10;
            else if (*s >= 'A' && *s <= 'F') d = *s - 'A' + 10;
            else break;
            result = result * 16.0 + d;
            has_digits = 1;
            s++;
        }
        if (*s == '.') {
            s++;
            double frac = 1.0 / 16.0;
            while (1) {
                int d;
                if (*s >= '0' && *s <= '9') d = *s - '0';
                else if (*s >= 'a' && *s <= 'f') d = *s - 'a' + 10;
                else if (*s >= 'A' && *s <= 'F') d = *s - 'A' + 10;
                else break;
                result += d * frac;
                frac /= 16.0;
                has_digits = 1;
                s++;
            }
        }
        if (!has_digits) { if (endptr) *endptr = (char *)nptr; return 0.0; }
        if (*s == 'p' || *s == 'P') {
            s++;
            int eneg = 0;
            if (*s == '-') { eneg = 1; s++; }
            else if (*s == '+') s++;
            int e = 0;
            while (*s >= '0' && *s <= '9') { e = e * 10 + (*s - '0'); s++; }
            result = ldexp(result, eneg ? -e : e);
        }
        if (endptr) *endptr = (char *)s;
        return neg ? -result : result;
    }

    /* Decimal float */
    double result = 0.0;
    int has_digits = 0;
    while (*s >= '0' && *s <= '9') {
        result = result * 10.0 + (*s - '0');
        has_digits = 1;
        s++;
    }
    if (*s == '.') {
        s++;
        double frac = 0.1;
        while (*s >= '0' && *s <= '9') {
            result += (*s - '0') * frac;
            frac *= 0.1;
            has_digits = 1;
            s++;
        }
    }
    if (!has_digits) { if (endptr) *endptr = (char *)nptr; return 0.0; }
    if (*s == 'e' || *s == 'E') {
        s++;
        int eneg = 0;
        if (*s == '-') { eneg = 1; s++; }
        else if (*s == '+') s++;
        int e = 0;
        while (*s >= '0' && *s <= '9') { e = e * 10 + (*s - '0'); s++; }
        double mul = 1.0;
        for (int i = 0; i < e; i++) mul *= 10.0;
        if (eneg) result /= mul; else result *= mul;
    }
    if (endptr) *endptr = (char *)s;
    return neg ? -result : result;
}

double strtod(const char *nptr, char **endptr) {
    return _parse_double(nptr, endptr);
}

float strtof(const char *nptr, char **endptr) {
    return (float)_parse_double(nptr, endptr);
}

long double strtold(const char *nptr, char **endptr) {
    return (long double)_parse_double(nptr, endptr);
}
