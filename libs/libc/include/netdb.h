/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * POSIX netdb.h - Network database operations (DNS, service lookup)
 */

#ifndef _NETDB_H
#define _NETDB_H

#include <sys/socket.h>
#include <netinet/in.h>

/* Error codes for gethostbyname / getaddrinfo */
#define HOST_NOT_FOUND  1
#define TRY_AGAIN       2
#define NO_RECOVERY     3
#define NO_DATA         4
#define NO_ADDRESS      NO_DATA

/* getaddrinfo error codes */
#define EAI_AGAIN       2
#define EAI_BADFLAGS    3
#define EAI_FAIL        4
#define EAI_FAMILY      5
#define EAI_MEMORY      6
#define EAI_NONAME      8
#define EAI_SERVICE     9
#define EAI_SOCKTYPE    10
#define EAI_SYSTEM      11
#define EAI_OVERFLOW    14

/* getaddrinfo flags */
#define AI_PASSIVE      0x01
#define AI_CANONNAME    0x02
#define AI_NUMERICHOST  0x04
#define AI_NUMERICSERV  0x0400
#define AI_ADDRCONFIG   0x0020

/* getnameinfo flags */
#define NI_NUMERICHOST  0x01
#define NI_NUMERICSERV  0x02
#define NI_MAXHOST      1025
#define NI_MAXSERV      32

/* Legacy hostent structure */
struct hostent {
    char   *h_name;       /* Official name of host */
    char  **h_aliases;    /* Alias list */
    int     h_addrtype;   /* Host address type (AF_INET) */
    int     h_length;     /* Length of address */
    char  **h_addr_list;  /* List of addresses */
};

#define h_addr h_addr_list[0]  /* For backward compatibility */

/* addrinfo structure */
struct addrinfo {
    int              ai_flags;
    int              ai_family;
    int              ai_socktype;
    int              ai_protocol;
    socklen_t        ai_addrlen;
    struct sockaddr *ai_addr;
    char            *ai_canonname;
    struct addrinfo *ai_next;
};

/* Legacy DNS lookup */
struct hostent *gethostbyname(const char *name);

/* Modern DNS lookup */
int getaddrinfo(const char *node, const char *service,
                const struct addrinfo *hints,
                struct addrinfo **res);
void freeaddrinfo(struct addrinfo *res);
const char *gai_strerror(int errcode);

/* Reverse lookup */
int getnameinfo(const struct sockaddr *sa, socklen_t salen,
                char *host, socklen_t hostlen,
                char *serv, socklen_t servlen, int flags);

/* Error variable for gethostbyname */
extern int h_errno;

#endif /* _NETDB_H */
