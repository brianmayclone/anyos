/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 */

#ifndef _REGEX_H
#define _REGEX_H

#include <stddef.h>

#define REG_EXTENDED 1
#define REG_ICASE    2
#define REG_NOSUB    4
#define REG_NEWLINE  8

#define REG_NOMATCH  1
#define REG_BADPAT   2
#define REG_ESPACE   12

typedef struct {
    size_t re_nsub;
    void  *__data;
} regex_t;

typedef struct {
    int rm_so;
    int rm_eo;
} regmatch_t;

#ifdef __cplusplus
extern "C" {
#endif

int regcomp(regex_t *preg, const char *regex, int cflags);
int regexec(const regex_t *preg, const char *string, size_t nmatch,
            regmatch_t pmatch[], int eflags);
void regfree(regex_t *preg);
size_t regerror(int errcode, const regex_t *preg, char *errbuf, size_t errbuf_size);

#ifdef __cplusplus
}
#endif

#endif
