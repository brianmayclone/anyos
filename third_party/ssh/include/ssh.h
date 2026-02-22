/*
 * ssh.h — Minimal SSH-2 protocol library for anyOS
 *
 * Supports:
 *   KEX:     curve25519-sha256
 *   Cipher:  aes128-ctr + hmac-sha2-256
 *   Auth:    password
 *   Channel: session -> shell
 */
#ifndef SSH_H
#define SSH_H

#include <stdint.h>
#include <stddef.h>

/* SSH message types (RFC 4253, 4252, 4254) */
#define SSH_MSG_DISCONNECT         1
#define SSH_MSG_IGNORE             2
#define SSH_MSG_UNIMPLEMENTED      3
#define SSH_MSG_DEBUG              4
#define SSH_MSG_SERVICE_REQUEST    5
#define SSH_MSG_SERVICE_ACCEPT     6
#define SSH_MSG_KEXINIT           20
#define SSH_MSG_NEWKEYS           21
#define SSH_MSG_KEX_ECDH_INIT    30
#define SSH_MSG_KEX_ECDH_REPLY   31
#define SSH_MSG_USERAUTH_REQUEST  50
#define SSH_MSG_USERAUTH_FAILURE  51
#define SSH_MSG_USERAUTH_SUCCESS  52
#define SSH_MSG_USERAUTH_BANNER   53
#define SSH_MSG_GLOBAL_REQUEST    80
#define SSH_MSG_REQUEST_SUCCESS   81
#define SSH_MSG_REQUEST_FAILURE   82
#define SSH_MSG_CHANNEL_OPEN      90
#define SSH_MSG_CHANNEL_OPEN_CONFIRMATION 91
#define SSH_MSG_CHANNEL_OPEN_FAILURE      92
#define SSH_MSG_CHANNEL_WINDOW_ADJUST     93
#define SSH_MSG_CHANNEL_DATA      94
#define SSH_MSG_CHANNEL_EOF       96
#define SSH_MSG_CHANNEL_CLOSE     97
#define SSH_MSG_CHANNEL_REQUEST   98
#define SSH_MSG_CHANNEL_SUCCESS   99
#define SSH_MSG_CHANNEL_FAILURE  100

/* Disconnect reason codes */
#define SSH_DISCONNECT_HOST_NOT_ALLOWED_TO_CONNECT  1
#define SSH_DISCONNECT_PROTOCOL_ERROR               2
#define SSH_DISCONNECT_KEY_EXCHANGE_FAILED           3
#define SSH_DISCONNECT_AUTH_CANCELLED_BY_USER       13
#define SSH_DISCONNECT_BY_APPLICATION               11

/* Max sizes */
#define SSH_MAX_PACKET  35000
#define SSH_MAX_PAYLOAD 32768

/* Error codes */
#define SSH_OK           0
#define SSH_ERR_IO      -1
#define SSH_ERR_PROTO   -2
#define SSH_ERR_AUTH    -3
#define SSH_ERR_TIMEOUT -4
#define SSH_ERR_KEX     -5
#define SSH_ERR_ALLOC   -6

/* Direction indices for cipher keys */
#define SSH_DIR_C2S  0  /* client-to-server */
#define SSH_DIR_S2C  1  /* server-to-client */

