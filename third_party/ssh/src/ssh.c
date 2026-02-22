/*
 * ssh.c — Minimal SSH-2 protocol implementation for anyOS
 *
 * KEX:    curve25519-sha256
 * Cipher: aes128-ctr + hmac-sha2-256 (encrypt-and-MAC)
 * Auth:   password
 *
 * Uses BearSSL for all cryptographic operations.
 */

#include "ssh.h"
#include <string.h>
#include <stdlib.h>
#include <bearssl.h>

/* anyOS syscall interface (32-bit INT 0x80) */
extern int _syscall(int num, int a, int b, int c, int d);

#define SYS_TCP_SEND    101
#define SYS_TCP_RECV    102
#define SYS_TCP_STATUS  104
#define SYS_TCP_RECV_AVAILABLE 130
#define SYS_RANDOM      210
#define SYS_NET_POLL    50
#define SYS_WRITE       2

/* Simple debug output via serial (fd=1) */
static void dbg(const char *msg) {
    int len = 0;
    while (msg[len]) len++;
    _syscall(SYS_WRITE, 1, (int)msg, len, 0);
}
static void dbg_int(const char *prefix, int val) {
    char buf[64];
    int pos = 0;
    while (*prefix && pos < 50) buf[pos++] = *prefix++;
    /* Write decimal number */
    if (val < 0) { buf[pos++] = '-'; val = -val; }
    char tmp[12];
    int tpos = 0;
    if (val == 0) tmp[tpos++] = '0';
    while (val > 0 && tpos < 11) { tmp[tpos++] = '0' + (val % 10); val /= 10; }
    while (tpos > 0) buf[pos++] = tmp[--tpos];
    buf[pos++] = '\n';
    _syscall(SYS_WRITE, 1, (int)buf, pos, 0);
}

static void dbg_hex(const char *label, const uint8_t *data, int len) {
    static const char hex[] = "0123456789abcdef";
    char buf[256];
    int pos = 0;
    while (*label && pos < 40) buf[pos++] = *label++;
    for (int i = 0; i < len && pos < 250; i++) {
        buf[pos++] = hex[(data[i] >> 4) & 0x0F];
        buf[pos++] = hex[data[i] & 0x0F];
    }
    buf[pos++] = '\n';
    _syscall(SYS_WRITE, 1, (int)buf, pos, 0);
}

/* =========================================================================
 * Helpers
 * ========================================================================= */

static void put_u32(uint8_t *buf, uint32_t v) {
    buf[0] = (v >> 24) & 0xFF;
    buf[1] = (v >> 16) & 0xFF;
    buf[2] = (v >>  8) & 0xFF;
    buf[3] = v & 0xFF;
}

static uint32_t get_u32(const uint8_t *buf) {
    return ((uint32_t)buf[0] << 24) | ((uint32_t)buf[1] << 16) |
           ((uint32_t)buf[2] <<  8) | (uint32_t)buf[3];
}

static void put_string(uint8_t *buf, const void *data, uint32_t len, uint32_t *offset) {
    put_u32(buf + *offset, len);
    *offset += 4;
    memcpy(buf + *offset, data, len);
    *offset += len;
}

static void put_cstring(uint8_t *buf, const char *str, uint32_t *offset) {
    uint32_t len = (uint32_t)strlen(str);
    put_string(buf, str, len, offset);
}

static int get_string(const uint8_t *buf, uint32_t buf_len, uint32_t *offset,
                      const uint8_t **out, uint32_t *out_len) {
    if (*offset + 4 > buf_len) return -1;
    uint32_t len = get_u32(buf + *offset);
    *offset += 4;
    if (*offset + len > buf_len) return -1;
    *out = buf + *offset;
    *out_len = len;
    *offset += len;
    return 0;
}

/* Reverse 32 bytes in-place (little-endian <-> big-endian) */
static void reverse32(uint8_t *buf) {
    for (int i = 0; i < 16; i++) {
        uint8_t tmp = buf[i];
        buf[i] = buf[31 - i];
        buf[31 - i] = tmp;
    }
}

/* Generate random bytes using kernel RDRAND */
static void ssh_random(void *buf, size_t len) {
    _syscall(SYS_RANDOM, (int)buf, (int)len, 0, 0);
}

/* Raw TCP send (bypasses libc, uses anyOS syscall directly) */
static int tcp_send(int sock, const void *data, int len) {
    return _syscall(SYS_TCP_SEND, sock, (int)data, len, 0);
}

/* Raw TCP recv (with polling) */
static int tcp_recv(int sock, void *buf, int len) {
    return _syscall(SYS_TCP_RECV, sock, (int)buf, len, 0);
}

/* Non-blocking check for data available */
static int tcp_available(int sock) {
    return _syscall(SYS_TCP_RECV_AVAILABLE, sock, 0, 0, 0);
}

/* Trigger network polling */
static void net_poll(void) {
    _syscall(SYS_NET_POLL, 0, 0, 0, 0);
}

/* =========================================================================
 * SSH Binary Packet Protocol (RFC 4253 Section 6)
 * ========================================================================= */

/* Read exactly n bytes from socket */
static int read_exact(int sock, uint8_t *buf, int n) {
    int total = 0;
    while (total < n) {
        net_poll();
        int r = tcp_recv(sock, buf + total, n - total);
        if (r == 0 || r == (int)0xFFFFFFFF) return SSH_ERR_IO;
        total += r;
    }
    return total;
}

/* Send exactly n bytes to socket */
static int write_all(int sock, const uint8_t *buf, int n) {
    int total = 0;
    while (total < n) {
        int w = tcp_send(sock, buf + total, n - total);
        if (w == 0 || w == (int)0xFFFFFFFF) return SSH_ERR_IO;
        total += w;
    }
    return total;
}

/* AES-128 CTR encryption/decryption (in-place) */
static void aes_ctr_crypt(const uint8_t *key, uint8_t *iv, uint8_t *data, uint32_t len) {
    br_aes_ct_ctr_keys aes;
    br_aes_ct_ctr_init(&aes, key, 16);
    /* iv is 16 bytes: first 12 are the IV, last 4 are the counter.
     * BearSSL CTR mode expects: iv[0..3] = block counter (big-endian).
     * We use the full 16-byte IV as the initial counter block. */
    uint32_t ctr = get_u32(iv + 12);
    ctr = br_aes_ct_ctr_run(&aes, iv, ctr, data, len);
    /* Update IV counter for next block */
    put_u32(iv + 12, ctr);
}

/* Compute HMAC-SHA256 */
static void hmac_sha256(const uint8_t *key, uint32_t key_len,
                        const uint8_t *data, uint32_t data_len,
                        uint8_t *mac) {
    br_hmac_key_context kc;
    br_hmac_context hc;
    br_hmac_key_init(&kc, &br_sha256_vtable, key, key_len);
    br_hmac_init(&hc, &kc, 32);
    br_hmac_update(&hc, data, data_len);
    br_hmac_out(&hc, mac);
}

