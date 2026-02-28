/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 */

#ifndef _STRING_H
#define _STRING_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

void  *memcpy(void *dest, const void *src, size_t n);
void  *memmove(void *dest, const void *src, size_t n);
void  *memset(void *s, int c, size_t n);
int    memcmp(const void *s1, const void *s2, size_t n);
void  *memchr(const void *s, int c, size_t n);
void  *memrchr(const void *s, int c, size_t n);
void  *mempcpy(void *dest, const void *src, size_t n);
size_t strlen(const char *s);
size_t strnlen(const char *s, size_t maxlen);
int    strcmp(const char *s1, const char *s2);
int    strcoll(const char *s1, const char *s2);
size_t strxfrm(char *dest, const char *src, size_t n);
int    strncmp(const char *s1, const char *s2, size_t n);
char  *strcpy(char *dest, const char *src);
char  *strncpy(char *dest, const char *src, size_t n);
char  *strcat(char *dest, const char *src);
char  *strncat(char *dest, const char *src, size_t n);
char  *strchr(const char *s, int c);
char  *strrchr(const char *s, int c);
char  *strstr(const char *haystack, const char *needle);
char  *strdup(const char *s);
char  *strndup(const char *s, size_t n);
char  *strerror(int errnum);
size_t strspn(const char *s, const char *accept);
size_t strcspn(const char *s, const char *reject);
char  *strpbrk(const char *s, const char *accept);
char  *strtok(char *str, const char *delim);
int    strcasecmp(const char *s1, const char *s2);
int    strncasecmp(const char *s1, const char *s2, size_t n);
char  *strcasestr(const char *haystack, const char *needle);
char  *strchrnul(const char *s, int c);
char  *stpcpy(char *dest, const char *src);
char  *stpncpy(char *dest, const char *src, size_t n);
char  *strsignal(int sig);
char  *strsep(char **stringp, const char *delim);
int    ffs(int i);
int    ffsl(long i);
int    ffsll(long long i);

#ifdef __cplusplus
}
#endif

#endif
