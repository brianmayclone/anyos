#ifndef _STDLIB_H
#define _STDLIB_H

#include <stddef.h>

#define EXIT_SUCCESS 0
#define EXIT_FAILURE 1

void *malloc(size_t size);
void *calloc(size_t nmemb, size_t size);
void *realloc(void *ptr, size_t size);
void free(void *ptr);
void exit(int status);
void abort(void);
int atoi(const char *nptr);
long atol(const char *nptr);
long strtol(const char *nptr, char **endptr, int base);
unsigned long strtoul(const char *nptr, char **endptr, int base);
int abs(int j);
long labs(long j);
char *getenv(const char *name);
void qsort(void *base, size_t nmemb, size_t size, int (*compar)(const void *, const void *));
int rand(void);
void srand(unsigned int seed);
long long strtoll(const char *nptr, char **endptr, int base);
unsigned long long strtoull(const char *nptr, char **endptr, int base);
double strtod(const char *nptr, char **endptr);

#define RAND_MAX 0x7FFFFFFF

#endif
