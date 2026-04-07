/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * anyos_tls.c -- TLS client wrapper for anyOS using BearSSL.
 *
 * Provides a simple high-level API: tls_connect / tls_send / tls_recv /
 * tls_close using per-connection handles instead of a single global context.
 * Uses a "trust-all" X.509 validator (no certificate chain verification).
 */

#include "bearssl.h"
#include <string.h>

/* Forward declarations for Rust-side functions */
extern int anyos_tcp_send(int fd, const void *data, int len);
extern int anyos_tcp_recv(int fd, void *data, int len);
extern void anyos_sleep(int ms);
extern int anyos_random(void *buf, int len);

/* -------------------------------------------------------------------------- */
/* Trust-all X.509 validator                                                  */
/* -------------------------------------------------------------------------- */

typedef struct {
    const br_x509_class *vtable;
    br_x509_decoder_context decoder;
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

static void ta_append(const br_x509_class **ctx, const unsigned char *buf, size_t len)
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
            size_t off = 0;
            tc->pkey.key_type = pk->key_type;

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
    return 0;
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
/* TLS state (multiple concurrent connections)                                */
/* -------------------------------------------------------------------------- */

#define MAX_TLS_CONTEXTS 16

typedef struct {
    int used;
    int fd;
    br_ssl_client_context sc;
    br_x509_minimal_context xc;
    trust_all_x509_ctx ta_ctx;
    unsigned char iobuf[BR_SSL_BUFSIZE_BIDI];
    br_sslio_context ioc;
} tls_slot;

static tls_slot TLS_SLOTS[MAX_TLS_CONTEXTS];

static tls_slot *slot_from_ctx(void *ctx)
{
    return (tls_slot *)ctx;
}

static tls_slot *get_slot(int handle)
{
    int idx;
    if (handle <= 0) {
        return 0;
    }
    idx = handle - 1;
    if (idx < 0 || idx >= MAX_TLS_CONTEXTS) {
        return 0;
    }
    if (!TLS_SLOTS[idx].used) {
        return 0;
    }
    return &TLS_SLOTS[idx];
}

static int alloc_slot(void)
{
    int i;
    for (i = 0; i < MAX_TLS_CONTEXTS; i++) {
        if (!TLS_SLOTS[i].used) {
            memset(&TLS_SLOTS[i], 0, sizeof(TLS_SLOTS[i]));
            TLS_SLOTS[i].used = 1;
            return i + 1;
        }
    }
    return 0;
}

static void free_slot(int handle)
{
    tls_slot *slot = get_slot(handle);
    if (slot == 0) {
        return;
    }
    memset(slot, 0, sizeof(*slot));
}

/* -------------------------------------------------------------------------- */
/* Low-level I/O callbacks for BearSSL                                        */
/* -------------------------------------------------------------------------- */

static int low_read(void *ctx, unsigned char *buf, size_t len)
{
    tls_slot *slot = slot_from_ctx(ctx);
    int fd = slot->fd;
    int retries = 0;
    int err_retries = 0;

    for (;;) {
        int n = anyos_tcp_recv(fd, buf, (int)len);
        if (n > 0) return n;
        if (n == 0) {
            /* No data yet — sleep briefly and retry. */
            anyos_sleep(1);
            retries++;
            if (retries > 15000) return -1; /* 15 s total timeout */
            continue;
        }
        /* n < 0 — this may be a transient TCP timeout (the kernel
           returns error after its own 3 s timeout) rather than a real
           connection failure.  Retry a few times before giving up. */
        err_retries++;
        if (err_retries > 4) return -1; /* ~12 s of TCP timeouts */
        anyos_sleep(10);
    }
}

static int low_write(void *ctx, const unsigned char *buf, size_t len)
{
    tls_slot *slot = slot_from_ctx(ctx);
    int fd = slot->fd;
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
/* Public API                                                                 */
/* -------------------------------------------------------------------------- */

int tls_connect(int fd, const char *host)
{
    int handle = alloc_slot();
    tls_slot *slot;
    int err;

    if (handle == 0) {
        return -1;
    }
    slot = get_slot(handle);
    if (slot == 0) {
        return -1;
    }

    slot->fd = fd;
    br_ssl_client_init_full(&slot->sc, &slot->xc, 0, 0);

    {
        unsigned char entropy[32];
        anyos_random(entropy, 32);
        br_ssl_engine_inject_entropy(&slot->sc.eng, entropy, sizeof entropy);
    }

    slot->ta_ctx.vtable = &trust_all_vtable;
    br_ssl_engine_set_x509(&slot->sc.eng, &slot->ta_ctx.vtable);
    br_ssl_engine_set_buffer(&slot->sc.eng, slot->iobuf, sizeof slot->iobuf, 1);
    br_ssl_client_reset(&slot->sc, host, 0);
    br_sslio_init(&slot->ioc, &slot->sc.eng, low_read, slot, low_write, slot);
    br_sslio_flush(&slot->ioc);

    err = br_ssl_engine_last_error(&slot->sc.eng);
    if (err != BR_ERR_OK) {
        free_slot(handle);
        return -err;
    }

    return handle;
}

int tls_send(int handle, const void *data, int len)
{
    tls_slot *slot = get_slot(handle);
    int ret;
    if (slot == 0) return -1;
    ret = br_sslio_write_all(&slot->ioc, data, (size_t)len);
    if (ret < 0) return -1;
    ret = br_sslio_flush(&slot->ioc);
    if (ret < 0) return -1;
    return len;
}

int tls_recv(int handle, void *data, int len)
{
    tls_slot *slot = get_slot(handle);
    int ret;
    int err;
    if (slot == 0) return -1;
    ret = br_sslio_read(&slot->ioc, data, (size_t)len);
    if (ret < 0) {
        err = br_ssl_engine_last_error(&slot->sc.eng);
        if (err == BR_ERR_OK) return 0;
        return -1;
    }
    return ret;
}

void tls_close(int handle)
{
    tls_slot *slot = get_slot(handle);
    if (slot == 0) {
        return;
    }
    br_sslio_close(&slot->ioc);
    free_slot(handle);
}

int tls_last_error(int handle)
{
    tls_slot *slot = get_slot(handle);
    if (slot == 0) {
        return -1;
    }
    return br_ssl_engine_last_error(&slot->sc.eng);
}
