/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * libc64 — x86_64 POSIX socket implementation for anyOS.
 */

#include <sys/socket.h>
#include <netinet/in.h>
#include <netdb.h>
#include <arpa/inet.h>
#include <sys/select.h>
#include <sys/time.h>
#include <time.h>
#include <poll.h>
#include <string.h>
#include <stdlib.h>
#include <errno.h>
#include <stdio.h>

#include <sys/syscall.h>

extern long _syscall(long num, long a1, long a2, long a3, long a4, long a5);

/* TCP status codes from kernel */
#define TCP_STATE_CLOSED        0
#define TCP_STATE_SYN_SENT      1
#define TCP_STATE_ESTABLISHED   2
#define TCP_STATE_FIN_WAIT1     3
#define TCP_STATE_FIN_WAIT2     4
#define TCP_STATE_TIME_WAIT     5
#define TCP_STATE_CLOSE_WAIT    6
#define TCP_STATE_LAST_ACK      7

/* =========================================================================
 * Internal socket table
 * ========================================================================= */

#define MAX_SOCKETS         16
#define SOCKET_FD_BASE      128

typedef struct {
    int      in_use;
    int      domain;
    int      type;
    int      protocol;
    int      tcp_sock_id;
    uint16_t udp_port;
    uint16_t bind_port;
    int      listening;
    struct sockaddr_in peer_addr;
    int      connected;
    int      recv_timeout_ms;
    int      send_timeout_ms;
} socket_entry_t;

static socket_entry_t socket_table[MAX_SOCKETS];
static int socket_table_init = 0;

static void ensure_init(void) {
    if (!socket_table_init) {
        memset(socket_table, 0, sizeof(socket_table));
        for (int i = 0; i < MAX_SOCKETS; i++)
            socket_table[i].tcp_sock_id = -1;
        socket_table_init = 1;
    }
}

static socket_entry_t *get_socket(int sockfd) {
    int idx = sockfd - SOCKET_FD_BASE;
    if (idx < 0 || idx >= MAX_SOCKETS) return NULL;
    if (!socket_table[idx].in_use) return NULL;
    return &socket_table[idx];
}

/* =========================================================================
 * socket()
 * ========================================================================= */

int socket(int domain, int type, int protocol) {
    ensure_init();
    if (domain != AF_INET) { errno = EAFNOSUPPORT; return -1; }
    if (type != SOCK_STREAM && type != SOCK_DGRAM) { errno = EPROTONOSUPPORT; return -1; }

    for (int i = 0; i < MAX_SOCKETS; i++) {
        if (!socket_table[i].in_use) {
            memset(&socket_table[i], 0, sizeof(socket_entry_t));
            socket_table[i].in_use = 1;
            socket_table[i].domain = domain;
            socket_table[i].type = type;
            socket_table[i].protocol = protocol;
            socket_table[i].tcp_sock_id = -1;
            socket_table[i].recv_timeout_ms = 30000;
            socket_table[i].send_timeout_ms = 10000;
            return i + SOCKET_FD_BASE;
        }
    }
    errno = EMFILE;
    return -1;
}

/* =========================================================================
 * connect()
 * ========================================================================= */

int connect(int sockfd, const struct sockaddr *addr, socklen_t addrlen) {
    (void)addrlen;
    socket_entry_t *s = get_socket(sockfd);
    if (!s) { errno = EBADF; return -1; }

    if (s->type == SOCK_STREAM) {
        const struct sockaddr_in *sin = (const struct sockaddr_in *)addr;
        struct {
            uint8_t  ip[4];
            uint16_t port;
            uint16_t pad;
            uint32_t timeout;
        } __attribute__((packed)) params;

        uint32_t addr_n = sin->sin_addr.s_addr;
        params.ip[0] = (addr_n >>  0) & 0xFF;
        params.ip[1] = (addr_n >>  8) & 0xFF;
        params.ip[2] = (addr_n >> 16) & 0xFF;
        params.ip[3] = (addr_n >> 24) & 0xFF;
        params.port = ntohs(sin->sin_port);
        params.pad = 0;
        params.timeout = (uint32_t)s->send_timeout_ms;

        long result = _syscall(SYS_TCP_CONNECT, (long)&params, 0, 0, 0, 0);
        if (result == -1L || result == (long)0xFFFFFFFFu) {
            errno = ECONNREFUSED;
            return -1;
        }
        s->tcp_sock_id = (int)result;
        s->connected = 1;
        memcpy(&s->peer_addr, sin, sizeof(struct sockaddr_in));
        return 0;
    } else if (s->type == SOCK_DGRAM) {
        const struct sockaddr_in *sin = (const struct sockaddr_in *)addr;
        memcpy(&s->peer_addr, sin, sizeof(struct sockaddr_in));
        s->connected = 1;
        return 0;
    }
    errno = EOPNOTSUPP;
    return -1;
}

