/*
 * sshd — anyOS SSH server daemon
 *
 * Reads configuration from /System/etc/ssh/ssh_users.conf
 * Format:
 *   [welcome]   — banner text shown after login
 *   [shell]     — path to shell (e.g. /bin/sh)
 *   [users]     — username lines; prefixed with ! means DENIED
 *
 * Listens on port 22 (or -p PORT), accepts SSH connections,
 * authenticates against anyOS user database, and spawns a shell
 * with stdin/stdout piped through the SSH channel.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "ssh.h"

extern int _syscall(int num, int a1, int a2, int a3, int a4);

#define SYS_EXIT           1
#define SYS_WRITE          2
#define SYS_READ           3
#define SYS_OPEN           4
#define SYS_CLOSE          5
#define SYS_YIELD          7
#define SYS_SLEEP          8
#define SYS_SPAWN          27
#define SYS_TRY_WAITPID   29
#define SYS_NET_POLL       50
#define SYS_PIPE_CREATE    45
#define SYS_PIPE_READ      46
#define SYS_PIPE_CLOSE     47
#define SYS_PIPE_WRITE     48
#define SYS_PIPE_OPEN      49
#define SYS_TCP_SEND       101
#define SYS_TCP_RECV       102
#define SYS_TCP_CLOSE      103
#define SYS_TCP_STATUS     104
#define SYS_TCP_LISTEN     132
#define SYS_TCP_ACCEPT     133
#define SYS_AUTHENTICATE   223
#define SYS_RANDOM         210

#define TCP_STATE_ESTABLISHED 4
#define STILL_RUNNING 0xFFFFFFFE

/* Configuration from ssh_users.conf */
#define MAX_USERS 32
#define MAX_WELCOME 512

typedef struct {
    char welcome[MAX_WELCOME];
    char shell[128];
    char users[MAX_USERS][64];
    int  user_denied[MAX_USERS]; /* 1 if prefixed with ! */
    int  user_count;
    int  listen_port;
} sshd_config_t;

/* Ed25519 host key (generated at first boot) */
static unsigned char host_key_priv[64];
static unsigned char host_key_pub[32];

/* =========================================================================
 * Configuration parser
 * ========================================================================= */

static void parse_config(sshd_config_t *cfg, const char *path)
{
    memset(cfg, 0, sizeof(sshd_config_t));
    strcpy(cfg->shell, "/bin/sh");  /* default */
    cfg->listen_port = 22;

    /* Open and read config file */
    int fd = _syscall(SYS_OPEN, (int)path, 0 /* O_RDONLY */, 0, 0);
    if (fd < 0) {
        printf("sshd: warning: cannot open %s, using defaults\n", path);
        return;
    }

    char buf[2048];
    int n = _syscall(SYS_READ, fd, (int)buf, sizeof(buf) - 1, 0);
    _syscall(SYS_CLOSE, fd, 0, 0, 0);
    if (n <= 0) return;
    buf[n] = '\0';

    /* Simple INI parser */
    int section = 0; /* 0=none, 1=welcome, 2=shell, 3=users */
    int welcome_pos = 0;

    char *line = buf;
    while (line && *line) {
        char *nl = strchr(line, '\n');
        if (nl) *nl = '\0';

        /* Skip leading whitespace */
        while (*line == ' ' || *line == '\t') line++;

        /* Check for section header */
        if (line[0] == '[') {
            if (strncmp(line, "[welcome]", 9) == 0) {
                section = 1;
            } else if (strncmp(line, "[shell]", 7) == 0) {
                section = 2;
            } else if (strncmp(line, "[users]", 7) == 0) {
                section = 3;
            } else {
                section = 0;
            }
        } else if (line[0] != '\0' && line[0] != '#') {
            /* Content line */
            switch (section) {
                case 1: /* welcome */
                    if (welcome_pos > 0 && welcome_pos < MAX_WELCOME - 2) {
                        cfg->welcome[welcome_pos++] = '\n';
                    }
                    {
                        int len = strlen(line);
                        if (welcome_pos + len < MAX_WELCOME - 1) {
                            memcpy(cfg->welcome + welcome_pos, line, len);
                            welcome_pos += len;
                        }
                    }
                    cfg->welcome[welcome_pos] = '\0';
                    break;

                case 2: /* shell */
                    strncpy(cfg->shell, line, sizeof(cfg->shell) - 1);
                    break;

                case 3: /* users */
                    if (cfg->user_count < MAX_USERS) {
                        int idx = cfg->user_count;
                        if (line[0] == '!') {
                            cfg->user_denied[idx] = 1;
                            strncpy(cfg->users[idx], line + 1,
                                    sizeof(cfg->users[idx]) - 1);
                        } else {
                            cfg->user_denied[idx] = 0;
                            strncpy(cfg->users[idx], line,
                                    sizeof(cfg->users[idx]) - 1);
                        }
                        cfg->user_count++;
                    }
                    break;
            }
        }

        line = nl ? nl + 1 : NULL;
    }
}

