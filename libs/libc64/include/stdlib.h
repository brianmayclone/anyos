/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 */

#ifndef _STDLIB_H
#define _STDLIB_H

#include <stddef.h>

#define EXIT_SUCCESS 0
#define EXIT_FAILURE 1
#define RAND_MAX     0x7FFFFFFF

void  *malloc(size_t size);
void  *calloc(size_t nmemb, size_t size);
void  *realloc(void *ptr, size_t size);
void   free(void *ptr);
void   exit(int status);
void   abort(void);
int    atoi(const char *nptr);
long   atol(const char *nptr);
long long atoll(const char *nptr);
long   strtol(const char *nptr, char **endptr, int base);
unsigned long strtoul(const char *nptr, char **endptr, int base);
long long strtoll(const char *nptr, char **endptr, int base);
unsigned long long strtoull(const char *nptr, char **endptr, int base);
int    rand(void);
void   srand(unsigned int seed);
void   qsort(void *base, size_t nmemb, size_t size,
              int (*compar)(const void *, const void *));
void  *bsearch(const void *key, const void *base, size_t nmemb, size_t size,
               int (*compar)(const void *, const void *));
int    abs(int j);
long   labs(long j);
char  *getenv(const char *name);
int    setenv(const char *name, const char *value, int overwrite);
int    unsetenv(const char *name);
int    atexit(void (*function)(void));
int    mkstemp(char *tmpl);
char  *mkdtemp(char *tmpl);
char  *mktemp(char *tmpl);
char  *realpath(const char *path, char *resolved_path);
int    putenv(char *string);
int    clearenv(void);
double atof(const char *nptr);
double strtod(const char *nptr, char **endptr);
float  strtof(const char *nptr, char **endptr);
long double strtold(const char *nptr, char **endptr);
int    system(const char *command);

extern char **environ;

#endif