/* =========================================================================
 * bind()
 * ========================================================================= */

int bind(int sockfd, const struct sockaddr *addr, socklen_t addrlen) {
    (void)addrlen;
    socket_entry_t *s = get_socket(sockfd);
    if (!s) { errno = EBADF; return -1; }

    if (s->type == SOCK_DGRAM) {
        const struct sockaddr_in *sin = (const struct sockaddr_in *)addr;
        uint16_t port = ntohs(sin->sin_port);
        long result = _syscall(SYS_UDP_BIND, (long)port, 0, 0, 0, 0);
        if (result == (long)0xFFFFFFFFu) { errno = EADDRINUSE; return -1; }
        s->udp_port = port;
        return 0;
    }
    if (s->type == SOCK_STREAM) {
        const struct sockaddr_in *sin = (const struct sockaddr_in *)addr;
        s->bind_port = ntohs(sin->sin_port);
        return 0;
    }
    errno = EOPNOTSUPP;
    return -1;
}

/* =========================================================================
 * listen() / accept()
 * ========================================================================= */

int listen(int sockfd, int backlog) {
    socket_entry_t *s = get_socket(sockfd);
    if (!s) { errno = EBADF; return -1; }
    if (s->type != SOCK_STREAM) { errno = EOPNOTSUPP; return -1; }
    if (s->bind_port == 0) { errno = EINVAL; return -1; }

    long result = _syscall(SYS_TCP_LISTEN, (long)s->bind_port, backlog > 0 ? backlog : 5, 0, 0, 0);
    if (result == (long)0xFFFFFFFFu) { errno = EADDRINUSE; return -1; }
    s->tcp_sock_id = (int)result;
    s->listening = 1;
    return 0;
}

int accept(int sockfd, struct sockaddr *addr, socklen_t *addrlen) {
    socket_entry_t *s = get_socket(sockfd);
    if (!s) { errno = EBADF; return -1; }
    if (!s->listening || s->tcp_sock_id < 0) { errno = EINVAL; return -1; }

    uint8_t result_buf[12];
    long rc = _syscall(SYS_TCP_ACCEPT, s->tcp_sock_id, (long)result_buf, 0, 0, 0);
    if (rc == (long)0xFFFFFFFFu) { errno = EAGAIN; return -1; }

    uint32_t new_sock_id = *(uint32_t *)&result_buf[0];
    uint16_t remote_port = *(uint16_t *)&result_buf[8];

    int new_fd = -1;
    for (int i = 0; i < MAX_SOCKETS; i++) {
        if (!socket_table[i].in_use) {
            memset(&socket_table[i], 0, sizeof(socket_entry_t));
            socket_table[i].in_use = 1;
            socket_table[i].domain = AF_INET;
            socket_table[i].type = SOCK_STREAM;
            socket_table[i].tcp_sock_id = (int)new_sock_id;
            socket_table[i].connected = 1;
            socket_table[i].recv_timeout_ms = 30000;
            socket_table[i].send_timeout_ms = 10000;
            socket_table[i].peer_addr.sin_family = AF_INET;
            socket_table[i].peer_addr.sin_port = htons(remote_port);
            memcpy(&socket_table[i].peer_addr.sin_addr.s_addr, &result_buf[4], 4);
            new_fd = i + SOCKET_FD_BASE;
            break;
        }
    }
    if (new_fd < 0) {
        _syscall(SYS_TCP_CLOSE, (long)new_sock_id, 0, 0, 0, 0);
        errno = EMFILE;
        return -1;
    }
    if (addr && addrlen) {
        struct sockaddr_in sin;
        sin.sin_family = AF_INET;
        sin.sin_port = htons(remote_port);
        memcpy(&sin.sin_addr.s_addr, &result_buf[4], 4);
        socklen_t copylen = sizeof(sin);
        if (*addrlen < copylen) copylen = *addrlen;
        memcpy(addr, &sin, copylen);
        *addrlen = sizeof(sin);
    }
    return new_fd;
}

