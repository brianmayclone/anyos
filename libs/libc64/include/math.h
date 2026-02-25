/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 */

#ifndef _MATH_H
#define _MATH_H

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

#define M_PI 3.14159265358979323846
#define M_PI_2 1.57079632679489661923
#define M_PI_4 0.78539816339744830962
#define M_E 2.71828182845904523536
#define M_LN2 0.693147180559945309417
#define M_LOG2E 1.44269504088896340736

#define HUGE_VAL (__builtin_huge_val())
#define INFINITY (__builtin_inf())
#define NAN (__builtin_nan(""))

#define isnan(x) __builtin_isnan(x)
#define isinf(x) __builtin_isinf(x)
#define isfinite(x) __builtin_isfinite(x)

#endif
