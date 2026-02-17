/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 */

#ifndef _NETINET_TCP_H
#define _NETINET_TCP_H

/* TCP socket options (for setsockopt/getsockopt at IPPROTO_TCP level) */
#define TCP_NODELAY     1   /* Disable Nagle's algorithm */
#define TCP_MAXSEG      2   /* Set maximum segment size */
#define TCP_KEEPIDLE    4   /* Idle time before keepalives */
#define TCP_KEEPINTVL   5   /* Interval between keepalives */
#define TCP_KEEPCNT     6   /* Number of keepalives */
#define TCP_FASTOPEN    23  /* TCP Fast Open */

#endif /* _NETINET_TCP_H */