int ssh_send_packet(ssh_ctx_t *ctx, const uint8_t *payload, uint32_t len) {
    /* Direction: client sends c2s, server sends s2c */
    const uint8_t *skey = ctx->is_server ? ctx->key_s2c : ctx->key_c2s;
    uint8_t       *siv  = ctx->is_server ? ctx->iv_s2c  : ctx->iv_c2s;
    const uint8_t *smac = ctx->is_server ? ctx->mac_s2c : ctx->mac_c2s;
    uint32_t      *sseq = ctx->is_server ? &ctx->seq_s2c : &ctx->seq_c2s;

    /* packet_length(4) + padding_length(1) + payload(len) + padding(pad) */
    /* Block size is 16 for AES-CTR, minimum padding is 4 */
    uint32_t block_size = ctx->encrypted ? 16 : 8;
    uint32_t base = 4 + 1 + len;
    uint32_t pad = block_size - (base % block_size);
    if (pad < 4) pad += block_size;
    uint32_t packet_length = 1 + len + pad;
    uint32_t total = 4 + packet_length;

    uint8_t *pkt = (uint8_t *)malloc(total + 32); /* +32 for MAC */
    if (!pkt) return SSH_ERR_ALLOC;

    put_u32(pkt, packet_length);
    pkt[4] = (uint8_t)pad;
    memcpy(pkt + 5, payload, len);
    ssh_random(pkt + 5 + len, pad);

    if (ctx->encrypted) {
        /* MAC: HMAC-SHA256(mac_key, sequence_number(4) || unencrypted_packet) */
        uint8_t mac_input[4];
        put_u32(mac_input, *sseq);

        br_hmac_key_context kc;
        br_hmac_context hc;
        br_hmac_key_init(&kc, &br_sha256_vtable, smac, 32);
        br_hmac_init(&hc, &kc, 32);
        br_hmac_update(&hc, mac_input, 4);
        br_hmac_update(&hc, pkt, total);
        br_hmac_out(&hc, pkt + total);

        /* Encrypt (the entire packet including length) */
        aes_ctr_crypt(skey, siv, pkt, total);

        total += 32; /* MAC appended */
    }

    (*sseq)++;
    int rc = write_all(ctx->sock, pkt, total);
    free(pkt);
    return rc > 0 ? SSH_OK : SSH_ERR_IO;
}

int ssh_recv_packet(ssh_ctx_t *ctx) {
    /* Direction: client receives s2c, server receives c2s */
    const uint8_t *rkey = ctx->is_server ? ctx->key_c2s : ctx->key_s2c;
    uint8_t       *riv  = ctx->is_server ? ctx->iv_c2s  : ctx->iv_s2c;
    const uint8_t *rmac = ctx->is_server ? ctx->mac_c2s : ctx->mac_s2c;
    uint32_t      *rseq = ctx->is_server ? &ctx->seq_c2s : &ctx->seq_s2c;

    uint8_t header[4];
    int rc;

    if (ctx->encrypted) {
        /* Read first 16 bytes (one AES block), decrypt to get packet_length */
        uint8_t first_block[16];
        rc = read_exact(ctx->sock, first_block, 16);
        if (rc < 0) return rc;

        /* Decrypt first block to get packet length */
        aes_ctr_crypt(rkey, riv, first_block, 16);

        uint32_t packet_length = get_u32(first_block);
        if (packet_length > SSH_MAX_PACKET - 4) return SSH_ERR_PROTO;

        uint32_t remaining = packet_length + 4 - 16;
        uint8_t *full_pkt = (uint8_t *)malloc(packet_length + 4 + 32);
        if (!full_pkt) return SSH_ERR_ALLOC;

        memcpy(full_pkt, first_block, 16);

        if (remaining > 0) {
            rc = read_exact(ctx->sock, full_pkt + 16, remaining);
            if (rc < 0) { free(full_pkt); return rc; }
            /* Decrypt remaining */
            aes_ctr_crypt(rkey, riv, full_pkt + 16, remaining);
        }

        /* Read MAC (32 bytes for HMAC-SHA256) */
        uint8_t received_mac[32];
        rc = read_exact(ctx->sock, received_mac, 32);
        if (rc < 0) { free(full_pkt); return rc; }

        /* Verify MAC */
        uint8_t seq_buf[4];
        put_u32(seq_buf, *rseq);
        uint8_t computed_mac[32];
        br_hmac_key_context kc;
        br_hmac_context hc;
        br_hmac_key_init(&kc, &br_sha256_vtable, rmac, 32);
        br_hmac_init(&hc, &kc, 32);
        br_hmac_update(&hc, seq_buf, 4);
        br_hmac_update(&hc, full_pkt, packet_length + 4);
        br_hmac_out(&hc, computed_mac);

        if (memcmp(received_mac, computed_mac, 32) != 0) {
            free(full_pkt);
            return SSH_ERR_PROTO; /* MAC verification failed */
        }

        uint8_t pad_len = full_pkt[4];
        uint32_t payload_len = packet_length - pad_len - 1;
        if (payload_len > SSH_MAX_PAYLOAD) { free(full_pkt); return SSH_ERR_PROTO; }

        memcpy(ctx->rbuf, full_pkt + 5, payload_len);
        ctx->rbuf_len = payload_len;
        ctx->rbuf_pos = 0;
        (*rseq)++;
        int msg_type = ctx->rbuf[0];
        free(full_pkt);
        return msg_type;

    } else {
        /* Unencrypted: read 4-byte length, then rest */
        rc = read_exact(ctx->sock, header, 4);
        if (rc < 0) return rc;

        uint32_t packet_length = get_u32(header);
        if (packet_length > SSH_MAX_PACKET - 4) return SSH_ERR_PROTO;

        uint8_t *body = (uint8_t *)malloc(packet_length);
        if (!body) return SSH_ERR_ALLOC;

        rc = read_exact(ctx->sock, body, packet_length);
        if (rc < 0) { free(body); return rc; }

        uint8_t pad_len = body[0];
        uint32_t payload_len = packet_length - pad_len - 1;
        if (payload_len > SSH_MAX_PAYLOAD) { free(body); return SSH_ERR_PROTO; }

        memcpy(ctx->rbuf, body + 1, payload_len);
        ctx->rbuf_len = payload_len;
        ctx->rbuf_pos = 0;
        (*rseq)++;
        int msg_type = ctx->rbuf[0];
        free(body);
        return msg_type;
    }
}

/* =========================================================================
 * Context Management
 * ========================================================================= */

void ssh_init(ssh_ctx_t *ctx, int sock, int is_server) {
    memset(ctx, 0, sizeof(ssh_ctx_t));
    ctx->sock = sock;
    ctx->is_server = is_server;
    if (is_server) {
        strcpy(ctx->server_version, "SSH-2.0-anyOS_sshd_1.0");
    } else {
        strcpy(ctx->client_version, "SSH-2.0-anyOS_1.0");
    }
    ctx->local_window = 0x200000; /* 2 MB */
    ctx->channel_id = 0;
}

void ssh_free(ssh_ctx_t *ctx) {
    if (ctx->client_kexinit) free(ctx->client_kexinit);
    if (ctx->server_kexinit) free(ctx->server_kexinit);
    ctx->client_kexinit = NULL;
    ctx->server_kexinit = NULL;
}

/* =========================================================================
 * Version Exchange (RFC 4253 Section 4.2)
 * ========================================================================= */

