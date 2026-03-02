/*
 * scp — anyOS SCP client
 *
 * Usage:
 *   scp [-P port] [-r] [-p] [-q] source target
 *
 *   source/target syntax:
 *     [[user@]host:]path
 *
 *   Examples:
 *     scp file.txt user@host:/remote/dir/
 *     scp user@host:/remote/file.txt ./local/
 *     scp -r localdir/ user@host:/remote/
 *     scp -P 2222 user@host:/file.txt .
 *
 * Note: both source and target can be remote, but only one may be
 * remote at a time (standard scp behaviour).
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "ssh.h"

extern int _syscall(int num, int a1, int a2, int a3, int a4);

#define SYS_WRITE        2
#define SYS_READ         3
#define SYS_YIELD        7
#define SYS_SLEEP        8
#define SYS_NET_DNS      43
#define SYS_NET_POLL     50
#define SYS_TCP_CONNECT  100
#define SYS_TCP_CLOSE    103
#define SYS_TCP_STATUS   104

/* File system syscalls */
#define SYS_OPEN         10
#define SYS_CLOSE        11
#define SYS_READ_FD      12
#define SYS_WRITE_FD     13
#define SYS_STAT         20
#define SYS_READDIR      21
#define SYS_MKDIR        22

/* Open flags */
#define O_RDONLY   0
#define O_WRONLY   1
#define O_CREAT    0x200
#define O_TRUNC    0x400

/* TCP state */
#define TCP_STATE_ESTABLISHED 4

/* =========================================================================
 * Helpers
 * ========================================================================= */

static int tcp_connect(const unsigned char *ip, int port)
{
    unsigned char params[12];
    memcpy(params, ip, 4);
    params[4] = (port >> 8) & 0xFF;
    params[5] = port & 0xFF;
    params[6] = 0;
    params[7] = 0;
    unsigned int timeout = 10000;
    memcpy(params + 8, &timeout, 4);
    return _syscall(SYS_TCP_CONNECT, (int)params, 0, 0, 0);
}