/* =========================================================================
 * send() / recv()
 * ========================================================================= */

ssize_t send(int sockfd, const void *buf, size_t len, int flags) {
    (void)flags;
    socket_entry_t *s = get_socket(sockfd);
    if (!s) { errno = EBADF; return -1; }
    if (s->type == SOCK_STREAM) {
        if (s->tcp_sock_id < 0) { errno = ENOTCONN; return -1; }
        long result = _syscall(SYS_TCP_SEND, s->tcp_sock_id, (long)buf, (long)len, 0, 0);
        if (result == (long)0xFFFFFFFFu) { errno = EPIPE; return -1; }
        return (ssize_t)result;
    }
    errno = EOPNOTSUPP;
    return -1;
}

ssize_t recv(int sockfd, void *buf, size_t len, int flags) {
    (void)flags;
    socket_entry_t *s = get_socket(sockfd);
    if (!s) { errno = EBADF; return -1; }
    if (s->type == SOCK_STREAM) {
        if (s->tcp_sock_id < 0) { errno = ENOTCONN; return -1; }
        long result = _syscall(SYS_TCP_RECV, s->tcp_sock_id, (long)buf, (long)len, 0, 0);
        if (result == (long)0xFFFFFFFFu) { errno = ETIMEDOUT; return -1; }
        return (ssize_t)result;
    }
    errno = EOPNOTSUPP;
    return -1;
}

ssize_t sendto(int sockfd, const void *buf, size_t len, int flags,
               const struct sockaddr *dest_addr, socklen_t addrlen) {
    (void)flags; (void)addrlen;
    socket_entry_t *s = get_socket(sockfd);
    if (!s) { errno = EBADF; return -1; }

    if (s->type == SOCK_DGRAM) {
        const struct sockaddr_in *sin = (const struct sockaddr_in *)dest_addr;
        uint32_t addr_n = sin->sin_addr.s_addr;
        struct {
            uint8_t  dst_ip[4];
            uint16_t dst_port;
            uint16_t src_port;
            unsigned long data_ptr;
            unsigned long data_len;
            unsigned long flags;
        } __attribute__((packed)) params;

        params.dst_ip[0] = (addr_n >>  0) & 0xFF;
        params.dst_ip[1] = (addr_n >>  8) & 0xFF;
        params.dst_ip[2] = (addr_n >> 16) & 0xFF;
        params.dst_ip[3] = (addr_n >> 24) & 0xFF;
        params.dst_port = ntohs(sin->sin_port);
        params.src_port = s->udp_port;
        params.data_ptr = (unsigned long)buf;
        params.data_len = (unsigned long)len;
        params.flags = 0;

        long result = _syscall(SYS_UDP_SENDTO, (long)&params, 0, 0, 0, 0);
        if (result == (long)0xFFFFFFFFu) { errno = ENETUNREACH; return -1; }
        return (ssize_t)result;
    }
    if (s->type == SOCK_STREAM) return send(sockfd, buf, len, flags);
    errno = EOPNOTSUPP;
    return -1;
}