/* Check if a user is allowed to login per the config.
 * Returns 1 if allowed, 0 if denied. */
static int user_allowed(sshd_config_t *cfg, const char *username)
{
    for (int i = 0; i < cfg->user_count; i++) {
        if (strcmp(cfg->users[i], username) == 0) {
            return cfg->user_denied[i] ? 0 : 1;
        }
    }
    /* User not in list: allow by default */
    return 1;
}

/* Authenticate user against anyOS user database */
static int authenticate_user(const char *username, const char *password)
{
    /* SYS_AUTHENTICATE(username, password) → 0=success, else fail */
    return _syscall(SYS_AUTHENTICATE, (int)username, (int)password, 0, 0);
}

/* =========================================================================
 * Session handler (one per accepted connection)
 * ========================================================================= */

static void handle_session(int sock, sshd_config_t *cfg)
{
    ssh_ctx_t ctx;
    ssh_init(&ctx, sock, 1); /* server mode */

    /* Version exchange */
    int rc = ssh_version_exchange(&ctx);
    if (rc != SSH_OK) {
        printf("sshd: version exchange failed (%d)\n", rc);
        goto done;
    }

    /* Key exchange (server side) */
    rc = ssh_server_kex(&ctx, host_key_priv, sizeof(host_key_priv),
                        host_key_pub, sizeof(host_key_pub));
    if (rc != SSH_OK) {
        printf("sshd: key exchange failed (%d)\n", rc);
        goto done;
    }

    /* Authentication */
    char username[64], password[128];
    rc = ssh_server_auth(&ctx, username, sizeof(username),
                         password, sizeof(password));
    if (rc != SSH_OK) {
        printf("sshd: auth protocol error (%d)\n", rc);
        goto done;
    }

    /* Check user against config deny list */
    if (!user_allowed(cfg, username)) {
        printf("sshd: user '%s' denied by config\n", username);
        /* Send auth failure */
        uint8_t fail[32];
        uint32_t off = 0;
        fail[off++] = SSH_MSG_USERAUTH_FAILURE;
        fail[off++] = 0; fail[off++] = 0; fail[off++] = 0; fail[off++] = 0;
        fail[off++] = 0; /* partial success = false */
        ssh_send_packet(&ctx, fail, off);
        goto done;
    }

    /* Verify password against anyOS user DB */
    if (authenticate_user(username, password) != 0) {
        printf("sshd: auth failed for '%s'\n", username);
        uint8_t fail[32];
        uint32_t off = 0;
        fail[off++] = SSH_MSG_USERAUTH_FAILURE;
        fail[off++] = 0; fail[off++] = 0; fail[off++] = 0; fail[off++] = 0;
        fail[off++] = 0;
        ssh_send_packet(&ctx, fail, off);
        goto done;
    }
    memset(password, 0, sizeof(password));

    /* Send auth success */
    {
        uint8_t ok = SSH_MSG_USERAUTH_SUCCESS;
        ssh_send_packet(&ctx, &ok, 1);
    }

    printf("sshd: user '%s' authenticated\n", username);

    /* Accept channel open + shell request */
    rc = ssh_server_accept_channel(&ctx);
    if (rc != SSH_OK) goto done;

    rc = ssh_server_accept_shell(&ctx);
    if (rc != SSH_OK) goto done;

    /* Send welcome banner if configured */
    if (cfg->welcome[0]) {
        ssh_channel_write(&ctx, (uint8_t *)cfg->welcome, strlen(cfg->welcome));
        ssh_channel_write(&ctx, (uint8_t *)"\r\n", 2);
    }

    /* Create pipe for shell I/O */
    char pipe_name[64];
    {
        unsigned int rnd;
        _syscall(SYS_RANDOM, (int)&rnd, 4, 0, 0);
        snprintf(pipe_name, sizeof(pipe_name), "sshd_%u", rnd & 0xFFFF);
    }
    _syscall(SYS_PIPE_CREATE, (int)pipe_name, 4096, 0, 0);

    /* Spawn shell */
    char shell_args[256];
    snprintf(shell_args, sizeof(shell_args), "%s --pipe %s",
             cfg->shell, pipe_name);
    int shell_tid = _syscall(SYS_SPAWN, (int)cfg->shell, 0,
                             (int)shell_args, 0);
    if (shell_tid <= 0) {
        printf("sshd: failed to spawn shell '%s'\n", cfg->shell);
        _syscall(SYS_PIPE_CLOSE, (int)pipe_name, 0, 0, 0);
        goto done;
    }

    /* I/O forwarding loop: SSH channel ↔ shell pipe */
    {
        unsigned char buf[4096];
        int pipe_fd = _syscall(SYS_PIPE_OPEN, (int)pipe_name, 0, 0, 0);

        while (1) {
            /* Check if shell exited */
            int exit_code = _syscall(SYS_TRY_WAITPID, shell_tid, 0, 0, 0);
            if (exit_code != (int)STILL_RUNNING) {
                printf("sshd: shell exited with code %d\n", exit_code);
                break;
            }

            /* Shell → SSH: read pipe, write to channel */
            int n = _syscall(SYS_PIPE_READ, (int)pipe_name, (int)buf,
                             sizeof(buf), 0);
            if (n > 0) {
                ssh_channel_write(&ctx, buf, n);
            }

            /* SSH → Shell: read channel, write to pipe */
            int r = ssh_channel_read(&ctx, buf, sizeof(buf));
            if (r < 0) break;
            if (r > 0) {
                _syscall(SYS_PIPE_WRITE, (int)pipe_name, (int)buf, r, 0);
            }

            if (n <= 0 && r <= 0) {
                _syscall(SYS_SLEEP, 10, 0, 0, 0); /* 10ms idle sleep */
            }
        }

        if (pipe_fd >= 0) {
            _syscall(SYS_PIPE_CLOSE, (int)pipe_name, 0, 0, 0);
        }
    }

done:
    ssh_disconnect(&ctx, SSH_DISCONNECT_BY_APPLICATION, "goodbye");
    ssh_free(&ctx);
    _syscall(SYS_TCP_CLOSE, sock, 0, 0, 0);
}