static int resolve_host(const char *host, unsigned char *ip_out)
{
    int a, b, c, d;
    if (sscanf(host, "%d.%d.%d.%d", &a, &b, &c, &d) == 4) {
        ip_out[0] = a; ip_out[1] = b; ip_out[2] = c; ip_out[3] = d;
        return 0;
    }
    return _syscall(SYS_NET_DNS, (int)host, (int)ip_out, 0, 0);
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

/* stat: returns file size in bytes, -1 if not found */
static int file_stat_size(const char *path)
{
    /* stat_buf: [type:u32, size:u32, ...] */
    unsigned int stat_buf[8];
    int rc = _syscall(SYS_STAT, (int)path, (int)stat_buf, 0, 0);
    if (rc != 0) return -1;
    return (int)stat_buf[1];
}

/* Check if path is a directory (type == 2) */
static int is_directory(const char *path)
{
    unsigned int stat_buf[8];
    int rc = _syscall(SYS_STAT, (int)path, (int)stat_buf, 0, 0);
    if (rc != 0) return 0;
    return (stat_buf[0] == 2);
}

/* Format a human-readable size */
static void fmt_size(char *buf, unsigned int bytes)
{
    if (bytes < 1024) {
        sprintf(buf, "%uB", bytes);
    } else if (bytes < 1024 * 1024) {
        sprintf(buf, "%u.%uKB", bytes / 1024, (bytes % 1024) * 10 / 1024);
    } else {
        sprintf(buf, "%u.%uMB", bytes / (1024*1024),
                (bytes % (1024*1024)) * 10 / (1024*1024));
    }
}

/* =========================================================================
 * Location parsing
 * ========================================================================= */

typedef struct {
    char user[64];
    char host[128];
    char path[512];
    int  is_remote;
} scp_loc_t;

/*
 * Parse [user@host:]path
 * Sets is_remote=1 when a host is found.
 */
static void parse_location(const char *arg, scp_loc_t *loc)
{
    memset(loc, 0, sizeof(*loc));

    const char *colon = strchr(arg, ':');
    if (!colon) {
        /* Local path */
        strncpy(loc->path, arg, sizeof(loc->path) - 1);
        loc->is_remote = 0;
        return;
    }

    loc->is_remote = 1;

    /* host_part is everything before ':' */
    int host_part_len = (int)(colon - arg);
    char host_part[192];
    if (host_part_len >= (int)sizeof(host_part))
        host_part_len = (int)sizeof(host_part) - 1;
    memcpy(host_part, arg, host_part_len);
    host_part[host_part_len] = '\0';

    /* Split user@host */
    const char *at = strchr(host_part, '@');
    if (at) {
        int ulen = (int)(at - host_part);
        if (ulen >= (int)sizeof(loc->user)) ulen = (int)sizeof(loc->user) - 1;
        memcpy(loc->user, host_part, ulen);
        loc->user[ulen] = '\0';
        strncpy(loc->host, at + 1, sizeof(loc->host) - 1);
    } else {
        strncpy(loc->host, host_part, sizeof(loc->host) - 1);
    }

    /* Path is everything after ':' */
    strncpy(loc->path, colon + 1, sizeof(loc->path) - 1);
}

/* =========================================================================
 * SCP protocol helpers
 * ========================================================================= */

/*
 * SCP uses a simple line-based control channel over the SSH data channel.
 *
 * Commands sent TO remote:
 *   "C<mode> <size> <filename>\n"   — single file
 *   "D<mode> 0 <dirname>\n"         — enter directory
 *   "E\n"                           — leave directory
 *   "T<mtime> 0 <atime> 0\n"        — timestamps (with -p)
 *
 * Responses:
 *   0x00   — ACK (ok)
 *   0x01   — warning (followed by message + '\n')
 *   0x02   — fatal error (followed by message + '\n')
 */

/* Wait for a single-byte ACK (0x00) from the remote scp sink */
static int scp_wait_ack(ssh_ctx_t *ctx)
{
    unsigned char ack;
    /* Poll in a loop (ssh_channel_read is non-blocking) */
    int tries = 0;
    while (tries < 50000) {
        int n = ssh_channel_read(ctx, &ack, 1);
        if (n < 0) return -1;
        if (n == 0) {
            _syscall(SYS_YIELD, 0, 0, 0, 0);
            tries++;
            continue;
        }
        if (ack == 0x00) return 0;  /* ok */
        if (ack == 0x01 || ack == 0x02) {
            /* Read the error message */
            char errmsg[256];
            int ep = 0;
            unsigned char ch;
            while (ep < (int)sizeof(errmsg) - 1) {
                int r = ssh_channel_read(ctx, &ch, 1);
                if (r < 0 || ch == '\n') break;
                if (r > 0) errmsg[ep++] = (char)ch;
                else _syscall(SYS_YIELD, 0, 0, 0, 0);
            }
            errmsg[ep] = '\0';
            fprintf(stderr, "scp: remote error: %s\n", errmsg);
            return (ack == 0x02) ? -2 : -1;
        }
        return -1;
    }
    fprintf(stderr, "scp: timeout waiting for ACK\n");
    return -1;
}

/* Send a zero-byte ACK to the source side */
static int scp_send_ack(ssh_ctx_t *ctx)
{
    unsigned char ack = 0x00;
    return ssh_channel_write(ctx, &ack, 1);
}

/* Read one response line from remote (including newline, null-terminated) */
static int scp_read_response_line(ssh_ctx_t *ctx, char *buf, int maxlen)
{
    int pos = 0;
    int tries = 0;
    while (pos < maxlen - 1 && tries < 100000) {
        unsigned char ch;
        int n = ssh_channel_read(ctx, &ch, 1);
        if (n < 0) return -1;
        if (n == 0) {
            _syscall(SYS_YIELD, 0, 0, 0, 0);
            tries++;
            continue;
        }
        tries = 0;
        buf[pos++] = (char)ch;
        if (ch == '\n') break;
    }
    buf[pos] = '\0';
    return pos;
}

/* =========================================================================
 * Upload: local → remote
 * ========================================================================= */

/* Forward declaration for recursive directory upload */
static int scp_upload_dir(ssh_ctx_t *ctx, const char *local_path,
                           const char *dirname, int quiet, int preserve);

static int scp_upload_file(ssh_ctx_t *ctx, const char *local_path,
                            const char *filename, int quiet, int preserve)
{
    int size = file_stat_size(local_path);
    if (size < 0) {
        fprintf(stderr, "scp: %s: no such file\n", local_path);
        return 1;
    }

    /* Open local file */
    int fd = _syscall(SYS_OPEN, (int)local_path, O_RDONLY, 0, 0);
    if (fd < 0 || fd == (int)0xFFFFFFFF) {
        fprintf(stderr, "scp: cannot open '%s'\n", local_path);
        return 1;
    }

    /* Timestamps */
    if (preserve) {
        /* We don't have mtime from stat; send zeros */
        char tbuf[64];
        sprintf(tbuf, "T0 0 0 0\n");
        if (ssh_channel_write(ctx, (unsigned char *)tbuf, strlen(tbuf)) < 0) goto fail;
        if (scp_wait_ack(ctx) < 0) goto fail;
    }

    /* "C0644 <size> <filename>\n" */
    char cmd[640];
    sprintf(cmd, "C0644 %d %s\n", size, filename);
    if (ssh_channel_write(ctx, (unsigned char *)cmd, strlen(cmd)) < 0) goto fail;
    if (scp_wait_ack(ctx) < 0) goto fail;

    if (!quiet)
        printf("%s", filename);

    /* Send file data */
    unsigned char buf[8192];
    int sent = 0;
    while (sent < size) {
        int chunk = size - sent;
        if (chunk > (int)sizeof(buf)) chunk = (int)sizeof(buf);
        int n = _syscall(SYS_READ_FD, fd, (int)buf, chunk, 0);
        if (n <= 0) break;
        if (ssh_channel_write(ctx, buf, n) < 0) goto fail;
        sent += n;
        if (!quiet) {
            /* Overwrite progress */
            char sbuf[32], tbuf[32];
            fmt_size(sbuf, sent);
            fmt_size(tbuf, size);
            printf("\r%s  %s/%s", filename, sbuf, tbuf);
        }
    }

    _syscall(SYS_CLOSE, fd, 0, 0, 0);

    /* Send trailing NUL (end of file data) */
    unsigned char nul = 0x00;
    ssh_channel_write(ctx, &nul, 1);

    if (scp_wait_ack(ctx) < 0) return 1;

    if (!quiet) printf("\r%s  %s\n", filename, "done");
    return 0;

fail:
    _syscall(SYS_CLOSE, fd, 0, 0, 0);
    return 1;
}

static int scp_upload_dir(ssh_ctx_t *ctx, const char *local_path,
                           const char *dirname, int quiet, int preserve)
{
    /* "D0755 0 <dirname>\n" */
    char cmd[640];
    sprintf(cmd, "D0755 0 %s\n", dirname);
    if (ssh_channel_write(ctx, (unsigned char *)cmd, strlen(cmd)) < 0) return 1;
    if (scp_wait_ack(ctx) < 0) return 1;

    /* Read directory entries */
    unsigned char dirbuf[64 * 128];
    int count = _syscall(SYS_READDIR, (int)local_path, (int)dirbuf, sizeof(dirbuf), 0);
    if (count < 0) count = 0;

    /* Each entry: [type:u8, name_len:u8, flags:u8, pad:u8, size:u32, name:56bytes] */
    int errors = 0;
    for (int i = 0; i < count; i++) {
        unsigned char *entry = dirbuf + i * 64;
        unsigned char etype = entry[0];
        unsigned char name_len = entry[1];
        unsigned int esize;
        memcpy(&esize, entry + 4, 4);

        char name[57];
        memcpy(name, entry + 8, name_len < 56 ? name_len : 56);
        name[name_len < 56 ? name_len : 56] = '\0';

        /* Skip . and .. */
        if (strcmp(name, ".") == 0 || strcmp(name, "..") == 0) continue;

        char child_path[600];
        snprintf(child_path, sizeof(child_path), "%s/%s", local_path, name);

        if (etype == 2) {
            /* Directory */
            errors += scp_upload_dir(ctx, child_path, name, quiet, preserve);
        } else {
            /* File */
            errors += scp_upload_file(ctx, child_path, name, quiet, preserve);
        }
    }

    /* "E\n" — leave directory */
    const char *end_cmd = "E\n";
    if (ssh_channel_write(ctx, (unsigned char *)end_cmd, 2) < 0) return 1;
    if (scp_wait_ack(ctx) < 0) return 1;

    return errors;
}

/* =========================================================================
 * Download: remote → local
 * ========================================================================= */

static int scp_download(ssh_ctx_t *ctx, const char *local_dest,
                         int recursive, int quiet, int preserve)
{
    /* Send initial ACK to start the source */
    if (scp_send_ack(ctx) < 0) return 1;

    /* Determine if local_dest is an existing directory */
    int dest_is_dir = is_directory(local_dest);

    for (;;) {
        char line[640];
        int n = scp_read_response_line(ctx, line, sizeof(line));
        if (n <= 0) break;

        /* Remove trailing \n */
        int len = strlen(line);
        while (len > 0 && (line[len-1] == '\n' || line[len-1] == '\r'))
            line[--len] = '\0';

        if (len == 0) break;

        if (line[0] == 'E') {
            /* Leave directory */
            scp_send_ack(ctx);
            break;
        }

        if (line[0] == 'T') {
            /* Timestamps — acknowledge and skip (ignore for now) */
            scp_send_ack(ctx);
            continue;
        }

        if (line[0] == 'C' || line[0] == 'D') {
            /* "C<mode> <size> <name>" or "D<mode> 0 <name>" */
            char mode_str[8];
            unsigned int file_size = 0;
            char fname[256];
            if (sscanf(line + 1, "%7s %u %255s", mode_str, &file_size, fname) != 3) {
                fprintf(stderr, "scp: malformed scp header: %s\n", line);
                scp_send_ack(ctx);
                return 1;
            }

            /* Build local path */
            char dest_path[600];
            if (dest_is_dir) {
                snprintf(dest_path, sizeof(dest_path), "%s/%s", local_dest, fname);
            } else {
                strncpy(dest_path, local_dest, sizeof(dest_path) - 1);
                dest_path[sizeof(dest_path)-1] = '\0';
            }

            if (line[0] == 'D') {
                if (!recursive) {
                    fprintf(stderr, "scp: remote path is a directory; use -r\n");
                    return 1;
                }
                /* Create local directory */
                _syscall(SYS_MKDIR, (int)dest_path, 0755, 0, 0);
                scp_send_ack(ctx);

                /* Recurse into the directory */
                int rc = scp_download(ctx, dest_path, recursive, quiet, preserve);
                if (rc != 0) return rc;

                dest_is_dir = is_directory(local_dest);
                continue;
            }

            /* It's a file: "C" */
            scp_send_ack(ctx);

            /* Open local file for writing */
            int fd = _syscall(SYS_OPEN, (int)dest_path,
                              O_WRONLY | O_CREAT | O_TRUNC, 0, 0);
            if (fd < 0 || fd == (int)0xFFFFFFFF) {
                fprintf(stderr, "scp: cannot create '%s'\n", dest_path);
                return 1;
            }

            if (!quiet)
                printf("%s", fname);

            /* Receive file data */
            unsigned char buf[8192];
            unsigned int received = 0;
            while (received < file_size) {
                unsigned int want = file_size - received;
                if (want > sizeof(buf)) want = sizeof(buf);
                int got = ssh_channel_read(ctx, buf, want);
                if (got < 0) {
                    _syscall(SYS_CLOSE, fd, 0, 0, 0);
                    return 1;
                }
                if (got == 0) {
                    _syscall(SYS_YIELD, 0, 0, 0, 0);
                    continue;
                }
                _syscall(SYS_WRITE_FD, fd, (int)buf, got, 0);
                received += (unsigned int)got;

                if (!quiet) {
                    char sbuf[32], tbuf[32];
                    fmt_size(sbuf, received);
                    fmt_size(tbuf, file_size);
                    printf("\r%s  %s/%s", fname, sbuf, tbuf);
                }
            }

            _syscall(SYS_CLOSE, fd, 0, 0, 0);

            /* Read trailing NUL */
            {
                unsigned char nul;
                int tries = 0;
                while (tries < 10000) {
                    int r = ssh_channel_read(ctx, &nul, 1);
                    if (r > 0) break;
                    _syscall(SYS_YIELD, 0, 0, 0, 0);
                    tries++;
                }
            }

            scp_send_ack(ctx);

            if (!quiet) printf("\r%s  done\n", fname);

            /* After writing a single named file, stop */
            if (!dest_is_dir) break;

            continue;
        }

        if (line[0] == 0x01 || line[0] == 0x02) {
            fprintf(stderr, "scp: remote: %s\n", line + 1);
            return 1;
        }
    }

    return 0;
}

/* =========================================================================
 * SSH connection helpers
 * ========================================================================= */

static int do_connect(const char *host, int port, int quiet,
                      ssh_ctx_t *ctx_out)
{
    unsigned char ip[4];
    if (resolve_host(host, ip) != 0) {
        fprintf(stderr, "scp: cannot resolve '%s'\n", host);
        return -1;
    }

    if (!quiet)
        printf("Connecting to %s (%d.%d.%d.%d:%d)...\n",
               host, ip[0], ip[1], ip[2], ip[3], port);

    int sock = tcp_connect(ip, port);
    if (sock < 0 || sock == (int)0xFFFFFFFF) {
        fprintf(stderr, "scp: connection failed\n");
        return -1;
    }

    for (int tries = 0; tries < 100; tries++) {
        _syscall(SYS_NET_POLL, 0, 0, 0, 0);
        int st = _syscall(SYS_TCP_STATUS, sock, 0, 0, 0);
        if (st == TCP_STATE_ESTABLISHED) break;
        _syscall(SYS_SLEEP, 100, 0, 0, 0);
    }
    if (_syscall(SYS_TCP_STATUS, sock, 0, 0, 0) != TCP_STATE_ESTABLISHED) {
        fprintf(stderr, "scp: connection timed out\n");
        _syscall(SYS_TCP_CLOSE, sock, 0, 0, 0);
        return -1;
    }

    ssh_init(ctx_out, sock, 0);

    if (ssh_version_exchange(ctx_out) != SSH_OK) {
        fprintf(stderr, "scp: SSH version exchange failed\n");
        goto fail;
    }
    if (ssh_kex(ctx_out) != SSH_OK) {
        fprintf(stderr, "scp: SSH key exchange failed\n");
        goto fail;
    }

    return sock;

fail:
    ssh_free(ctx_out);
    _syscall(SYS_TCP_CLOSE, sock, 0, 0, 0);
    return -1;
}

static int do_auth(ssh_ctx_t *ctx, const char *host,
                   char *user, int quiet)
{
    /* Prompt for username if empty */
    if (user[0] == '\0') {
        printf("%s's username: ", host);
        read_line(user, 64);
    }

    char password[128];
    printf("%s@%s's password: ", user, host);
    read_line(password, sizeof(password));

    int rc = ssh_auth_password(ctx, user, password);
    memset(password, 0, sizeof(password));

    if (rc != SSH_OK) {
        fprintf(stderr, "scp: authentication failed\n");
        return -1;
    }
    return 0;
}

/* =========================================================================
 * Usage
 * ========================================================================= */

static void print_usage(void)
{
    printf("usage: scp [-pqr] [-P port] source ... target\n");
    printf("\n");
    printf("  source / target:  [[user@]host:]path\n");
    printf("\n");
    printf("  -P port   Use specified port (default 22)\n");
    printf("  -r        Recursively copy entire directories\n");
    printf("  -p        Preserve modification times and permissions\n");
    printf("  -q        Quiet mode (suppress progress)\n");
}

/* =========================================================================
 * main
 * ========================================================================= */

int main(int argc, char **argv)
{
    int port      = 22;
    int recursive = 0;
    int preserve  = 0;
    int quiet     = 0;

    /* ── Argument parsing ── */
    int optind = 1;
    for (; optind < argc; optind++) {
        if (argv[optind][0] != '-') break;
        const char *opt = argv[optind] + 1;
        if (*opt == '\0') break;   /* bare '-' treated as filename */

        for (; *opt; opt++) {
            switch (*opt) {
            case 'P':
                if (opt[1] != '\0') {
                    port = atoi(opt + 1);
                    opt += strlen(opt) - 1;  /* consume rest */
                } else if (optind + 1 < argc) {
                    port = atoi(argv[++optind]);
                }
                break;
            case 'r': case 'R': recursive = 1; break;
            case 'p':           preserve  = 1; break;
            case 'q':           quiet     = 1; break;
            case 'h':
                print_usage();
                return 0;
            default:
                fprintf(stderr, "scp: unknown option -%c\n", *opt);
                print_usage();
                return 1;
            }
        }
    }

    /* Need at least two non-option arguments: source + target */
    if (argc - optind < 2) {
        print_usage();
        return 1;
    }

    /* Last argument is the target; everything before are sources */
    int num_sources = argc - optind - 1;
    char **sources  = argv + optind;
    char  *target   = argv[argc - 1];

    scp_loc_t src_loc, dst_loc;
    parse_location(sources[0], &src_loc);
    parse_location(target, &dst_loc);

    /* Validate: at most one side can be remote */
    if (src_loc.is_remote && dst_loc.is_remote) {
        fprintf(stderr, "scp: remote-to-remote copy is not supported\n");
        return 1;
    }

    ssh_ctx_t ctx;
    memset(&ctx, 0, sizeof(ctx));
    int sock;

    /* ── Upload: local → remote ── */
    if (dst_loc.is_remote) {
        sock = do_connect(dst_loc.host, port, quiet, &ctx);
        if (sock < 0) return 1;

        if (do_auth(&ctx, dst_loc.host, dst_loc.user, quiet) < 0) {
            ssh_free(&ctx);
            _syscall(SYS_TCP_CLOSE, sock, 0, 0, 0);
            return 1;
        }

        /* Open session channel */
        if (ssh_channel_open_session(&ctx) != SSH_OK) {
            fprintf(stderr, "scp: could not open SSH channel\n");
            goto cleanup;
        }

        /* Launch remote scp -t (sink mode), with -r if recursive, -p if preserve */
        {
            char remote_cmd[600];
            snprintf(remote_cmd, sizeof(remote_cmd),
                     "scp%s%s -t %s",
                     recursive ? " -r" : "",
                     preserve  ? " -p" : "",
                     dst_loc.path);

            /* We reuse ssh_channel_request_shell but send an exec request */
            /* Build SSH_MSG_CHANNEL_REQUEST "exec" manually via low-level API */
            unsigned char pkt[700];
            unsigned int pkt_len = 0;
            unsigned int cmd_len = (unsigned int)strlen(remote_cmd);

            pkt[pkt_len++] = SSH_MSG_CHANNEL_REQUEST;
            /* recipient channel (remote_channel, big-endian u32) */
            pkt[pkt_len++] = (ctx.remote_channel >> 24) & 0xFF;
            pkt[pkt_len++] = (ctx.remote_channel >> 16) & 0xFF;
            pkt[pkt_len++] = (ctx.remote_channel >>  8) & 0xFF;
            pkt[pkt_len++] = (ctx.remote_channel      ) & 0xFF;
            /* request type: "exec" (u32 len + bytes) */
            pkt[pkt_len++] = 0; pkt[pkt_len++] = 0;
            pkt[pkt_len++] = 0; pkt[pkt_len++] = 4;
            pkt[pkt_len++] = 'e'; pkt[pkt_len++] = 'x';
            pkt[pkt_len++] = 'e'; pkt[pkt_len++] = 'c';
            /* want reply: true */
            pkt[pkt_len++] = 1;
            /* command string (u32 len + bytes) */
            pkt[pkt_len++] = (cmd_len >> 24) & 0xFF;
            pkt[pkt_len++] = (cmd_len >> 16) & 0xFF;
            pkt[pkt_len++] = (cmd_len >>  8) & 0xFF;
            pkt[pkt_len++] = (cmd_len      ) & 0xFF;
            memcpy(pkt + pkt_len, remote_cmd, cmd_len);
            pkt_len += cmd_len;

            if (ssh_send_packet(&ctx, pkt, pkt_len) != SSH_OK) {
                fprintf(stderr, "scp: could not exec remote scp\n");
                goto cleanup;
            }

            /* Wait for CHANNEL_SUCCESS */
            int msg = ssh_recv_packet(&ctx);
            if (msg != SSH_MSG_CHANNEL_SUCCESS && msg != SSH_MSG_CHANNEL_DATA) {
                fprintf(stderr, "scp: remote exec failed\n");
                goto cleanup;
            }
        }

        /* Wait for initial ACK from remote scp sink */
        if (scp_wait_ack(&ctx) < 0) goto cleanup;

        /* Upload each source */
        int errors = 0;
        for (int i = 0; i < num_sources; i++) {
            scp_loc_t sloc;
            parse_location(sources[i], &sloc);

            if (is_directory(sloc.path)) {
                if (!recursive) {
                    fprintf(stderr, "scp: %s: is a directory; use -r\n", sloc.path);
                    errors++;
                    continue;
                }
                /* Extract dirname (last component) */
                const char *dname = sloc.path;
                const char *p = sloc.path + strlen(sloc.path) - 1;
                while (p > sloc.path && (*p == '/' || *p == '\\')) p--;
                const char *slash = p;
                while (slash > sloc.path && *slash != '/' && *slash != '\\') slash--;
                if (*slash == '/' || *slash == '\\') dname = slash + 1;
                else dname = sloc.path;

                char dname_clean[256];
                int dlen = (int)(p - dname + 1);
                if (dlen <= 0) dlen = 1;
                if (dlen >= (int)sizeof(dname_clean)) dlen = (int)sizeof(dname_clean) - 1;
                memcpy(dname_clean, dname, dlen);
                dname_clean[dlen] = '\0';

                errors += scp_upload_dir(&ctx, sloc.path, dname_clean,
                                          quiet, preserve);
            } else {
                /* Extract filename */
                const char *fname = sloc.path;
                const char *sp = strrchr(sloc.path, '/');
                if (sp) fname = sp + 1;
                if (*fname == '\0') fname = sloc.path;

                errors += scp_upload_file(&ctx, sloc.path, fname,
                                           quiet, preserve);
            }
        }

        if (errors == 0 && !quiet)
            printf("scp: transfer complete\n");

    /* ── Download: remote → local ── */
    } else if (src_loc.is_remote) {
        sock = do_connect(src_loc.host, port, quiet, &ctx);
        if (sock < 0) return 1;

        if (do_auth(&ctx, src_loc.host, src_loc.user, quiet) < 0) {
            ssh_free(&ctx);
            _syscall(SYS_TCP_CLOSE, sock, 0, 0, 0);
            return 1;
        }

        if (ssh_channel_open_session(&ctx) != SSH_OK) {
            fprintf(stderr, "scp: could not open SSH channel\n");
            goto cleanup;
        }

        {
            char remote_cmd[600];
            snprintf(remote_cmd, sizeof(remote_cmd),
                     "scp%s%s -f %s",
                     recursive ? " -r" : "",
                     preserve  ? " -p" : "",
                     src_loc.path);

            unsigned char pkt[700];
            unsigned int pkt_len = 0;
            unsigned int cmd_len = (unsigned int)strlen(remote_cmd);

            pkt[pkt_len++] = SSH_MSG_CHANNEL_REQUEST;
            pkt[pkt_len++] = (ctx.remote_channel >> 24) & 0xFF;
            pkt[pkt_len++] = (ctx.remote_channel >> 16) & 0xFF;
            pkt[pkt_len++] = (ctx.remote_channel >>  8) & 0xFF;
            pkt[pkt_len++] = (ctx.remote_channel      ) & 0xFF;
            pkt[pkt_len++] = 0; pkt[pkt_len++] = 0;
            pkt[pkt_len++] = 0; pkt[pkt_len++] = 4;
            pkt[pkt_len++] = 'e'; pkt[pkt_len++] = 'x';
            pkt[pkt_len++] = 'e'; pkt[pkt_len++] = 'c';
            pkt[pkt_len++] = 1;
            pkt[pkt_len++] = (cmd_len >> 24) & 0xFF;
            pkt[pkt_len++] = (cmd_len >> 16) & 0xFF;
            pkt[pkt_len++] = (cmd_len >>  8) & 0xFF;
            pkt[pkt_len++] = (cmd_len      ) & 0xFF;
            memcpy(pkt + pkt_len, remote_cmd, cmd_len);
            pkt_len += cmd_len;

            if (ssh_send_packet(&ctx, pkt, pkt_len) != SSH_OK) {
                fprintf(stderr, "scp: could not exec remote scp\n");
                goto cleanup;
            }

            int msg = ssh_recv_packet(&ctx);
            if (msg != SSH_MSG_CHANNEL_SUCCESS && msg != SSH_MSG_CHANNEL_DATA) {
                fprintf(stderr, "scp: remote exec failed\n");
                goto cleanup;
            }
        }

        int rc = scp_download(&ctx, dst_loc.path, recursive, quiet, preserve);
        if (rc != 0) {
            ssh_disconnect(&ctx, SSH_DISCONNECT_BY_APPLICATION, "error");
            ssh_free(&ctx);
            _syscall(SYS_TCP_CLOSE, sock, 0, 0, 0);
            return rc;
        }

    } else {
        /* Local to local — delegate to cp */
        fprintf(stderr, "scp: local-to-local copy; use cp instead\n");
        return 1;
    }

cleanup:
    ssh_disconnect(&ctx, SSH_DISCONNECT_BY_APPLICATION, "done");
    ssh_free(&ctx);
    _syscall(SYS_TCP_CLOSE, sock, 0, 0, 0);
    return 0;
}