int ssh_version_exchange(ssh_ctx_t *ctx) {
    /* Our version is in server_version (server mode) or client_version (client) */
    const char *our_ver = ctx->is_server ? ctx->server_version : ctx->client_version;
    char *peer_ver = ctx->is_server ? ctx->client_version : ctx->server_version;

    /* Send our version string */
    char ver[128];
    int vlen = strlen(our_ver);
    memcpy(ver, our_ver, vlen);
    ver[vlen] = '\r';
    ver[vlen + 1] = '\n';

    if (write_all(ctx->sock, (uint8_t *)ver, vlen + 2) < 0)
        return SSH_ERR_IO;

    /* Read peer version string */
    char line[256];
    int pos = 0;
    while (pos < 255) {
        int r = read_exact(ctx->sock, (uint8_t *)&line[pos], 1);
        if (r < 0) return SSH_ERR_IO;
        if (line[pos] == '\n') {
            line[pos] = '\0';
            /* Strip \r if present */
            if (pos > 0 && line[pos - 1] == '\r')
                line[pos - 1] = '\0';
            break;
        }
        pos++;
    }

    if (strncmp(line, "SSH-2.0-", 8) != 0 && strncmp(line, "SSH-1.99-", 9) != 0) {
        dbg("ssh: bad version line\n");
        return SSH_ERR_PROTO;
    }

    strncpy(peer_ver, line, 63);
    peer_ver[63] = '\0';
    dbg("ssh: peer version: "); dbg(peer_ver); dbg("\n");
    return SSH_OK;
}

/* =========================================================================
 * Key Exchange (curve25519-sha256)
 * ========================================================================= */

/* Algorithm name lists for KEXINIT */
static const char *kex_algos      = "curve25519-sha256,curve25519-sha256@libssh.org";
static const char *host_key_algos = "ecdsa-sha2-nistp256,ssh-ed25519,ssh-rsa,rsa-sha2-256,rsa-sha2-512";
static const char *cipher_algos   = "aes128-ctr";
static const char *mac_algos      = "hmac-sha2-256";
static const char *comp_algos     = "none";
static const char *lang           = "";

static uint32_t build_kexinit(uint8_t *buf) {
    uint32_t off = 0;
    buf[off++] = SSH_MSG_KEXINIT;

    /* 16 bytes cookie (random) */
    ssh_random(buf + off, 16);
    off += 16;

    /* Algorithm lists */
    put_cstring(buf, kex_algos, &off);
    put_cstring(buf, host_key_algos, &off);
    put_cstring(buf, cipher_algos, &off);  /* c2s encryption */
    put_cstring(buf, cipher_algos, &off);  /* s2c encryption */
    put_cstring(buf, mac_algos, &off);     /* c2s MAC */
    put_cstring(buf, mac_algos, &off);     /* s2c MAC */
    put_cstring(buf, comp_algos, &off);    /* c2s compression */
    put_cstring(buf, comp_algos, &off);    /* s2c compression */
    put_cstring(buf, lang, &off);          /* c2s language */
    put_cstring(buf, lang, &off);          /* s2c language */

    buf[off++] = 0; /* first_kex_packet_follows = false */
    put_u32(buf + off, 0); off += 4; /* reserved */

    return off;
}

/* Derive a key using SHA-256(K || H || X || session_id) per RFC 4253 7.2 */
static void derive_key(const uint8_t *shared_secret, uint32_t ss_len,
                       const uint8_t *hash, const uint8_t *session_id,
                       char label, uint32_t needed,
                       uint8_t *out) {
    br_sha256_context sha;
    uint8_t digest[32];

    br_sha256_init(&sha);
    /* K as mpint (length-prefixed, with leading zero if high bit set) */
    if (ss_len > 0 && (shared_secret[0] & 0x80)) {
        uint32_t mpint_len = ss_len + 1;
        uint8_t len_buf[4];
        put_u32(len_buf, mpint_len);
        br_sha256_update(&sha, len_buf, 4);
        uint8_t zero = 0;
        br_sha256_update(&sha, &zero, 1);
        br_sha256_update(&sha, shared_secret, ss_len);
    } else {
        uint8_t len_buf[4];
        put_u32(len_buf, ss_len);
        br_sha256_update(&sha, len_buf, 4);
        br_sha256_update(&sha, shared_secret, ss_len);
    }
    br_sha256_update(&sha, hash, 32);      /* H */
    br_sha256_update(&sha, (uint8_t *)&label, 1); /* single char */
    br_sha256_update(&sha, session_id, 32); /* session_id */
    br_sha256_out(&sha, digest);

    uint32_t have = 32;
    memcpy(out, digest, have < needed ? have : needed);

    /* If we need more than 32 bytes, chain: K || H || K_prev */
    while (have < needed) {
        br_sha256_init(&sha);
        uint8_t len_buf[4];
        put_u32(len_buf, ss_len);
        br_sha256_update(&sha, len_buf, 4);
        br_sha256_update(&sha, shared_secret, ss_len);
        br_sha256_update(&sha, hash, 32);
        br_sha256_update(&sha, out, have);
        br_sha256_out(&sha, digest);
        uint32_t copy = (needed - have < 32) ? (needed - have) : 32;
        memcpy(out + have, digest, copy);
        have += copy;
    }
}