ssize_t recvfrom(int sockfd, void *buf, size_t len, int flags,
                 struct sockaddr *src_addr, socklen_t *addrlen) {
    (void)flags;
    socket_entry_t *s = get_socket(sockfd);
    if (!s) { errno = EBADF; return -1; }

    if (s->type == SOCK_DGRAM) {
        if (s->udp_port == 0) { errno = ENOTCONN; return -1; }
        size_t total_len = 8 + len;
        uint8_t *tmp = (uint8_t *)malloc(total_len);
        if (!tmp) { errno = ENOMEM; return -1; }

        long result = _syscall(SYS_UDP_RECVFROM, (long)s->udp_port, (long)tmp, (long)total_len, 0, 0);
        if (result == 0 || result == (long)0xFFFFFFFFu) {
            free(tmp);
            if (result == 0) return 0;
            errno = ETIMEDOUT;
            return -1;
        }
        uint16_t payload_len = (uint16_t)(tmp[6] | (tmp[7] << 8));
        size_t copy_len = payload_len < len ? payload_len : len;
        memcpy(buf, tmp + 8, copy_len);

        if (src_addr && addrlen && *addrlen >= sizeof(struct sockaddr_in)) {
            struct sockaddr_in *sin = (struct sockaddr_in *)src_addr;
            sin->sin_family = AF_INET;
            memcpy(&sin->sin_addr.s_addr, tmp, 4);
            sin->sin_port = htons((uint16_t)(tmp[4] | (tmp[5] << 8)));
            *addrlen = sizeof(struct sockaddr_in);
        }
        free(tmp);
        return (ssize_t)copy_len;
    }
    if (s->type == SOCK_STREAM) return recv(sockfd, buf, len, flags);
    errno = EOPNOTSUPP;
    return -1;
}

/* =========================================================================
 * close() wrapper for sockets
 * ========================================================================= */

int __socket_close(int sockfd) {
    socket_entry_t *s = get_socket(sockfd);
    if (!s) return -1;
    if (s->type == SOCK_STREAM && s->tcp_sock_id >= 0)
        _syscall(SYS_TCP_CLOSE, s->tcp_sock_id, 0, 0, 0, 0);
    if (s->type == SOCK_DGRAM && s->udp_port > 0)
        _syscall(SYS_UDP_UNBIND, (long)s->udp_port, 0, 0, 0, 0);
    s->in_use = 0;
    s->tcp_sock_id = -1;
    s->udp_port = 0;
    s->connected = 0;
    return 0;
}

/* =========================================================================
 * setsockopt() / getsockopt()
 * ========================================================================= */

int setsockopt(int sockfd, int level, int optname,
               const void *optval, socklen_t optlen) {
    socket_entry_t *s = get_socket(sockfd);
    if (!s) { errno = EBADF; return -1; }
    if (level == SOL_SOCKET && optname == SO_RCVTIMEO && optlen >= sizeof(struct timeval)) {
        const struct timeval *tv = (const struct timeval *)optval;
        s->recv_timeout_ms = (int)(tv->tv_sec * 1000 + tv->tv_usec / 1000);
        return 0;
    }
    if (level == SOL_SOCKET && optname == SO_SNDTIMEO && optlen >= sizeof(struct timeval)) {
        const struct timeval *tv = (const struct timeval *)optval;
        s->send_timeout_ms = (int)(tv->tv_sec * 1000 + tv->tv_usec / 1000);
        return 0;
    }
    return 0; /* silently accept other options */
}

int getsockopt(int sockfd, int level, int optname,
               void *optval, socklen_t *optlen) {
    socket_entry_t *s = get_socket(sockfd);
    if (!s) { errno = EBADF; return -1; }
    if (level == SOL_SOCKET && optname == SO_ERROR) {
        if (optval && optlen && *optlen >= sizeof(int)) {
            int err = 0;
            if (s->type == SOCK_STREAM && s->tcp_sock_id >= 0) {
                long st = _syscall(SYS_TCP_STATUS, s->tcp_sock_id, 0, 0, 0, 0);
                if (st == TCP_STATE_CLOSED || st == (long)0xFFFFFFFFu) err = ECONNRESET;
            }
            *(int *)optval = err;
            *optlen = sizeof(int);
        }
        return 0;
    }
    if (optval && optlen && *optlen >= sizeof(int)) {
        *(int *)optval = 0;
        *optlen = sizeof(int);
    }
    return 0;
}

/* =========================================================================
 * shutdown() / getpeername() / getsockname()
 * ========================================================================= */

