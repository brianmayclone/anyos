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
double log2(double x);
double exp(double x);

#define HUGE_VAL (__builtin_huge_val())
#define INFINITY (__builtin_inf())
#define NAN (__builtin_nan(""))

#endif
