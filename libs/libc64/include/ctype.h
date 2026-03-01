/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 */

#ifndef _CTYPE_H
#define _CTYPE_H

#ifdef __cplusplus
extern "C" {
#endif

int isalpha(int c);
int isdigit(int c);
int isalnum(int c);
int isspace(int c);
int isupper(int c);
int islower(int c);
int isprint(int c);
int ispunct(int c);
int isxdigit(int c);
int iscntrl(int c);
int isgraph(int c);
int toupper(int c);
int tolower(int c);
int isascii(int c);
int isblank(int c);

#ifdef __cplusplus
}
#endif

#endif