int ssh_kex(ssh_ctx_t *ctx) {
    int rc;

    /* 1. Send KEXINIT */
    uint8_t kexinit_buf[1024];
    uint32_t kexinit_len = build_kexinit(kexinit_buf);

    /* Save our KEXINIT for exchange hash */
    ctx->client_kexinit = (uint8_t *)malloc(kexinit_len);
    if (!ctx->client_kexinit) return SSH_ERR_ALLOC;
    memcpy(ctx->client_kexinit, kexinit_buf, kexinit_len);
    ctx->client_kexinit_len = kexinit_len;

    rc = ssh_send_packet(ctx, kexinit_buf, kexinit_len);
    if (rc != SSH_OK) return rc;

    /* 2. Receive server KEXINIT */
    rc = ssh_recv_packet(ctx);
    if (rc < 0) return rc;
    if (rc != SSH_MSG_KEXINIT) return SSH_ERR_PROTO;

    ctx->server_kexinit = (uint8_t *)malloc(ctx->rbuf_len);
    if (!ctx->server_kexinit) return SSH_ERR_ALLOC;
    memcpy(ctx->server_kexinit, ctx->rbuf, ctx->rbuf_len);
    ctx->server_kexinit_len = ctx->rbuf_len;

    /* 3. Generate ephemeral X25519 key pair */
    uint8_t my_priv[32], my_pub[32];
    ssh_random(my_priv, 32);
    /* Clamp private key per Curve25519 spec */
    my_priv[0] &= 248;
    my_priv[31] &= 127;
    my_priv[31] |= 64;

    /* Compute public key: my_pub = my_priv * G */
    /* BearSSL Curve25519: point is just 32 bytes (x-coordinate) */
    const br_ec_impl *ec = &br_ec_c25519_i31;

    /* The generator for Curve25519 is the point with x=9 */
    uint8_t gen[32];
    memset(gen, 0, 32);
    gen[0] = 9;

    memcpy(my_pub, gen, 32);
    uint32_t mul_ok = ec->mul(my_pub, 32, my_priv, 32, BR_EC_curve25519);
    if (!mul_ok) return SSH_ERR_KEX;

    /* 4. Send KEX_ECDH_INIT */
    uint8_t ecdh_init[1 + 4 + 32];
    ecdh_init[0] = SSH_MSG_KEX_ECDH_INIT;
    put_u32(ecdh_init + 1, 32);
    memcpy(ecdh_init + 5, my_pub, 32);
    rc = ssh_send_packet(ctx, ecdh_init, sizeof(ecdh_init));
    if (rc != SSH_OK) return rc;

    /* 5. Receive KEX_ECDH_REPLY */
    rc = ssh_recv_packet(ctx);
    if (rc < 0) return rc;
    if (rc != SSH_MSG_KEX_ECDH_REPLY) return SSH_ERR_PROTO;

    /* Parse: K_S (host key), Q_S (server ephemeral pub), signature */
    uint32_t off = 1; /* skip message type */
    const uint8_t *host_key_blob;
    uint32_t host_key_blob_len;
    if (get_string(ctx->rbuf, ctx->rbuf_len, &off, &host_key_blob, &host_key_blob_len) < 0)
        return SSH_ERR_PROTO;

    const uint8_t *server_pub;
    uint32_t server_pub_len;
    if (get_string(ctx->rbuf, ctx->rbuf_len, &off, &server_pub, &server_pub_len) < 0)
        return SSH_ERR_PROTO;
    if (server_pub_len != 32) return SSH_ERR_PROTO;

    const uint8_t *signature;
    uint32_t signature_len;
    if (get_string(ctx->rbuf, ctx->rbuf_len, &off, &signature, &signature_len) < 0)
        return SSH_ERR_PROTO;

    /* 6. Compute shared secret K = my_priv * server_pub */
    uint8_t shared_secret[32];
    memcpy(shared_secret, server_pub, 32);
    mul_ok = ec->mul(shared_secret, 32, my_priv, 32, BR_EC_curve25519);
    if (!mul_ok) return SSH_ERR_KEX;

    /* Note: BearSSL returns X25519 result in little-endian (RFC 7748).
     * OpenSSH feeds raw X25519 output directly to sshbuf_put_bignum2_bytes
     * without reversing — treating LE bytes as BE. We must do the same
     * for exchange hash compatibility. Do NOT reverse. */

    /* 7. Compute exchange hash H = SHA256(V_C || V_S || I_C || I_S || K_S || Q_C || Q_S || K) */
    br_sha256_context sha;
    br_sha256_init(&sha);

    /* V_C (client version string, without CRLF) */
    uint8_t len_buf[4];
    put_u32(len_buf, strlen(ctx->client_version));
    br_sha256_update(&sha, len_buf, 4);
    br_sha256_update(&sha, ctx->client_version, strlen(ctx->client_version));

    /* V_S (server version string) */
    put_u32(len_buf, strlen(ctx->server_version));
    br_sha256_update(&sha, len_buf, 4);
    br_sha256_update(&sha, ctx->server_version, strlen(ctx->server_version));

    /* I_C (client KEXINIT payload) */
    put_u32(len_buf, ctx->client_kexinit_len);
    br_sha256_update(&sha, len_buf, 4);
    br_sha256_update(&sha, ctx->client_kexinit, ctx->client_kexinit_len);

    /* I_S (server KEXINIT payload) */
    put_u32(len_buf, ctx->server_kexinit_len);
    br_sha256_update(&sha, len_buf, 4);
    br_sha256_update(&sha, ctx->server_kexinit, ctx->server_kexinit_len);

    /* K_S (host key blob) */
    put_u32(len_buf, host_key_blob_len);
    br_sha256_update(&sha, len_buf, 4);
    br_sha256_update(&sha, host_key_blob, host_key_blob_len);

    /* Q_C (our ephemeral public) */
    put_u32(len_buf, 32);
    br_sha256_update(&sha, len_buf, 4);
    br_sha256_update(&sha, my_pub, 32);

    /* Q_S (server ephemeral public) */
    put_u32(len_buf, 32);
    br_sha256_update(&sha, len_buf, 4);
    br_sha256_update(&sha, server_pub, 32);

    /* K (shared secret as mpint) */
    if (shared_secret[0] & 0x80) {
        put_u32(len_buf, 33);
        br_sha256_update(&sha, len_buf, 4);
        uint8_t zero = 0;
        br_sha256_update(&sha, &zero, 1);
        br_sha256_update(&sha, shared_secret, 32);
    } else {
        /* Strip leading zeros for mpint */
        int start = 0;
        while (start < 31 && shared_secret[start] == 0) start++;
        uint32_t ss_len = 32 - start;
        put_u32(len_buf, ss_len);
        br_sha256_update(&sha, len_buf, 4);
        br_sha256_update(&sha, shared_secret + start, ss_len);
    }

    br_sha256_out(&sha, ctx->kex_hash);

    /* Set session_id from first KEX */
    if (!ctx->session_id_set) {
        memcpy(ctx->session_id, ctx->kex_hash, 32);
        ctx->session_id_set = 1;
    }

    /* NOTE: Host key signature verification is skipped for now (TOFU model) */

    /* 8. Send NEWKEYS */
    uint8_t newkeys = SSH_MSG_NEWKEYS;
    rc = ssh_send_packet(ctx, &newkeys, 1);
    if (rc != SSH_OK) return rc;

    /* 9. Receive NEWKEYS */
    rc = ssh_recv_packet(ctx);
    if (rc < 0) return rc;
    if (rc != SSH_MSG_NEWKEYS) return SSH_ERR_PROTO;

    /* 10. Derive encryption keys */
    /* Strip leading zeros from shared_secret for key derivation */
    int ss_start = 0;
    while (ss_start < 31 && shared_secret[ss_start] == 0) ss_start++;
    uint32_t ss_len = 32 - ss_start;

    derive_key(shared_secret + ss_start, ss_len, ctx->kex_hash, ctx->session_id,
               'A', 16, ctx->iv_c2s);   /* Initial IV c2s */
    derive_key(shared_secret + ss_start, ss_len, ctx->kex_hash, ctx->session_id,
               'B', 16, ctx->iv_s2c);   /* Initial IV s2c */
    derive_key(shared_secret + ss_start, ss_len, ctx->kex_hash, ctx->session_id,
               'C', 16, ctx->key_c2s);  /* Encryption key c2s */
    derive_key(shared_secret + ss_start, ss_len, ctx->kex_hash, ctx->session_id,
               'D', 16, ctx->key_s2c);  /* Encryption key s2c */
    derive_key(shared_secret + ss_start, ss_len, ctx->kex_hash, ctx->session_id,
               'E', 32, ctx->mac_c2s);  /* MAC key c2s */
    derive_key(shared_secret + ss_start, ss_len, ctx->kex_hash, ctx->session_id,
               'F', 32, ctx->mac_s2c);  /* MAC key s2c */

    ctx->encrypted = 1;
    /* RFC 4253 Section 6.4: sequence numbers must NEVER be reset, even after rekey */

    /* Clean up sensitive data */
    memset(my_priv, 0, 32);
    memset(shared_secret, 0, 32);

    return SSH_OK;
}

/* =========================================================================
 * User Authentication (RFC 4252)
 * ========================================================================= */

int ssh_auth_password(ssh_ctx_t *ctx, const char *username, const char *password) {
    int rc;

    /* Request ssh-userauth service */
    uint8_t srv[64];
    uint32_t off = 0;
    srv[off++] = SSH_MSG_SERVICE_REQUEST;
    put_cstring(srv, "ssh-userauth", &off);
    rc = ssh_send_packet(ctx, srv, off);
    if (rc != SSH_OK) return rc;

    rc = ssh_recv_packet(ctx);
    if (rc < 0) return rc;
    if (rc != SSH_MSG_SERVICE_ACCEPT) return SSH_ERR_PROTO;

    /* Send password auth request */
    uint8_t auth[512];
    off = 0;
    auth[off++] = SSH_MSG_USERAUTH_REQUEST;
    put_cstring(auth, username, &off);
    put_cstring(auth, "ssh-connection", &off);
    put_cstring(auth, "password", &off);
    auth[off++] = 0; /* FALSE = not changing password */
    put_cstring(auth, password, &off);

    rc = ssh_send_packet(ctx, auth, off);
    if (rc != SSH_OK) return rc;

    /* Receive response */
    rc = ssh_recv_packet(ctx);
    if (rc < 0) return rc;

    if (rc == SSH_MSG_USERAUTH_SUCCESS)
        return SSH_OK;
    if (rc == SSH_MSG_USERAUTH_FAILURE)
        return SSH_ERR_AUTH;
    if (rc == SSH_MSG_USERAUTH_BANNER) {
        /* Read banner, then wait for the real response */
        rc = ssh_recv_packet(ctx);
        if (rc == SSH_MSG_USERAUTH_SUCCESS) return SSH_OK;
        if (rc == SSH_MSG_USERAUTH_FAILURE) return SSH_ERR_AUTH;
    }

    return SSH_ERR_PROTO;
}

