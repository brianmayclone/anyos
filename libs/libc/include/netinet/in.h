/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * POSIX netinet/in.h - Internet address family
 */

#ifndef _NETINET_IN_H
#define _NETINET_IN_H

#include <stdint.h>
#include <sys/socket.h>

/* Internet address (IPv4) */
typedef uint32_t in_addr_t;
typedef uint16_t in_port_t;

struct in_addr {
    in_addr_t s_addr;
};

/* IPv4 socket address */
struct sockaddr_in {
    sa_family_t    sin_family;   /* AF_INET */
    in_port_t      sin_port;     /* Port number (network byte order) */
    struct in_addr sin_addr;     /* Internet address */
    unsigned char  sin_zero[8];  /* Padding to match struct sockaddr size */
};

/* IPv6 address (stub for compatibility) */
struct in6_addr {
    uint8_t s6_addr[16];
};

/* IPv6 socket address (stub for compatibility) */
struct sockaddr_in6 {
    sa_family_t     sin6_family;
    in_port_t       sin6_port;
    uint32_t        sin6_flowinfo;
    struct in6_addr sin6_addr;
    uint32_t        sin6_scope_id;
};

/* Special addresses */
#define INADDR_ANY          ((in_addr_t)0x00000000)
#define INADDR_BROADCAST    ((in_addr_t)0xFFFFFFFF)
#define INADDR_LOOPBACK     ((in_addr_t)0x7F000001)
#define INADDR_NONE         ((in_addr_t)0xFFFFFFFF)

/* Byte order conversion (x86 is little-endian) */
static inline uint16_t htons(uint16_t hostshort) {
    return (uint16_t)((hostshort >> 8) | (hostshort << 8));
}

static inline uint16_t ntohs(uint16_t netshort) {
    return htons(netshort);
}

static inline uint32_t htonl(uint32_t hostlong) {
    return ((hostlong & 0xFF000000u) >> 24) |
           ((hostlong & 0x00FF0000u) >>  8) |
           ((hostlong & 0x0000FF00u) <<  8) |
           ((hostlong & 0x000000FFu) << 24);
}

static inline uint32_t ntohl(uint32_t netlong) {
    return htonl(netlong);
}

#endif /* _NETINET_IN_H */
