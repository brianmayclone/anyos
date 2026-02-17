/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * POSIX sys/socket.h - Socket interface
 */

#ifndef _SYS_SOCKET_H
#define _SYS_SOCKET_H

#include <sys/types.h>
#include <sys/select.h>
#include <stdint.h>
#include <stddef.h>

/* Socket types */
#define SOCK_STREAM     1   /* TCP */
#define SOCK_DGRAM      2   /* UDP */
#define SOCK_RAW        3   /* Raw IP */

/* Address families */
#define AF_UNSPEC       0
#define AF_INET         2   /* IPv4 */
#define AF_INET6        10  /* IPv6 (not yet supported) */

/* Protocol families (same as AF_*) */
#define PF_UNSPEC       AF_UNSPEC
#define PF_INET         AF_INET
#define PF_INET6        AF_INET6

/* Protocol numbers */
#define IPPROTO_IP      0
#define IPPROTO_TCP     6
#define IPPROTO_UDP     17

/* Socket option levels */
#define SOL_SOCKET      1

/* Socket options */
#define SO_REUSEADDR    2
#define SO_KEEPALIVE    9
#define SO_RCVTIMEO     20
#define SO_SNDTIMEO     21
#define SO_RCVBUF       8
#define SO_SNDBUF       7
#define SO_ERROR        4
#define SO_NOSIGPIPE    0x1022  /* BSD extension */
#define SO_BROADCAST    6

/* TCP level options */
#define IPPROTO_TCP_OPT 6
#define TCP_NODELAY     1

/* Shutdown types */
#define SHUT_RD         0
#define SHUT_WR         1
#define SHUT_RDWR       2

/* send/recv flags */
#define MSG_PEEK        0x02
#define MSG_DONTWAIT    0x40
#define MSG_NOSIGNAL    0x4000

typedef unsigned int socklen_t;
typedef unsigned short sa_family_t;

/* Generic socket address */
struct sockaddr {
    sa_family_t sa_family;
    char        sa_data[14];
};

/* Socket address storage (large enough for any address type) */
struct sockaddr_storage {
    sa_family_t ss_family;
    char        __ss_pad[126];
};

/* msghdr for sendmsg/recvmsg */
struct msghdr {
    void         *msg_name;
    socklen_t     msg_namelen;
    struct iovec *msg_iov;
    int           msg_iovlen;
    void         *msg_control;
    socklen_t     msg_controllen;
    int           msg_flags;
};

struct iovec {
    void   *iov_base;
    size_t  iov_len;
};

/* Function prototypes */
int socket(int domain, int type, int protocol);
int connect(int sockfd, const struct sockaddr *addr, socklen_t addrlen);
int bind(int sockfd, const struct sockaddr *addr, socklen_t addrlen);
int listen(int sockfd, int backlog);
int accept(int sockfd, struct sockaddr *addr, socklen_t *addrlen);
ssize_t send(int sockfd, const void *buf, size_t len, int flags);
ssize_t recv(int sockfd, void *buf, size_t len, int flags);
ssize_t sendto(int sockfd, const void *buf, size_t len, int flags,
               const struct sockaddr *dest_addr, socklen_t addrlen);
ssize_t recvfrom(int sockfd, void *buf, size_t len, int flags,
                 struct sockaddr *src_addr, socklen_t *addrlen);
int setsockopt(int sockfd, int level, int optname,
               const void *optval, socklen_t optlen);
int getsockopt(int sockfd, int level, int optname,
               void *optval, socklen_t *optlen);
int shutdown(int sockfd, int how);
int getpeername(int sockfd, struct sockaddr *addr, socklen_t *addrlen);
int getsockname(int sockfd, struct sockaddr *addr, socklen_t *addrlen);

#endif /* _SYS_SOCKET_H */
