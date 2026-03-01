/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * libc64 — x86_64 math functions.
 * Uses x87 FPU instructions (available on all x86_64 CPUs).
 */

#include <math.h>
#include <stddef.h>

double ldexp(double x, int exp) {
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

double round(double x) {
    return (x >= 0.0) ? floor(x + 0.5) : ceil(x - 0.5);
}

double trunc(double x) {
    return (double)(long)x;
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
    double t;
    __asm__ __volatile__(
        "fyl2x"
        : "=t"(t)
        : "0"(base), "u"(exponent)
        : "st(1)"
    );
    double result;
    __asm__ __volatile__(
        "fld %%st(0)\n\t"
        "frndint\n\t"
        "fxch %%st(1)\n\t"
        "fsub %%st(1), %%st(0)\n\t"
        "f2xm1\n\t"
        "fld1\n\t"
        "faddp\n\t"
        "fscale\n\t"
        "fstp %%st(1)\n\t"
        : "=t"(result)
        : "0"(t)
    );
    return result;
}

double log(double x) {
    if (x <= 0.0) return -HUGE_VAL;
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

double log10(double x) {
    return log(x) / 2.302585092994045684;
}

double exp(double x) {
    if (x == 0.0) return 1.0;
    double term = 1.0;
    double sum = 1.0;
    for (int i = 1; i < 30; i++) {
        term *= x / i;
        sum += term;
    }
    return sum;
}

/* Trigonometric functions via x87 FPU */

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
    __asm__ __volatile__(
        "fpatan"
        : "=t"(result)
        : "0"(x), "u"(y)
        : "st(1)"
    );
    return result;
}

double asin(double x) {
    return atan2(x, sqrt(1.0 - x * x));
}

double acos(double x) {
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

/* ── Hyperbolic functions ─────────────────────────────────────────── */

double sinh(double x) {
    double ep = exp(x), em = exp(-x);
    return (ep - em) * 0.5;
}

double cosh(double x) {
    double ep = exp(x), em = exp(-x);
    return (ep + em) * 0.5;
}

double tanh(double x) {
    if (x > 20.0) return 1.0;
    if (x < -20.0) return -1.0;
    double e2x = exp(2.0 * x);
    return (e2x - 1.0) / (e2x + 1.0);
}

double asinh(double x) {
    return log(x + sqrt(x * x + 1.0));
}

double acosh(double x) {
    if (x < 1.0) return NAN;
    return log(x + sqrt(x * x - 1.0));
}

double atanh(double x) {
    if (x <= -1.0 || x >= 1.0) return NAN;
    return 0.5 * log((1.0 + x) / (1.0 - x));
}

/* ── Additional math functions ─────────────────────────────────────── */

double hypot(double x, double y) {
    x = fabs(x); y = fabs(y);
    if (x < y) { double t = x; x = y; y = t; }
    if (x == 0.0) return 0.0;
    double r = y / x;
    return x * sqrt(1.0 + r * r);
}

double cbrt(double x) {
    if (x == 0.0) return 0.0;
    int neg = x < 0.0;
    if (neg) x = -x;
    double r = exp(log(x) / 3.0);
    /* Newton refinement */
    r = (2.0 * r + x / (r * r)) / 3.0;
    r = (2.0 * r + x / (r * r)) / 3.0;
    return neg ? -r : r;
}

double copysign(double x, double y) {
    double ax = fabs(x);
    return __builtin_signbit(y) ? -ax : ax;
}

double fdim(double x, double y) {
    return (x > y) ? (x - y) : 0.0;
}

double fmax(double x, double y) {
    if (__builtin_isnan(x)) return y;
    if (__builtin_isnan(y)) return x;
    return (x > y) ? x : y;
}

double fmin(double x, double y) {
    if (__builtin_isnan(x)) return y;
    if (__builtin_isnan(y)) return x;
    return (x < y) ? x : y;
}

long lround(double x) {
    return (long)round(x);
}

long lrint(double x) {
    return (long)rint(x);
}

double nearbyint(double x) {
    return rint(x);
}

double rint(double x) {
    /* Round to nearest even */
    double r = round(x);
    double d = r - x;
    if (d == 0.5 || d == -0.5) {
        long lr = (long)r;
        if (lr & 1) r = r - (d > 0 ? 1.0 : -1.0);
    }
    return r;
}

double remainder(double x, double y) {
    if (y == 0.0) return NAN;
    double n = round(x / y);
    return x - n * y;
}

double nan(const char *tag) {
    (void)tag;
    return __builtin_nan("");
}

double nextafter(double x, double y) {
    if (__builtin_isnan(x) || __builtin_isnan(y)) return NAN;
    if (x == y) return y;
    union { double d; unsigned long u; } ux = {x};
    if (x == 0.0) {
        ux.u = 1;
        return (y > 0.0) ? ux.d : -ux.d;
    }
    if ((x > 0.0) == (y > x)) ux.u++;
    else ux.u--;
    return ux.d;
}

double scalbn(double x, int n) {
    return ldexp(x, n);
}

int ilogb(double x) {
    if (x == 0.0) return (-2147483647 - 1); /* FP_ILOGB0 */
    if (__builtin_isinf(x)) return 2147483647; /* INT_MAX */
    if (__builtin_isnan(x)) return (-2147483647 - 1);
    int e;
    frexp(x, &e);
    return e - 1;
}

double logb(double x) {
    if (x == 0.0) return -HUGE_VAL;
    if (__builtin_isinf(x)) return HUGE_VAL;
    if (__builtin_isnan(x)) return NAN;
    return (double)ilogb(x);
}

double exp2(double x) {
    return pow(2.0, x);
}

double expm1(double x) {
    if (fabs(x) < 1e-10) return x + 0.5 * x * x;
    return exp(x) - 1.0;
}

double log1p(double x) {
    if (fabs(x) < 1e-10) return x - 0.5 * x * x;
    return log(1.0 + x);
}

/* ── Float variants ──────────────────────────────────────────────── */

float fabsf(float x) { return x < 0.0f ? -x : x; }
float sqrtf(float x) { return (float)sqrt((double)x); }
float sinf(float x) { return (float)sin((double)x); }
float cosf(float x) { return (float)cos((double)x); }
float tanf(float x) { return (float)tan((double)x); }
float atan2f(float y, float x) { return (float)atan2((double)y, (double)x); }
float fmodf(float x, float y) { return (float)fmod((double)x, (double)y); }
float floorf(float x) { return (float)floor((double)x); }
float ceilf(float x) { return (float)ceil((double)x); }
float roundf(float x) { return (float)round((double)x); }
float truncf(float x) { return (float)trunc((double)x); }
float powf(float x, float y) { return (float)pow((double)x, (double)y); }
float logf(float x) { return (float)log((double)x); }
float log2f(float x) { return (float)log2((double)x); }
float log10f(float x) { return (float)log10((double)x); }
float expf(float x) { return (float)exp((double)x); }
float asinf(float x) { return (float)asin((double)x); }
float acosf(float x) { return (float)acos((double)x); }
float atanf(float x) { return (float)atan((double)x); }
float sinhf(float x) { return (float)sinh((double)x); }
float coshf(float x) { return (float)cosh((double)x); }
float tanhf(float x) { return (float)tanh((double)x); }
float asinhf(float x) { return (float)asinh((double)x); }
float acoshf(float x) { return (float)acosh((double)x); }
float atanhf(float x) { return (float)atanh((double)x); }
float hypotf(float x, float y) { return (float)hypot((double)x, (double)y); }
float cbrtf(float x) { return (float)cbrt((double)x); }
float copysignf(float x, float y) { return (float)copysign((double)x, (double)y); }
float fdimf(float x, float y) { return (float)fdim((double)x, (double)y); }
float fmaxf(float x, float y) { return (float)fmax((double)x, (double)y); }
float fminf(float x, float y) { return (float)fmin((double)x, (double)y); }
long lroundf(float x) { return lround((double)x); }
long lrintf(float x) { return lrint((double)x); }
float nearbyintf(float x) { return (float)nearbyint((double)x); }
float remainderf(float x, float y) { return (float)remainder((double)x, (double)y); }
float nanf(const char *tag) { (void)tag; return __builtin_nanf(""); }
float nextafterf(float x, float y) { return (float)nextafter((double)x, (double)y); }
float scalbnf(float x, int n) { return (float)scalbn((double)x, n); }
int ilogbf(float x) { return ilogb((double)x); }
float logbf(float x) { return (float)logb((double)x); }
float rintf(float x) { return (float)rint((double)x); }
float exp2f(float x) { return (float)exp2((double)x); }
float expm1f(float x) { return (float)expm1((double)x); }
float log1pf(float x) { return (float)log1p((double)x); }

/* Floating-point parsing */

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