int shutdown(int sockfd, int how) {
    socket_entry_t *s = get_socket(sockfd);
    if (!s) { errno = EBADF; return -1; }
    if (s->type == SOCK_STREAM && s->tcp_sock_id >= 0) {
        if (how == SHUT_RDWR) {
            _syscall(SYS_TCP_CLOSE, s->tcp_sock_id, 0, 0, 0, 0);
            s->tcp_sock_id = -1;
            s->connected = 0;
        } else if (how == SHUT_WR) {
            _syscall(SYS_TCP_SHUTDOWN_WR, s->tcp_sock_id, 0, 0, 0, 0);
        }
    }
    return 0;
}

int getpeername(int sockfd, struct sockaddr *addr, socklen_t *addrlen) {
    socket_entry_t *s = get_socket(sockfd);
    if (!s) { errno = EBADF; return -1; }
    if (!s->connected) { errno = ENOTCONN; return -1; }
    if (addr && addrlen && *addrlen >= sizeof(struct sockaddr_in)) {
        memcpy(addr, &s->peer_addr, sizeof(struct sockaddr_in));
        *addrlen = sizeof(struct sockaddr_in);
    }
    return 0;
}

int getsockname(int sockfd, struct sockaddr *addr, socklen_t *addrlen) {
    socket_entry_t *s = get_socket(sockfd);
    if (!s) { errno = EBADF; return -1; }
    if (addr && addrlen && *addrlen >= sizeof(struct sockaddr_in)) {
        struct sockaddr_in *sin = (struct sockaddr_in *)addr;
        sin->sin_family = AF_INET;
        sin->sin_port = htons(s->udp_port);
        sin->sin_addr.s_addr = INADDR_ANY;
        *addrlen = sizeof(struct sockaddr_in);
    }
    return 0;
}

/* =========================================================================
 * select() / poll()
 * ========================================================================= */

static int __select_check(int nfds, fd_set *readfds, fd_set *writefds,
                          fd_set *exceptfds,
                          fd_set *rd_result, fd_set *wr_result, fd_set *ex_result) {
    int ready = 0;
    _syscall(SYS_NET_POLL, 0, 0, 0, 0, 0);

    for (int fd = 0; fd < nfds && fd < FD_SETSIZE; fd++) {
        socket_entry_t *s = get_socket(fd);
        if (!s) continue;
        if (s->type == SOCK_STREAM && s->tcp_sock_id >= 0) {
            long st = _syscall(SYS_TCP_STATUS, s->tcp_sock_id, 0, 0, 0, 0);
            if (readfds && FD_ISSET(fd, readfds)) {
                long avail = _syscall(SYS_TCP_RECV_AVAILABLE, s->tcp_sock_id, 0, 0, 0, 0);
                if (avail > 0 || avail == (long)0xFFFFFFFEu || avail == (long)0xFFFFFFFFu) {
                    FD_SET(fd, rd_result);
                    ready++;
                }
            }
            if (writefds && FD_ISSET(fd, writefds)) {
                if (st == TCP_STATE_ESTABLISHED) { FD_SET(fd, wr_result); ready++; }
            }
            if (exceptfds && FD_ISSET(fd, exceptfds)) {
                if (st == (long)0xFFFFFFFFu) { FD_SET(fd, ex_result); ready++; }
            }
        } else if (s->type == SOCK_DGRAM) {
            if (readfds && FD_ISSET(fd, readfds)) { FD_SET(fd, rd_result); ready++; }
            if (writefds && FD_ISSET(fd, writefds)) { FD_SET(fd, wr_result); ready++; }
        }
    }
    return ready;
}

