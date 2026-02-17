/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * anyos_tls.c -- TLS client wrapper for anyOS using BearSSL.
 *
 * Provides a simple high-level API: tls_connect / tls_send / tls_recv / tls_close.
 * Uses a "trust-all" X.509 validator (no certificate chain verification).
 */

#include "bearssl.h"

/* Forward declarations for Rust-side functions */
extern int anyos_tcp_send(int fd, const void *data, int len);
extern int anyos_tcp_recv(int fd, void *data, int len);
extern void anyos_sleep(int ms);
extern int anyos_random(void *buf, int len);

/* -------------------------------------------------------------------------- */
/* Trust-all X.509 validator                                                  */
/* -------------------------------------------------------------------------- */

/*
 * This validator extracts the server's public key from the first certificate
 * (end-entity) without performing any chain or signature validation.
 * For use in environments without a trust store (hobby OS, testing, etc.).
 */

typedef struct {
    const br_x509_class *vtable;
    br_x509_decoder_context decoder;
    /* Local storage for the public key data. */
    unsigned char key_data[1024];
    size_t key_data_len;
    br_x509_pkey pkey;
    int got_pkey;
    int first_cert;
} trust_all_x509_ctx;

static void ta_start_chain(const br_x509_class **ctx, const char *name)
{
    trust_all_x509_ctx *tc = (trust_all_x509_ctx *)(void *)ctx;
    (void)name;
    tc->got_pkey = 0;
    tc->first_cert = 1;
    tc->key_data_len = 0;
}

static void ta_start_cert(const br_x509_class **ctx, uint32_t length)
{
    trust_all_x509_ctx *tc = (trust_all_x509_ctx *)(void *)ctx;
    (void)length;
    if (tc->first_cert) {
        br_x509_decoder_init(&tc->decoder, 0, 0);
    }
}

static void ta_append(const br_x509_class **ctx,
    const unsigned char *buf, size_t len)
{
    trust_all_x509_ctx *tc = (trust_all_x509_ctx *)(void *)ctx;
    if (tc->first_cert) {
        br_x509_decoder_push(&tc->decoder, buf, len);
    }
}

static void ta_end_cert(const br_x509_class **ctx)
{
    trust_all_x509_ctx *tc = (trust_all_x509_ctx *)(void *)ctx;
    if (tc->first_cert && !tc->got_pkey) {
        const br_x509_pkey *pk = br_x509_decoder_get_pkey(&tc->decoder);
        if (pk != 0) {
            /* Copy key type */
            tc->pkey.key_type = pk->key_type;

            /* Copy key data into our local buffer */
            size_t off = 0;
            if (pk->key_type == BR_KEYTYPE_RSA) {
                size_t nlen = pk->key.rsa.nlen;
                size_t elen = pk->key.rsa.elen;
                if (off + nlen + elen <= sizeof(tc->key_data)) {
                    memcpy(tc->key_data + off, pk->key.rsa.n, nlen);
                    tc->pkey.key.rsa.n = tc->key_data + off;
                    tc->pkey.key.rsa.nlen = nlen;
                    off += nlen;
                    memcpy(tc->key_data + off, pk->key.rsa.e, elen);
                    tc->pkey.key.rsa.e = tc->key_data + off;
                    tc->pkey.key.rsa.elen = elen;
                    off += elen;
                    tc->got_pkey = 1;
                }
            } else if (pk->key_type == BR_KEYTYPE_EC) {
                size_t qlen = pk->key.ec.qlen;
                if (off + qlen <= sizeof(tc->key_data)) {
                    memcpy(tc->key_data + off, pk->key.ec.q, qlen);
                    tc->pkey.key.ec.curve = pk->key.ec.curve;
                    tc->pkey.key.ec.q = tc->key_data + off;
                    tc->pkey.key.ec.qlen = qlen;
                    off += qlen;
                    tc->got_pkey = 1;
                }
            }
            tc->key_data_len = off;
        }
        tc->first_cert = 0;
    }
}

static unsigned ta_end_chain(const br_x509_class **ctx)
{
    (void)ctx;
    return 0; /* 0 = success: trust everything */
}

static const br_x509_pkey *ta_get_pkey(
    const br_x509_class *const *ctx, unsigned *usages)
{
    trust_all_x509_ctx *tc = (trust_all_x509_ctx *)(void *)ctx;
    if (usages) {
        *usages = BR_KEYTYPE_KEYX | BR_KEYTYPE_SIGN;
    }
    return tc->got_pkey ? &tc->pkey : 0;
}

