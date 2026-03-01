/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 */

#ifndef _MATH_H
#define _MATH_H

#ifdef __cplusplus
extern "C" {
#endif

double ldexp(double x, int exp);
double frexp(double x, int *exp);
double modf(double x, double *iptr);
float strtof(const char *nptr, char **endptr);
long double strtold(const char *nptr, char **endptr);
double strtod(const char *nptr, char **endptr);
double fabs(double x);
double floor(double x);
double ceil(double x);
double sqrt(double x);
double pow(double x, double y);
double log(double x);
double log10(double x);
double log2(double x);
double exp(double x);
double sin(double x);
double cos(double x);
double tan(double x);
double atan(double x);
double atan2(double y, double x);
double asin(double x);
double acos(double x);
double fmod(double x, double y);
float fabsf(float x);
float sqrtf(float x);
float sinf(float x);
float cosf(float x);
float atan2f(float y, float x);
float fmodf(float x, float y);
float floorf(float x);
float ceilf(float x);
float powf(float x, float y);
float logf(float x);
float log2f(float x);
float log10f(float x);
float expf(float x);
double round(double x);
float roundf(float x);
double trunc(double x);
float truncf(float x);
float tanf(float x);
float asinf(float x);
float acosf(float x);
float atanf(float x);

/* Hyperbolic functions */
double sinh(double x);
double cosh(double x);
double tanh(double x);
double asinh(double x);
double acosh(double x);
double atanh(double x);
float sinhf(float x);
float coshf(float x);
float tanhf(float x);
float asinhf(float x);
float acoshf(float x);
float atanhf(float x);

/* Additional math functions */
double hypot(double x, double y);
double cbrt(double x);
double copysign(double x, double y);
double fdim(double x, double y);
double fmax(double x, double y);
double fmin(double x, double y);
long lround(double x);
long lrint(double x);
double nearbyint(double x);
double remainder(double x, double y);
double nan(const char *tag);
double nextafter(double x, double y);
double scalbn(double x, int n);
int ilogb(double x);
double logb(double x);
double rint(double x);
double exp2(double x);
double expm1(double x);
double log1p(double x);
float hypotf(float x, float y);
float cbrtf(float x);
float copysignf(float x, float y);
float fdimf(float x, float y);
float fmaxf(float x, float y);
float fminf(float x, float y);
long lroundf(float x);
long lrintf(float x);
float nearbyintf(float x);
float remainderf(float x, float y);
float nanf(const char *tag);
float nextafterf(float x, float y);
float scalbnf(float x, int n);
int ilogbf(float x);
float logbf(float x);
float rintf(float x);
float exp2f(float x);
float expm1f(float x);
float log1pf(float x);

#ifdef __cplusplus
}
#endif

/* FP classification */
#define FP_NAN       0
#define FP_INFINITE  1
#define FP_ZERO      2
#define FP_SUBNORMAL 3
#define FP_NORMAL    4
#define fpclassify(x) __builtin_fpclassify(FP_NAN, FP_INFINITE, FP_NORMAL, FP_SUBNORMAL, FP_ZERO, (x))
#define signbit(x)    __builtin_signbit(x)

#define M_PI 3.14159265358979323846
#define M_PI_2 1.57079632679489661923
#define M_PI_4 0.78539816339744830962
#define M_E 2.71828182845904523536
#define M_LN2 0.693147180559945309417
#define M_LN10 2.30258509299404568402
#define M_LOG2E 1.44269504088896340736
#define M_LOG10E 0.43429448190325182765
#define M_SQRT2 1.41421356237309504880
#define M_SQRT1_2 0.70710678118654752440
#define M_1_PI 0.31830988618379067154
#define M_2_PI 0.63661977236758134308
#define M_2_SQRTPI 1.12837916709551257390

#define HUGE_VAL (__builtin_huge_val())
#define INFINITY (__builtin_inf())
#define NAN (__builtin_nan(""))

#define isnan(x) __builtin_isnan(x)
#define isinf(x) __builtin_isinf(x)
#define isfinite(x) __builtin_isfinite(x)

#endif