/* =========================================================================
 * Channel Management (RFC 4254)
 * ========================================================================= */

int ssh_channel_open_session(ssh_ctx_t *ctx) {
    uint8_t buf[64];
    uint32_t off = 0;
    buf[off++] = SSH_MSG_CHANNEL_OPEN;
    put_cstring(buf, "session", &off);
    put_u32(buf + off, ctx->channel_id); off += 4;  /* sender channel */
    put_u32(buf + off, ctx->local_window); off += 4; /* initial window */
    put_u32(buf + off, SSH_MAX_PAYLOAD); off += 4;   /* max packet */

    int rc = ssh_send_packet(ctx, buf, off);
    if (rc != SSH_OK) return rc;

    rc = ssh_recv_packet(ctx);
    if (rc < 0) return rc;
    if (rc != SSH_MSG_CHANNEL_OPEN_CONFIRMATION) return SSH_ERR_PROTO;

    /* Parse response */
    off = 1; /* skip type */
    ctx->remote_channel = get_u32(ctx->rbuf + off); off += 4;
    /* skip sender channel */ off += 4;
    ctx->remote_window = get_u32(ctx->rbuf + off); off += 4;
    ctx->remote_max_packet = get_u32(ctx->rbuf + off); off += 4;

    return SSH_OK;
}

int ssh_channel_request_shell(ssh_ctx_t *ctx) {
    /* Request pseudo-terminal first */
    uint8_t buf[128];
    uint32_t off = 0;
    buf[off++] = SSH_MSG_CHANNEL_REQUEST;
    put_u32(buf + off, ctx->remote_channel); off += 4;
    put_cstring(buf, "pty-req", &off);
    buf[off++] = 1; /* want reply */
    put_cstring(buf, "xterm", &off); /* TERM */
    put_u32(buf + off, 80); off += 4;  /* columns */
    put_u32(buf + off, 24); off += 4;  /* rows */
    put_u32(buf + off, 0); off += 4;   /* pixel width */
    put_u32(buf + off, 0); off += 4;   /* pixel height */
    put_u32(buf + off, 0); off += 4;   /* terminal modes (empty) */

    int rc = ssh_send_packet(ctx, buf, off);
    if (rc != SSH_OK) return rc;

    rc = ssh_recv_packet(ctx);
    if (rc < 0) return rc;
    /* Accept success or failure for pty (some servers don't support it) */

    /* Request shell */
    off = 0;
    buf[off++] = SSH_MSG_CHANNEL_REQUEST;
    put_u32(buf + off, ctx->remote_channel); off += 4;
    put_cstring(buf, "shell", &off);
    buf[off++] = 1; /* want reply */

    rc = ssh_send_packet(ctx, buf, off);
    if (rc != SSH_OK) return rc;

    rc = ssh_recv_packet(ctx);
    if (rc < 0) return rc;
    if (rc == SSH_MSG_CHANNEL_SUCCESS || rc == SSH_MSG_CHANNEL_WINDOW_ADJUST)
        return SSH_OK;

    return SSH_ERR_PROTO;
}

int ssh_channel_write(ssh_ctx_t *ctx, const uint8_t *data, uint32_t len) {
    if (len == 0) return 0;

    uint32_t max_chunk = ctx->remote_max_packet;
    if (max_chunk > SSH_MAX_PAYLOAD - 9) max_chunk = SSH_MAX_PAYLOAD - 9;
    if (len > max_chunk) len = max_chunk;

    uint8_t *buf = (uint8_t *)malloc(9 + len);
    if (!buf) return SSH_ERR_ALLOC;

    uint32_t off = 0;
    buf[off++] = SSH_MSG_CHANNEL_DATA;
    put_u32(buf + off, ctx->remote_channel); off += 4;
    put_u32(buf + off, len); off += 4;
    memcpy(buf + off, data, len);
    off += len;

    int rc = ssh_send_packet(ctx, buf, off);
    free(buf);
    return rc == SSH_OK ? (int)len : rc;
}

int ssh_channel_read(ssh_ctx_t *ctx, uint8_t *buf, uint32_t len) {
    /* Check if there's data available */
    net_poll();
    int avail = tcp_available(ctx->sock);
    if (avail <= 0) return 0; /* No data available yet */

    int rc = ssh_recv_packet(ctx);
    if (rc < 0) return rc;

    if (rc == SSH_MSG_CHANNEL_DATA) {
        uint32_t off = 1;
        /* skip recipient channel */ off += 4;
        uint32_t data_len = get_u32(ctx->rbuf + off); off += 4;
        if (data_len > len) data_len = len;
        memcpy(buf, ctx->rbuf + off, data_len);
        return (int)data_len;
    }
    if (rc == SSH_MSG_CHANNEL_WINDOW_ADJUST) {
        /* Update window and try again */
        uint32_t off = 1 + 4;
        ctx->remote_window += get_u32(ctx->rbuf + off);
        return 0; /* No data, but not an error */
    }
    if (rc == SSH_MSG_CHANNEL_EOF || rc == SSH_MSG_CHANNEL_CLOSE) {
        return -1; /* Channel closed */
    }
    if (rc == SSH_MSG_CHANNEL_REQUEST) {
        /* Server request (e.g. exit-status) — ignore */
        return 0;
    }

    return 0; /* Unknown message, ignore */
}

void ssh_disconnect(ssh_ctx_t *ctx, uint32_t reason, const char *desc) {
    uint8_t buf[256];
    uint32_t off = 0;
    buf[off++] = SSH_MSG_DISCONNECT;
    put_u32(buf + off, reason); off += 4;
    put_cstring(buf, desc ? desc : "", &off);
    put_cstring(buf, "", &off); /* language tag */
    ssh_send_packet(ctx, buf, off);
}

/* =========================================================================
 * Server-side Key Exchange (curve25519-sha256 + ecdsa-sha2-nistp256 host key)
 * ========================================================================= */

/* Server host key algorithm list (ecdsa-sha2-nistp256 only) */
static const char *server_host_key_algos = "ecdsa-sha2-nistp256";

/* Build an ecdsa-sha2-nistp256 host key blob from a P-256 public key.
 * pub_point: 65 bytes (04 || x(32) || y(32))
 * Returns blob length written to buf. */
static uint32_t build_ecdsa_host_key_blob(uint8_t *buf, const uint8_t *pub_point) {
    uint32_t off = 0;
    put_cstring(buf, "ecdsa-sha2-nistp256", &off);
    put_cstring(buf, "nistp256", &off);
    put_string(buf, pub_point, 65, &off);
    return off;
}

