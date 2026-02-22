/*
 * ssh — anyOS SSH client
 *
 * Usage: ssh user@host [-p port]
 *        ssh host -l user [-p port]
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "ssh.h"

extern int _syscall(int num, int a1, int a2, int a3, int a4);

#define SYS_WRITE          2
#define SYS_READ           3
#define SYS_YIELD          7
#define SYS_SLEEP          8
#define SYS_NET_DNS        43
#define SYS_NET_POLL       50
#define SYS_TCP_CONNECT    100
#define SYS_TCP_CLOSE      103
#define SYS_TCP_STATUS     104

/* TCP status codes (must match kernel TcpState) */
#define TCP_STATE_ESTABLISHED 4

/* Parse user@host from argument */
static int parse_target(const char *arg, char *user, int user_len,
                        char *host, int host_len)
{
    const char *at = strchr(arg, '@');
    if (at) {
        int ulen = at - arg;
        if (ulen >= user_len) ulen = user_len - 1;
        memcpy(user, arg, ulen);
        user[ulen] = '\0';
        strncpy(host, at + 1, host_len - 1);
        host[host_len - 1] = '\0';
        return 0;
    }
    /* No @ sign — just a hostname, no user extracted */
    user[0] = '\0';
    strncpy(host, arg, host_len - 1);
    host[host_len - 1] = '\0';
    return 0;
}

/* Read a line from stdin (blocking) */
static int read_line(char *buf, int len)
{
    int pos = 0;
    while (pos < len - 1) {
        char c;
        int r = _syscall(SYS_READ, 0, (int)&c, 1, 0);
        if (r <= 0) {
            _syscall(SYS_SLEEP, 10, 0, 0, 0);
            continue;
        }
        if (c == '\n' || c == '\r') break;
        buf[pos++] = c;
    }
    buf[pos] = '\0';
    return pos;
}

/* TCP connect to IP:port, returns socket id or 0xFFFFFFFF on error */
static int tcp_connect(const unsigned char *ip, int port)
{
    unsigned char params[12];
    memcpy(params, ip, 4);
    params[4] = (port >> 8) & 0xFF;
    params[5] = port & 0xFF;
    params[6] = 0;
    params[7] = 0;
    /* Timeout: 10 seconds in ms, little-endian u32 */
    unsigned int timeout = 10000;
    memcpy(params + 8, &timeout, 4);
    return _syscall(SYS_TCP_CONNECT, (int)params, 0, 0, 0);
}

int main(int argc, char **argv)
{
    char user[64] = "";
    char host[128] = "";
    int port = 22;

    /* Parse arguments */
    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], "-p") == 0 && i + 1 < argc) {
            port = atoi(argv[++i]);
        } else if (strcmp(argv[i], "-l") == 0 && i + 1 < argc) {
            strncpy(user, argv[++i], sizeof(user) - 1);
        } else if (host[0] == '\0') {
            parse_target(argv[i], user, sizeof(user), host, sizeof(host));
        }
    }

    if (host[0] == '\0') {
        printf("Usage: ssh user@host [-p port]\n");
        printf("       ssh host -l user [-p port]\n");
        return 1;
    }

    /* Prompt for username if not provided */
    if (user[0] == '\0') {
        printf("Username: ");
        read_line(user, sizeof(user));
    }

    /* Resolve hostname to IP */
    unsigned char ip[4];
    int a, b, c, d;
    if (sscanf(host, "%d.%d.%d.%d", &a, &b, &c, &d) == 4) {
        ip[0] = a; ip[1] = b; ip[2] = c; ip[3] = d;
    } else {
        printf("Resolving %s...\n", host);
        int rc = _syscall(SYS_NET_DNS, (int)host, (int)ip, 0, 0);
        if (rc != 0) {
            printf("ssh: could not resolve '%s'\n", host);
            return 1;
        }
    }

    printf("Connecting to %d.%d.%d.%d:%d...\n", ip[0], ip[1], ip[2], ip[3], port);

    /* TCP connect */
    int sock = tcp_connect(ip, port);
    if (sock < 0 || sock == (int)0xFFFFFFFF) {
        printf("ssh: connection failed\n");
        return 1;
    }

    /* Wait for TCP connection to establish */
    for (int tries = 0; tries < 100; tries++) {
        _syscall(SYS_NET_POLL, 0, 0, 0, 0);
        int st = _syscall(SYS_TCP_STATUS, sock, 0, 0, 0);
        if (st == TCP_STATE_ESTABLISHED) break;
        _syscall(SYS_SLEEP, 100, 0, 0, 0);
    }
    if (_syscall(SYS_TCP_STATUS, sock, 0, 0, 0) != TCP_STATE_ESTABLISHED) {
        printf("ssh: connection timed out\n");
        _syscall(SYS_TCP_CLOSE, sock, 0, 0, 0);
        return 1;
    }

    printf("Connected.\n");

    /* SSH protocol handshake */
    ssh_ctx_t ctx;
    ssh_init(&ctx, sock, 0);

    int rc = ssh_version_exchange(&ctx);
    if (rc != SSH_OK) {
        printf("ssh: version exchange failed (%d)\n", rc);
        goto cleanup;
    }
    printf("Server: %s\n", ctx.server_version);

    rc = ssh_kex(&ctx);
    if (rc != SSH_OK) {
        printf("ssh: key exchange failed (%d)\n", rc);
        goto cleanup;
    }

    /* Password authentication */
    char password[128];
    printf("Password: ");
    read_line(password, sizeof(password));

    rc = ssh_auth_password(&ctx, user, password);
    memset(password, 0, sizeof(password));
    if (rc != SSH_OK) {
        printf("ssh: authentication failed\n");
        goto cleanup;
    }
    printf("Authenticated.\n");

    /* Open session channel + request shell */
    rc = ssh_channel_open_session(&ctx);
    if (rc != SSH_OK) {
        printf("ssh: failed to open session (%d)\n", rc);
        goto cleanup;
    }

    rc = ssh_channel_request_shell(&ctx);
    if (rc != SSH_OK) {
        printf("ssh: failed to start shell (%d)\n", rc);
        goto cleanup;
    }

    /* Interactive I/O loop */
    {
        unsigned char buf[4096];
        int prev_was_tilde = 0;

        while (1) {
            /* Read from SSH channel → stdout (non-blocking) */
            int n = ssh_channel_read(&ctx, buf, sizeof(buf));
            if (n < 0) {
                printf("\r\nConnection closed by remote host.\r\n");
                break;
            }
            if (n > 0) {
                _syscall(SYS_WRITE, 1, (int)buf, n, 0);
            }

            /* Read from stdin → SSH channel (non-blocking via raw syscall) */
            int r = _syscall(SYS_READ, 0, (int)buf, sizeof(buf), 0);
            if (r > 0) {
                /* Check for ~. escape sequence (disconnect) */
                for (int i = 0; i < r; i++) {
                    if (prev_was_tilde && buf[i] == '.') {
                        printf("\r\nDisconnected.\r\n");
                        goto cleanup;
                    }
                    prev_was_tilde = (buf[i] == '~');
                }

                int w = ssh_channel_write(&ctx, buf, r);
                if (w < 0) break;
            }

            /* Avoid busy-wait when idle */
            if (n == 0 && r <= 0) {
                _syscall(SYS_YIELD, 0, 0, 0, 0);
            }
        }
    }

cleanup:
    ssh_disconnect(&ctx, SSH_DISCONNECT_BY_APPLICATION, "bye");
    ssh_free(&ctx);
    _syscall(SYS_TCP_CLOSE, sock, 0, 0, 0);
    return 0;
}