int select(int nfds, fd_set *readfds, fd_set *writefds,
           fd_set *exceptfds, struct timeval *timeout) {
    fd_set rd_result, wr_result, ex_result;
    long timeout_ms = -1;
    if (timeout) timeout_ms = timeout->tv_sec * 1000 + timeout->tv_usec / 1000;

    long elapsed = 0;
    while (1) {
        FD_ZERO(&rd_result);
        FD_ZERO(&wr_result);
        FD_ZERO(&ex_result);
        int ready = __select_check(nfds, readfds, writefds, exceptfds,
                                   &rd_result, &wr_result, &ex_result);
        if (ready > 0 || timeout_ms == 0) {
            if (readfds)   memcpy(readfds, &rd_result, sizeof(fd_set));
            if (writefds)  memcpy(writefds, &wr_result, sizeof(fd_set));
            if (exceptfds) memcpy(exceptfds, &ex_result, sizeof(fd_set));
            return ready;
        }
        if (timeout_ms > 0 && elapsed >= timeout_ms) {
            if (readfds)   memcpy(readfds, &rd_result, sizeof(fd_set));
            if (writefds)  memcpy(writefds, &wr_result, sizeof(fd_set));
            if (exceptfds) memcpy(exceptfds, &ex_result, sizeof(fd_set));
            return 0;
        }
        int sleep_ms = 10;
        if (timeout_ms > 0) {
            long remaining = timeout_ms - elapsed;
            if (remaining < sleep_ms) sleep_ms = (int)remaining;
        }
        _syscall(SYS_SLEEP, sleep_ms, 0, 0, 0, 0);
        elapsed += sleep_ms;
    }
}

int pselect(int nfds, fd_set *readfds, fd_set *writefds,
            fd_set *exceptfds, const struct timespec *timeout,
            const void *sigmask) {
    (void)sigmask;
    struct timeval tv;
    struct timeval *tvp = NULL;
    if (timeout) {
        tv.tv_sec = timeout->tv_sec;
        tv.tv_usec = timeout->tv_nsec / 1000;
        tvp = &tv;
    }
    return select(nfds, readfds, writefds, exceptfds, tvp);
}

int poll(struct pollfd *fds, nfds_t nfds, int timeout) {
    long elapsed = 0;
    while (1) {
        int ready = 0;
        _syscall(SYS_NET_POLL, 0, 0, 0, 0, 0);
        for (nfds_t i = 0; i < nfds; i++) {
            fds[i].revents = 0;
            socket_entry_t *s = get_socket(fds[i].fd);
            if (!s) { fds[i].revents = POLLNVAL; continue; }
            if (s->type == SOCK_STREAM && s->tcp_sock_id >= 0) {
                long st = _syscall(SYS_TCP_STATUS, s->tcp_sock_id, 0, 0, 0, 0);
                if (fds[i].events & POLLIN) {
                    long avail = _syscall(SYS_TCP_RECV_AVAILABLE, s->tcp_sock_id, 0, 0, 0, 0);
                    if (avail > 0 || avail == (long)0xFFFFFFFEu || avail == (long)0xFFFFFFFFu)
                        fds[i].revents |= POLLIN;
                }
                if (fds[i].events & POLLOUT) {
                    if (st == TCP_STATE_ESTABLISHED) fds[i].revents |= POLLOUT;
                }
                if (st == (long)0xFFFFFFFFu) fds[i].revents |= POLLERR;
            } else if (s->type == SOCK_DGRAM) {
                if (fds[i].events & POLLIN) fds[i].revents |= POLLIN;
                if (fds[i].events & POLLOUT) fds[i].revents |= POLLOUT;
            }
            if (fds[i].revents) ready++;
        }
        if (ready > 0 || timeout == 0) return ready;
        if (timeout > 0 && elapsed >= timeout) return 0;
        int sleep_ms = 10;
        if (timeout > 0) {
            long remaining = timeout - elapsed;
            if (remaining < sleep_ms) sleep_ms = (int)remaining;
        }
        _syscall(SYS_SLEEP, sleep_ms, 0, 0, 0, 0);
        elapsed += sleep_ms;
    }
}

/* =========================================================================
 * DNS / gethostbyname() / getaddrinfo()
 * ========================================================================= */

int h_errno = 0;

static struct hostent __hostent;
static char *__h_aliases[] = { NULL };
static char *__h_addr_list[2] = { NULL, NULL };
static char __h_addr_buf[4];
static char __h_name_buf[256];

struct hostent *gethostbyname(const char *name) {
    if (!name) { h_errno = HOST_NOT_FOUND; return NULL; }