/* Build a server KEXINIT (uses server_host_key_algos) */
static uint32_t build_server_kexinit(uint8_t *buf) {
    uint32_t off = 0;
    buf[off++] = SSH_MSG_KEXINIT;
    ssh_random(buf + off, 16);
    off += 16;

    put_cstring(buf, kex_algos, &off);
    put_cstring(buf, server_host_key_algos, &off);
    put_cstring(buf, cipher_algos, &off);
    put_cstring(buf, cipher_algos, &off);
    put_cstring(buf, mac_algos, &off);
    put_cstring(buf, mac_algos, &off);
    put_cstring(buf, comp_algos, &off);
    put_cstring(buf, comp_algos, &off);
    put_cstring(buf, lang, &off);
    put_cstring(buf, lang, &off);

    buf[off++] = 0; /* first_kex_packet_follows = false */
    put_u32(buf + off, 0); off += 4;
    return off;
}

int ssh_server_kex(ssh_ctx_t *ctx,
                   const uint8_t *host_key_priv, uint32_t host_key_priv_len,
                   const uint8_t *host_key_pub, uint32_t host_key_pub_len) {
    int rc;
    (void)host_key_priv_len; (void)host_key_pub_len;

    /* Generate ECDSA P-256 host key pair for signing */
    uint8_t ecdsa_priv[32];
    uint8_t ecdsa_pub[65]; /* 04 + x(32) + y(32) */
    memcpy(ecdsa_priv, host_key_priv, 32);

    /* Compute public key: ecdsa_pub = ecdsa_priv * G on P-256 */
    const br_ec_impl *ec_p256 = br_ec_get_default();
    size_t pub_len = ec_p256->mulgen(ecdsa_pub, ecdsa_priv, 32, BR_EC_secp256r1);
    if (pub_len == 0) return SSH_ERR_KEX;

    /* Build host key blob */
    uint8_t host_key_blob[256];
    uint32_t host_key_blob_len = build_ecdsa_host_key_blob(host_key_blob, ecdsa_pub);

    /* 1. Receive client KEXINIT */
    dbg("sshd-kex: waiting for client KEXINIT...\n");
    rc = ssh_recv_packet(ctx);
    dbg_int("sshd-kex: recv_packet returned ", rc);
    if (rc < 0) return rc;
    if (rc != SSH_MSG_KEXINIT) return SSH_ERR_PROTO;

    ctx->client_kexinit = (uint8_t *)malloc(ctx->rbuf_len);
    if (!ctx->client_kexinit) return SSH_ERR_ALLOC;
    memcpy(ctx->client_kexinit, ctx->rbuf, ctx->rbuf_len);
    ctx->client_kexinit_len = ctx->rbuf_len;
    dbg_int("sshd-kex: got client KEXINIT bytes=", ctx->client_kexinit_len);

    /* 2. Send server KEXINIT */
    uint8_t kexinit_buf[1024];
    uint32_t kexinit_len = build_server_kexinit(kexinit_buf);
    dbg_int("sshd-kex: sending server KEXINIT bytes=", kexinit_len);

    ctx->server_kexinit = (uint8_t *)malloc(kexinit_len);
    if (!ctx->server_kexinit) return SSH_ERR_ALLOC;
    memcpy(ctx->server_kexinit, kexinit_buf, kexinit_len);
    ctx->server_kexinit_len = kexinit_len;

    rc = ssh_send_packet(ctx, kexinit_buf, kexinit_len);
    dbg_int("sshd-kex: send_packet returned ", rc);
    if (rc != SSH_OK) return rc;

    /* 3. Receive ECDH_INIT (client ephemeral public key) */
    dbg("sshd-kex: waiting for ECDH_INIT...\n");
    rc = ssh_recv_packet(ctx);
    dbg_int("sshd-kex: ECDH recv returned ", rc);
    if (rc < 0) return rc;
    if (rc != SSH_MSG_KEX_ECDH_INIT) return SSH_ERR_PROTO;

    uint32_t off = 1;
    const uint8_t *client_pub;
    uint32_t client_pub_len;
    if (get_string(ctx->rbuf, ctx->rbuf_len, &off, &client_pub, &client_pub_len) < 0)
        return SSH_ERR_PROTO;
    if (client_pub_len != 32) return SSH_ERR_PROTO;

    /* Save client pub for exchange hash */
    uint8_t client_pub_copy[32];
    memcpy(client_pub_copy, client_pub, 32);

    /* 4. Generate server ephemeral X25519 key pair */
    uint8_t my_priv[32], my_pub[32];
    ssh_random(my_priv, 32);
    my_priv[0] &= 248;
    my_priv[31] &= 127;
    my_priv[31] |= 64;

    const br_ec_impl *ec = &br_ec_c25519_i31;
    uint8_t gen[32];
    memset(gen, 0, 32);
    gen[0] = 9;
    memcpy(my_pub, gen, 32);
    uint32_t mul_ok = ec->mul(my_pub, 32, my_priv, 32, BR_EC_curve25519);
    if (!mul_ok) return SSH_ERR_KEX;

    /* 5. Compute shared secret K = my_priv * client_pub */
    uint8_t shared_secret[32];
    memcpy(shared_secret, client_pub_copy, 32);
    mul_ok = ec->mul(shared_secret, 32, my_priv, 32, BR_EC_curve25519);
    if (!mul_ok) return SSH_ERR_KEX;

    /* Note: BearSSL returns X25519 result in little-endian (RFC 7748).
     * OpenSSH feeds raw X25519 output directly to sshbuf_put_bignum2_bytes
     * without reversing — treating LE bytes as BE. We must do the same
     * for exchange hash compatibility. Do NOT reverse. */

    /* 6. Compute exchange hash H = SHA256(V_C || V_S || I_C || I_S || K_S || Q_C || Q_S || K) */
    br_sha256_context sha;
    br_sha256_init(&sha);
    uint8_t len_buf[4];

    /* V_C */
    dbg_int("H: V_C len=", (int)strlen(ctx->client_version));
    dbg_hex("H: V_C=", (const uint8_t *)ctx->client_version, strlen(ctx->client_version) > 32 ? 32 : strlen(ctx->client_version));
    put_u32(len_buf, strlen(ctx->client_version));
    br_sha256_update(&sha, len_buf, 4);
    br_sha256_update(&sha, ctx->client_version, strlen(ctx->client_version));
    /* V_S */
    dbg_int("H: V_S len=", (int)strlen(ctx->server_version));
    dbg_hex("H: V_S=", (const uint8_t *)ctx->server_version, strlen(ctx->server_version) > 32 ? 32 : strlen(ctx->server_version));
    put_u32(len_buf, strlen(ctx->server_version));
    br_sha256_update(&sha, len_buf, 4);
    br_sha256_update(&sha, ctx->server_version, strlen(ctx->server_version));
    /* I_C */
    dbg_int("H: I_C len=", (int)ctx->client_kexinit_len);
    dbg_hex("H: I_C[0:16]=", ctx->client_kexinit, 16);
    put_u32(len_buf, ctx->client_kexinit_len);
    br_sha256_update(&sha, len_buf, 4);
    br_sha256_update(&sha, ctx->client_kexinit, ctx->client_kexinit_len);
    /* I_S */
    dbg_int("H: I_S len=", (int)ctx->server_kexinit_len);
    dbg_hex("H: I_S[0:16]=", ctx->server_kexinit, 16);
    put_u32(len_buf, ctx->server_kexinit_len);
    br_sha256_update(&sha, len_buf, 4);
    br_sha256_update(&sha, ctx->server_kexinit, ctx->server_kexinit_len);
    /* K_S (host key blob) */
    dbg_int("H: K_S len=", (int)host_key_blob_len);
    dbg_hex("H: K_S[0:16]=", host_key_blob, 16);
    put_u32(len_buf, host_key_blob_len);
    br_sha256_update(&sha, len_buf, 4);
    br_sha256_update(&sha, host_key_blob, host_key_blob_len);
    /* Q_C (client ephemeral) */
    dbg_hex("H: Q_C=", client_pub_copy, 32);
    put_u32(len_buf, 32);
    br_sha256_update(&sha, len_buf, 4);
    br_sha256_update(&sha, client_pub_copy, 32);
    /* Q_S (server ephemeral) */
    dbg_hex("H: Q_S=", my_pub, 32);
    put_u32(len_buf, 32);
    br_sha256_update(&sha, len_buf, 4);
    br_sha256_update(&sha, my_pub, 32);
    /* K (shared secret as mpint) */
    dbg_hex("H: K=", shared_secret, 32);
    if (shared_secret[0] & 0x80) {
        dbg_int("H: K mpint len=", 33);
        put_u32(len_buf, 33);
        br_sha256_update(&sha, len_buf, 4);
        uint8_t zero = 0;
        br_sha256_update(&sha, &zero, 1);
        br_sha256_update(&sha, shared_secret, 32);
    } else {
        int start = 0;
        while (start < 31 && shared_secret[start] == 0) start++;
        uint32_t ss_len2 = 32 - start;
        if ((shared_secret[start] & 0x80) != 0) {
            /* Need 0x00 prefix for positive mpint */
            dbg_int("H: K mpint len(+pad)=", (int)(ss_len2 + 1));
            put_u32(len_buf, ss_len2 + 1);
            br_sha256_update(&sha, len_buf, 4);
            uint8_t zero = 0;
            br_sha256_update(&sha, &zero, 1);
            br_sha256_update(&sha, shared_secret + start, ss_len2);
        } else {
            dbg_int("H: K mpint len=", (int)ss_len2);
            put_u32(len_buf, ss_len2);
            br_sha256_update(&sha, len_buf, 4);
            br_sha256_update(&sha, shared_secret + start, ss_len2);
        }
    }

    br_sha256_out(&sha, ctx->kex_hash);
    dbg_hex("H: hash=", ctx->kex_hash, 32);

    if (!ctx->session_id_set) {
        memcpy(ctx->session_id, ctx->kex_hash, 32);
        ctx->session_id_set = 1;
    }

    /* 7. Sign exchange hash with ECDSA-SHA256-P256 host key */
    uint8_t sig_asn1[80]; /* ECDSA signature in ASN.1 DER, max ~72 bytes */
    br_ec_private_key sk;
    sk.curve = BR_EC_secp256r1;
    sk.x = ecdsa_priv;
    sk.xlen = 32;

    size_t sig_len = br_ecdsa_i31_sign_asn1(
        ec_p256, &br_sha256_vtable, ctx->kex_hash, &sk, sig_asn1);
    dbg_int("sshd-kex: ECDSA sign len=", (int)sig_len);
    if (sig_len == 0) return SSH_ERR_KEX;
    dbg_hex("sshd-kex: sig_asn1=", sig_asn1, sig_len > 32 ? 32 : sig_len);

    /* Self-verify: confirm our signature is valid over our hash */
    {
        br_ec_public_key pk;
        pk.curve = BR_EC_secp256r1;
        pk.q = ecdsa_pub;
        pk.qlen = 65;
        uint32_t vfy = br_ecdsa_i31_vrfy_asn1(
            ec_p256, ctx->kex_hash, 32, &pk, sig_asn1, sig_len);
        dbg_int("sshd-kex: self-verify=", (int)vfy);
    }

    /* Convert ASN.1 DER signature to SSH format: mpint(r) || mpint(s) per RFC 5656 */
    /* DER: 30 <len> 02 <r_len> <r_bytes> 02 <s_len> <s_bytes> */
    uint8_t sig_ssh[128];
    uint32_t sig_ssh_len = 0;
    {
        const uint8_t *d = sig_asn1;
        if (d[0] != 0x30) return SSH_ERR_KEX;
        size_t dp = 2; /* skip SEQUENCE tag + length */
        if (d[dp] != 0x02) return SSH_ERR_KEX;
        dp++;
        uint8_t r_len = d[dp++];
        const uint8_t *r_data = &d[dp];
        dp += r_len;
        if (d[dp] != 0x02) return SSH_ERR_KEX;
        dp++;
        uint8_t s_len = d[dp++];
        const uint8_t *s_data = &d[dp];

        /* Write mpint(r) || mpint(s) */
        put_u32(sig_ssh, r_len); sig_ssh_len = 4;
        memcpy(sig_ssh + sig_ssh_len, r_data, r_len); sig_ssh_len += r_len;
        put_u32(sig_ssh + sig_ssh_len, s_len); sig_ssh_len += 4;
        memcpy(sig_ssh + sig_ssh_len, s_data, s_len); sig_ssh_len += s_len;
    }

    /* Build signature blob: string("ecdsa-sha2-nistp256") + string(mpint_r || mpint_s) */
    uint8_t sig_blob[256];
    uint32_t sig_off = 0;
    put_cstring(sig_blob, "ecdsa-sha2-nistp256", &sig_off);
    put_string(sig_blob, sig_ssh, sig_ssh_len, &sig_off);

    /* 8. Send ECDH_REPLY: K_S + Q_S + signature */
    uint32_t reply_len = 1 + (4 + host_key_blob_len) + (4 + 32) + (4 + sig_off);
    uint8_t *reply = (uint8_t *)malloc(reply_len);
    if (!reply) return SSH_ERR_ALLOC;

    off = 0;
    reply[off++] = SSH_MSG_KEX_ECDH_REPLY;
    put_string(reply, host_key_blob, host_key_blob_len, &off);
    put_string(reply, my_pub, 32, &off);
    put_string(reply, sig_blob, sig_off, &off);

    dbg_int("sshd-kex: ECDH_REPLY payload size=", (int)off);
    rc = ssh_send_packet(ctx, reply, off);
    free(reply);
    dbg_int("sshd-kex: ECDH_REPLY send rc=", rc);
    if (rc != SSH_OK) return rc;

    /* 9. Send NEWKEYS */
    uint8_t newkeys = SSH_MSG_NEWKEYS;
    rc = ssh_send_packet(ctx, &newkeys, 1);
    dbg_int("sshd-kex: NEWKEYS send rc=", rc);
    if (rc != SSH_OK) return rc;

    /* 10. Receive NEWKEYS */
    dbg("sshd-kex: waiting for client NEWKEYS...\n");
    rc = ssh_recv_packet(ctx);
    dbg_int("sshd-kex: recv NEWKEYS rc=", rc);
    if (rc < 0) return rc;
    if (rc != SSH_MSG_NEWKEYS) return SSH_ERR_PROTO;

    /* 11. Derive encryption keys */
    int ss_start = 0;
    while (ss_start < 31 && shared_secret[ss_start] == 0) ss_start++;
    uint32_t ss_len = 32 - ss_start;

    derive_key(shared_secret + ss_start, ss_len, ctx->kex_hash, ctx->session_id,
               'A', 16, ctx->iv_c2s);
    derive_key(shared_secret + ss_start, ss_len, ctx->kex_hash, ctx->session_id,
               'B', 16, ctx->iv_s2c);
    derive_key(shared_secret + ss_start, ss_len, ctx->kex_hash, ctx->session_id,
               'C', 16, ctx->key_c2s);
    derive_key(shared_secret + ss_start, ss_len, ctx->kex_hash, ctx->session_id,
               'D', 16, ctx->key_s2c);
    derive_key(shared_secret + ss_start, ss_len, ctx->kex_hash, ctx->session_id,
               'E', 32, ctx->mac_c2s);
    derive_key(shared_secret + ss_start, ss_len, ctx->kex_hash, ctx->session_id,
               'F', 32, ctx->mac_s2c);

    ctx->encrypted = 1;
    /* RFC 4253 Section 6.4: sequence numbers must NEVER be reset, even after rekey */
    dbg("sshd-kex: KEX complete, encryption enabled\n");

    memset(my_priv, 0, 32);
    memset(shared_secret, 0, 32);
    memset(ecdsa_priv, 0, 32);

    return SSH_OK;
}