static const br_x509_class trust_all_vtable = {
    sizeof(trust_all_x509_ctx),
    ta_start_chain,
    ta_start_cert,
    ta_append,
    ta_end_cert,
    ta_end_chain,
    ta_get_pkey
};

/* -------------------------------------------------------------------------- */
/* Low-level I/O callbacks for BearSSL                                        */
/* -------------------------------------------------------------------------- */

static int low_read(void *ctx, unsigned char *buf, size_t len)
{
    int fd = *(int *)ctx;
    int total = 0;
    int retries = 0;

    while (total == 0) {
        int n = anyos_tcp_recv(fd, buf, (int)len);
        if (n < 0) return -1;
        if (n > 0) return n;
        /* n == 0: no data yet — retry with brief sleep */
        anyos_sleep(1);
        retries++;
        if (retries > 10000) return -1; /* 10s timeout */
    }
    return total;
}

static int low_write(void *ctx, const unsigned char *buf, size_t len)
{
    int fd = *(int *)ctx;
    int total = 0;

    while ((size_t)total < len) {
        int n = anyos_tcp_send(fd, buf + total, (int)(len - (size_t)total));
        if (n < 0) return -1;
        if (n == 0) {
            anyos_sleep(1);
            continue;
        }
        total += n;
    }
    return total;
}

/* -------------------------------------------------------------------------- */
/* TLS state (single connection at a time)                                    */
/* -------------------------------------------------------------------------- */

static br_ssl_client_context sc;
static br_x509_minimal_context xc; /* used internally by init_full */
static trust_all_x509_ctx ta_ctx;
static unsigned char iobuf[BR_SSL_BUFSIZE_BIDI];
static br_sslio_context ioc;
static int tls_fd_storage;

/* -------------------------------------------------------------------------- */
/* Public API                                                                 */
/* -------------------------------------------------------------------------- */

/*
 * Establish a TLS connection over an existing TCP socket.
 * Returns 0 on success, -1 on failure.
 * `host` must be a null-terminated hostname string (used for SNI).
 */
int tls_connect(int fd, const char *host)
{
    tls_fd_storage = fd;

    /* Initialize BearSSL client with full cipher suite support. */
    br_ssl_client_init_full(&sc, &xc, 0, 0);

    /* Seed the PRNG with entropy from the kernel RNG (RDRAND/TSC). */
    {
        unsigned char entropy[32];
        anyos_random(entropy, 32);
        br_ssl_engine_inject_entropy(&sc.eng, entropy, sizeof entropy);
    }

    /* Override X.509 engine with trust-all validator. */
    ta_ctx.vtable = &trust_all_vtable;
    br_ssl_engine_set_x509(&sc.eng, &ta_ctx.vtable);

    /* Set I/O buffer. */
    br_ssl_engine_set_buffer(&sc.eng, iobuf, sizeof iobuf, 1);

    /* Reset the client context for a new connection. */
    br_ssl_client_reset(&sc, host, 0);

    /* Initialize the sslio wrapper with our I/O callbacks. */
    br_sslio_init(&ioc, &sc.eng,
        low_read, &tls_fd_storage,
        low_write, &tls_fd_storage);

    /* The handshake happens lazily on first read/write. Force it. */
    br_sslio_flush(&ioc);

    /* Check for errors. */
    int err = br_ssl_engine_last_error(&sc.eng);
    if (err != BR_ERR_OK) {
        return -err; /* return negative BearSSL error code */
    }

    return 0;
}

/*
 * Send data over the TLS connection.
 * Returns number of bytes sent on success, -1 on failure.
 */
int tls_send(const void *data, int len)
{
    int ret = br_sslio_write_all(&ioc, data, (size_t)len);
    if (ret < 0) return -1;
    ret = br_sslio_flush(&ioc);
    if (ret < 0) return -1;
    return len;
}

/*
 * Receive data from the TLS connection.
 * Returns number of bytes read, 0 on EOF, -1 on error.
 */
int tls_recv(void *data, int len)
{
    int ret = br_sslio_read(&ioc, data, (size_t)len);
    if (ret < 0) {
        int err = br_ssl_engine_last_error(&sc.eng);
        if (err == BR_ERR_OK) return 0; /* clean close */
        return -1;
    }
    return ret;
}

/*
 * Close the TLS connection (sends close_notify).
 */
void tls_close(void)
{
    br_sslio_close(&ioc);
}

/*
 * Get last BearSSL error code.
 */
int tls_last_error(void)
{
    return br_ssl_engine_last_error(&sc.eng);
}