    struct in_addr addr;
    if (inet_aton(name, &addr)) {
        memcpy(__h_addr_buf, &addr.s_addr, 4);
        __h_addr_list[0] = __h_addr_buf;
        __h_addr_list[1] = NULL;
        size_t nlen = strlen(name);
        if (nlen >= sizeof(__h_name_buf)) nlen = sizeof(__h_name_buf) - 1;
        memcpy(__h_name_buf, name, nlen);
        __h_name_buf[nlen] = '\0';
        __hostent.h_name = __h_name_buf;
        __hostent.h_aliases = __h_aliases;
        __hostent.h_addrtype = AF_INET;
        __hostent.h_length = 4;
        __hostent.h_addr_list = __h_addr_list;
        return &__hostent;
    }

    uint8_t ip[4];
    long result = _syscall(SYS_NET_DNS, (long)name, (long)ip, 0, 0, 0);
    if (result != 0) { h_errno = HOST_NOT_FOUND; return NULL; }

    __h_addr_buf[0] = (char)ip[0];
    __h_addr_buf[1] = (char)ip[1];
    __h_addr_buf[2] = (char)ip[2];
    __h_addr_buf[3] = (char)ip[3];
    __h_addr_list[0] = __h_addr_buf;
    __h_addr_list[1] = NULL;

    size_t nlen = strlen(name);
    if (nlen >= sizeof(__h_name_buf)) nlen = sizeof(__h_name_buf) - 1;
    memcpy(__h_name_buf, name, nlen);
    __h_name_buf[nlen] = '\0';

    __hostent.h_name = __h_name_buf;
    __hostent.h_aliases = __h_aliases;
    __hostent.h_addrtype = AF_INET;
    __hostent.h_length = 4;
    __hostent.h_addr_list = __h_addr_list;
    return &__hostent;
}

int getaddrinfo(const char *node, const char *service,
                const struct addrinfo *hints, struct addrinfo **res) {
    if (!node && !service) return EAI_NONAME;
    if (!res) return EAI_FAIL;

    int family = hints ? hints->ai_family : AF_UNSPEC;
    int socktype = hints ? hints->ai_socktype : 0;
    int protocol = hints ? hints->ai_protocol : 0;
    (void)family;

    if (socktype == 0) socktype = SOCK_STREAM;
    if (protocol == 0 && socktype == SOCK_STREAM) protocol = IPPROTO_TCP;
    if (protocol == 0 && socktype == SOCK_DGRAM) protocol = IPPROTO_UDP;

    struct in_addr addr;
    addr.s_addr = INADDR_ANY;

    if (node) {
        if (!inet_aton(node, &addr)) {
            uint8_t ip[4];
            long r = _syscall(SYS_NET_DNS, (long)node, (long)ip, 0, 0, 0);
            if (r != 0) return EAI_NONAME;
            memcpy(&addr.s_addr, ip, 4);
        }
    } else if (hints && (hints->ai_flags & AI_PASSIVE)) {
        addr.s_addr = INADDR_ANY;
    } else {
        addr.s_addr = htonl(INADDR_LOOPBACK);
    }

    uint16_t port = 0;
    if (service) {
        port = (uint16_t)atoi(service);
        if (port == 0) {
            if (strcmp(service, "http") == 0) port = 80;
            else if (strcmp(service, "https") == 0) port = 443;
            else if (strcmp(service, "ftp") == 0) port = 21;
            else if (strcmp(service, "ssh") == 0) port = 22;
            else if (strcmp(service, "dns") == 0) port = 53;
            else return EAI_SERVICE;
        }
    }

    struct addrinfo *ai = (struct addrinfo *)calloc(1, sizeof(struct addrinfo) + sizeof(struct sockaddr_in));
    if (!ai) return EAI_MEMORY;

    struct sockaddr_in *sin = (struct sockaddr_in *)((char *)ai + sizeof(struct addrinfo));
    sin->sin_family = AF_INET;
    sin->sin_port = htons(port);
    sin->sin_addr = addr;

    ai->ai_flags = hints ? hints->ai_flags : 0;
    ai->ai_family = AF_INET;
    ai->ai_socktype = socktype;
    ai->ai_protocol = protocol;
    ai->ai_addrlen = sizeof(struct sockaddr_in);
    ai->ai_addr = (struct sockaddr *)sin;
    ai->ai_canonname = NULL;
    ai->ai_next = NULL;

    *res = ai;
    return 0;
}