/* =========================================================================
 * Server-side Authentication (RFC 4252)
 * ========================================================================= */

int ssh_server_auth(ssh_ctx_t *ctx, char *user_buf, uint32_t user_buf_len,
                    char *pass_buf, uint32_t pass_buf_len) {
    int rc;

    /* Receive SERVICE_REQUEST for ssh-userauth */
    rc = ssh_recv_packet(ctx);
    if (rc < 0) return rc;
    if (rc != SSH_MSG_SERVICE_REQUEST) return SSH_ERR_PROTO;

    /* Send SERVICE_ACCEPT */
    uint8_t accept[64];
    uint32_t off = 0;
    accept[off++] = SSH_MSG_SERVICE_ACCEPT;
    put_cstring(accept, "ssh-userauth", &off);
    rc = ssh_send_packet(ctx, accept, off);
    if (rc != SSH_OK) return rc;

    /* Receive USERAUTH_REQUEST(s) — OpenSSH sends "none" first, then "password" */
    for (int auth_attempts = 0; auth_attempts < 5; auth_attempts++) {
        rc = ssh_recv_packet(ctx);
        if (rc < 0) return rc;
        if (rc != SSH_MSG_USERAUTH_REQUEST) return SSH_ERR_PROTO;

        /* Parse: username + service + method + ... */
        off = 1; /* skip type byte */
        const uint8_t *username;
        uint32_t username_len;
        if (get_string(ctx->rbuf, ctx->rbuf_len, &off, &username, &username_len) < 0)
            return SSH_ERR_PROTO;

        const uint8_t *service;
        uint32_t service_len;
        if (get_string(ctx->rbuf, ctx->rbuf_len, &off, &service, &service_len) < 0)
            return SSH_ERR_PROTO;

        const uint8_t *method;
        uint32_t method_len;
        if (get_string(ctx->rbuf, ctx->rbuf_len, &off, &method, &method_len) < 0)
            return SSH_ERR_PROTO;

        /* If method is "password", extract credentials and return */
        if (method_len == 8 && memcmp(method, "password", 8) == 0) {
            /* Skip the boolean (FALSE = not changing password) */
            if (off >= ctx->rbuf_len) return SSH_ERR_PROTO;
            off++; /* skip boolean */

            const uint8_t *password;
            uint32_t password_len;
            if (get_string(ctx->rbuf, ctx->rbuf_len, &off, &password, &password_len) < 0)
                return SSH_ERR_PROTO;

            /* Copy to output buffers */
            uint32_t ucopy = username_len < user_buf_len - 1 ? username_len : user_buf_len - 1;
            memcpy(user_buf, username, ucopy);
            user_buf[ucopy] = '\0';

            uint32_t pcopy = password_len < pass_buf_len - 1 ? password_len : pass_buf_len - 1;
            memcpy(pass_buf, password, pcopy);
            pass_buf[pcopy] = '\0';

            return SSH_OK;
        }

        /* For "none" or other methods, send USERAUTH_FAILURE with supported methods */
        uint8_t fail[64];
        uint32_t foff = 0;
        fail[foff++] = SSH_MSG_USERAUTH_FAILURE;
        put_cstring(fail, "password", &foff); /* name-list of methods that can continue */
        fail[foff++] = 0; /* partial success = FALSE */
        rc = ssh_send_packet(ctx, fail, foff);
        if (rc != SSH_OK) return rc;
    }

    return SSH_ERR_AUTH;
}