/* SSH connection context */
typedef struct ssh_ctx {
    int sock;       /* TCP socket fd or anyOS socket id */

    /* Version strings (null-terminated) */
    char client_version[64];
    char server_version[64];

    /* Key exchange state */
    uint8_t session_id[32];  /* H from first KEX */
    int     session_id_set;
    uint8_t kex_hash[32];    /* exchange hash H for current KEX */

    /* Encryption keys (derived from KEX) */
    uint8_t key_c2s[32];     /* client-to-server encryption key */
    uint8_t key_s2c[32];     /* server-to-client encryption key */
    uint8_t iv_c2s[16];      /* client-to-server IV (AES-CTR counter) */
    uint8_t iv_s2c[16];      /* server-to-client IV */
    uint8_t mac_c2s[32];     /* client-to-server MAC key */
    uint8_t mac_s2c[32];     /* server-to-client MAC key */

    /* Sequence numbers */
    uint32_t seq_c2s;
    uint32_t seq_s2c;

    /* Encryption active flag */
    int encrypted;

    /* Channel state */
    uint32_t channel_id;
    uint32_t remote_channel;
    uint32_t remote_window;
    uint32_t remote_max_packet;
    uint32_t local_window;

    /* KEXINIT payloads (needed for exchange hash) */
    uint8_t *client_kexinit;
    uint32_t client_kexinit_len;
    uint8_t *server_kexinit;
    uint32_t server_kexinit_len;

    /* I/O buffers */
    uint8_t  rbuf[SSH_MAX_PACKET];
    uint32_t rbuf_len;
    uint32_t rbuf_pos;

    /* Server mode */
    int is_server;
} ssh_ctx_t;

/* =========================================================================
 * Core API
 * ========================================================================= */

/* Initialize an SSH context */
void ssh_init(ssh_ctx_t *ctx, int sock, int is_server);

/* Free resources associated with SSH context */
void ssh_free(ssh_ctx_t *ctx);

/* Perform SSH version exchange. Returns SSH_OK or error. */
int ssh_version_exchange(ssh_ctx_t *ctx);

/* Perform key exchange (curve25519-sha256). Returns SSH_OK or error. */
int ssh_kex(ssh_ctx_t *ctx);

/* Authenticate with password. Returns SSH_OK or SSH_ERR_AUTH. */
int ssh_auth_password(ssh_ctx_t *ctx, const char *username, const char *password);

/* Open a session channel. Returns SSH_OK or error. */
int ssh_channel_open_session(ssh_ctx_t *ctx);

/* Request a shell on the session channel. Returns SSH_OK or error. */
int ssh_channel_request_shell(ssh_ctx_t *ctx);

/* Send data on the channel. Returns bytes sent or < 0 on error. */
int ssh_channel_write(ssh_ctx_t *ctx, const uint8_t *data, uint32_t len);

/* Receive data from the channel. Returns bytes read, 0=EOF, <0=error.
 * Non-blocking: returns 0 immediately if no data available. */
int ssh_channel_read(ssh_ctx_t *ctx, uint8_t *buf, uint32_t len);

/* Send disconnect message and close. */
void ssh_disconnect(ssh_ctx_t *ctx, uint32_t reason, const char *desc);

/* =========================================================================
 * Low-level packet I/O
 * ========================================================================= */

/* Send an SSH binary packet (handles encryption + MAC if active).
 * payload does NOT include the type byte (it's payload[0]).
 * Returns SSH_OK or SSH_ERR_IO. */
int ssh_send_packet(ssh_ctx_t *ctx, const uint8_t *payload, uint32_t len);

/* Receive an SSH binary packet. Stores payload in ctx->rbuf, length in
 * ctx->rbuf_len, resets ctx->rbuf_pos to 0.
 * Returns message type (>0) or error (<0). */
int ssh_recv_packet(ssh_ctx_t *ctx);

/* =========================================================================
 * Server-side API
 * ========================================================================= */

/* Server KEX (responds to client's KEXINIT). Returns SSH_OK or error. */
int ssh_server_kex(ssh_ctx_t *ctx,
                   const uint8_t *host_key_priv, uint32_t host_key_priv_len,
                   const uint8_t *host_key_pub, uint32_t host_key_pub_len);

/* Server authentication: receive and validate auth request.
 * On success, writes username to user_buf. Returns SSH_OK or SSH_ERR_AUTH. */
int ssh_server_auth(ssh_ctx_t *ctx, char *user_buf, uint32_t user_buf_len,
                    char *pass_buf, uint32_t pass_buf_len);

/* Server channel: accept channel open request. Returns SSH_OK or error. */
int ssh_server_accept_channel(ssh_ctx_t *ctx);

/* Server: accept shell request. Returns SSH_OK or error. */
int ssh_server_accept_shell(ssh_ctx_t *ctx);

#endif /* SSH_H */