/* =========================================================================
 * Main — listen + accept loop
 * ========================================================================= */

int main(int argc, char **argv)
{
    sshd_config_t cfg;
    int port = 22;

    /* Parse arguments */
    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], "-p") == 0 && i + 1 < argc) {
            port = atoi(argv[++i]);
        }
    }

    /* Load configuration */
    parse_config(&cfg, "/System/etc/ssh/ssh_users.conf");
    cfg.listen_port = port;

    /* Generate host key (random Ed25519 key pair for now) */
    _syscall(SYS_RANDOM, (int)host_key_priv, 64, 0, 0);
    _syscall(SYS_RANDOM, (int)host_key_pub, 32, 0, 0);

    printf("sshd: starting on port %d\n", port);
    printf("sshd: shell = %s\n", cfg.shell);
    printf("sshd: %d user rules loaded\n", cfg.user_count);

    /* Start listening */
    int listener = _syscall(SYS_TCP_LISTEN, port, 5, 0, 0);
    if (listener < 0 || listener == (int)0xFFFFFFFF) {
        printf("sshd: failed to listen on port %d\n", port);
        return 1;
    }

    printf("sshd: listening on port %d (listener_id=%d)\n", port, listener);

    /* Accept loop */
    while (1) {
        _syscall(SYS_NET_POLL, 0, 0, 0, 0);

        /* Try to accept (non-blocking check) */
        unsigned char result[12]; /* socket_id(4) + ip(4) + port(2) + pad(2) */
        int rc = _syscall(SYS_TCP_ACCEPT, listener, (int)result, 0, 0);

        if (rc == 0) {
            unsigned int new_sock;
            memcpy(&new_sock, result, 4);
            unsigned char *remote_ip = result + 4;
            unsigned int remote_port = (result[8] << 8) | result[9];

            printf("sshd: connection from %d.%d.%d.%d:%d (sock=%d)\n",
                   remote_ip[0], remote_ip[1], remote_ip[2], remote_ip[3],
                   remote_port, new_sock);

            handle_session(new_sock, &cfg);
        } else {
            /* No pending connection — sleep 1 second before next poll */
            _syscall(SYS_SLEEP, 1000, 0, 0, 0);
        }
    }

    return 0;
}