/* =========================================================================
 * Server-side Channel Management (RFC 4254)
 * ========================================================================= */

int ssh_server_accept_channel(ssh_ctx_t *ctx) {
    int rc = ssh_recv_packet(ctx);
    if (rc < 0) return rc;
    if (rc != SSH_MSG_CHANNEL_OPEN) return SSH_ERR_PROTO;

    /* Parse channel open request */
    uint32_t off = 1;
    const uint8_t *chan_type;
    uint32_t chan_type_len;
    if (get_string(ctx->rbuf, ctx->rbuf_len, &off, &chan_type, &chan_type_len) < 0)
        return SSH_ERR_PROTO;

    uint32_t sender_channel = get_u32(ctx->rbuf + off); off += 4;
    uint32_t initial_window = get_u32(ctx->rbuf + off); off += 4;
    uint32_t max_packet = get_u32(ctx->rbuf + off); off += 4;

    ctx->remote_channel = sender_channel;
    ctx->remote_window = initial_window;
    ctx->remote_max_packet = max_packet;

    /* Send CHANNEL_OPEN_CONFIRMATION */
    uint8_t buf[64];
    off = 0;
    buf[off++] = SSH_MSG_CHANNEL_OPEN_CONFIRMATION;
    put_u32(buf + off, ctx->remote_channel); off += 4;  /* recipient channel */
    put_u32(buf + off, ctx->channel_id); off += 4;      /* sender channel */
    put_u32(buf + off, ctx->local_window); off += 4;    /* initial window */
    put_u32(buf + off, SSH_MAX_PAYLOAD); off += 4;      /* max packet */

    return ssh_send_packet(ctx, buf, off);
}

int ssh_server_accept_shell(ssh_ctx_t *ctx) {
    /* Accept channel requests until we get "shell" or "exec" */
    for (int attempts = 0; attempts < 5; attempts++) {
        int rc = ssh_recv_packet(ctx);
        if (rc < 0) return rc;

        if (rc == SSH_MSG_CHANNEL_REQUEST) {
            uint32_t off = 1;
            /* skip recipient channel */ off += 4;

            const uint8_t *req_type;
            uint32_t req_type_len;
            if (get_string(ctx->rbuf, ctx->rbuf_len, &off, &req_type, &req_type_len) < 0)
                return SSH_ERR_PROTO;

            uint8_t want_reply = 0;
            if (off < ctx->rbuf_len) want_reply = ctx->rbuf[off++];

            if (want_reply) {
                /* Send CHANNEL_SUCCESS for any request */
                uint8_t resp[8];
                uint32_t roff = 0;
                resp[roff++] = SSH_MSG_CHANNEL_SUCCESS;
                put_u32(resp + roff, ctx->remote_channel); roff += 4;
                ssh_send_packet(ctx, resp, roff);
            }

            /* Check if this is the shell/exec request */
            if ((req_type_len == 5 && memcmp(req_type, "shell", 5) == 0) ||
                (req_type_len == 4 && memcmp(req_type, "exec", 4) == 0)) {
                return SSH_OK;
            }
            /* Otherwise loop (could be pty-req, env, etc.) */
        } else if (rc == SSH_MSG_CHANNEL_WINDOW_ADJUST) {
            uint32_t off = 1 + 4;
            ctx->remote_window += get_u32(ctx->rbuf + off);
        } else {
            return SSH_ERR_PROTO;
        }
    }
    return SSH_ERR_PROTO;
}
