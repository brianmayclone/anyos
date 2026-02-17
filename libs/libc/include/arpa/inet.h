/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * POSIX arpa/inet.h - Internet operations
 */

#ifndef _ARPA_INET_H
#define _ARPA_INET_H

#include <netinet/in.h>

/* Convert IPv4 address from text to binary (dotted-decimal notation) */
int inet_aton(const char *cp, struct in_addr *inp);

/* Convert IPv4 address from text to binary (returns in_addr_t) */
in_addr_t inet_addr(const char *cp);

/* Convert IPv4 address from binary to text */
char *inet_ntoa(struct in_addr in);

/* Convert address from text to binary (supports AF_INET and AF_INET6) */
int inet_pton(int af, const char *src, void *dst);

/* Convert address from binary to text */
const char *inet_ntop(int af, const void *src, char *dst, socklen_t size);

#endif /* _ARPA_INET_H */