void freeaddrinfo(struct addrinfo *res) {
    while (res) {
        struct addrinfo *next = res->ai_next;
        free(res);
        res = next;
    }
}

const char *gai_strerror(int errcode) {
    switch (errcode) {
    case 0:             return "Success";
    case EAI_AGAIN:     return "Temporary failure in name resolution";
    case EAI_BADFLAGS:  return "Invalid flags";
    case EAI_FAIL:      return "Non-recoverable failure";
    case EAI_FAMILY:    return "Address family not supported";
    case EAI_MEMORY:    return "Memory allocation failure";
    case EAI_NONAME:    return "Name or service not known";
    case EAI_SERVICE:   return "Service not supported";
    case EAI_SOCKTYPE:  return "Socket type not supported";
    case EAI_SYSTEM:    return "System error";
    default:            return "Unknown error";
    }
}

int getnameinfo(const struct sockaddr *sa, socklen_t salen,
                char *host, socklen_t hostlen,
                char *serv, socklen_t servlen, int flags) {
    (void)salen; (void)flags;
    if (sa->sa_family != AF_INET) return EAI_FAMILY;
    const struct sockaddr_in *sin = (const struct sockaddr_in *)sa;
    if (host && hostlen > 0)
        inet_ntop(AF_INET, &sin->sin_addr, host, hostlen);
    if (serv && servlen > 0)
        snprintf(serv, servlen, "%u", ntohs(sin->sin_port));
    return 0;
}

/* =========================================================================
 * inet_aton() / inet_addr() / inet_ntoa() / inet_pton() / inet_ntop()
 * ========================================================================= */

int inet_aton(const char *cp, struct in_addr *inp) {
    unsigned int a, b, c, d;
    int n = 0;
    const char *p = cp;

    a = 0;
    while (*p >= '0' && *p <= '9') { a = a * 10 + (*p - '0'); p++; n++; }
    if (*p != '.' || a > 255 || n == 0) return 0;
    p++; n = 0;

    b = 0;
    while (*p >= '0' && *p <= '9') { b = b * 10 + (*p - '0'); p++; n++; }
    if (*p != '.' || b > 255 || n == 0) return 0;
    p++; n = 0;

    c = 0;
    while (*p >= '0' && *p <= '9') { c = c * 10 + (*p - '0'); p++; n++; }
    if (*p != '.' || c > 255 || n == 0) return 0;
    p++; n = 0;

    d = 0;
    while (*p >= '0' && *p <= '9') { d = d * 10 + (*p - '0'); p++; n++; }
    if (d > 255 || n == 0) return 0;

    if (inp)
        inp->s_addr = (in_addr_t)(a | (b << 8) | (c << 16) | (d << 24));
    return 1;
}

in_addr_t inet_addr(const char *cp) {
    struct in_addr addr;
    if (inet_aton(cp, &addr)) return addr.s_addr;
    return INADDR_NONE;
}

static char __inet_ntoa_buf[16];

char *inet_ntoa(struct in_addr in) {
    uint32_t a = in.s_addr;
    snprintf(__inet_ntoa_buf, sizeof(__inet_ntoa_buf), "%u.%u.%u.%u",
             a & 0xFF, (a >> 8) & 0xFF, (a >> 16) & 0xFF, (a >> 24) & 0xFF);
    return __inet_ntoa_buf;
}

int inet_pton(int af, const char *src, void *dst) {
    if (af == AF_INET) {
        struct in_addr addr;
        if (inet_aton(src, &addr)) {
            memcpy(dst, &addr.s_addr, 4);
            return 1;
        }
        return 0;
    }
    errno = EAFNOSUPPORT;
    return -1;
}

const char *inet_ntop(int af, const void *src, char *dst, unsigned int size) {
    if (af == AF_INET) {
        const uint8_t *p = (const uint8_t *)src;
        int n = snprintf(dst, size, "%u.%u.%u.%u", p[0], p[1], p[2], p[3]);
        if (n < 0 || (unsigned int)n >= size) { errno = ENOSPC; return NULL; }
        return dst;
    }
    errno = EAFNOSUPPORT;
    return NULL;
}
