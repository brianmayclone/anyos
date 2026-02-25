/*
 * BearSSL TLS stream backend for libgit2 on anyOS
 * Implements the git_stream interface using BearSSL for HTTPS support.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <netdb.h>
#include <unistd.h>
#include <errno.h>

#include <git2.h>
#include <git2/sys/stream.h>
#include <git2/sys/errors.h>

#include <bearssl.h>

/*
 * No-check X.509 verifier: accepts any server certificate without validation.
 *
 * anyOS has no CA certificate store, so we skip chain verification entirely
 * (equivalent to `curl --insecure`).  We still need to decode the server's
 * end-entity certificate to extract its public key, which BearSSL requires to
 * complete the TLS key-exchange.
 *
 * Implementation follows the br_x509_class vtable contract:
 *   start_chain → start_cert → append* → end_cert → end_chain → get_pkey
 */

typedef struct {
    const br_x509_class *vtable;
    br_x509_decoder_context decoder;
    br_x509_pkey            pkey;
    int                     pkey_valid;
    int                     first_cert;
} br_x509_nocheck_context;

static void nocheck_start_chain(const br_x509_class **ctx,
                                const char *server_name)
{
    br_x509_nocheck_context *nc = (br_x509_nocheck_context *)ctx;
    (void)server_name;
    /*
     * BearSSL contract: start_chain should reinitialise the context.
     * The vtable is already set in bearssl_connect() before the handshake;
     * we only need to reset the per-chain state here.
     */
    nc->first_cert = 1;
    nc->pkey_valid = 0;
}

static void nocheck_start_cert(const br_x509_class **ctx, uint32_t length)
{
    br_x509_nocheck_context *nc = (br_x509_nocheck_context *)ctx;
    (void)length;
    /* Only decode the end-entity (first) certificate. */
    if (nc->first_cert)
        br_x509_decoder_init(&nc->decoder, NULL, NULL);
}

static void nocheck_append(const br_x509_class **ctx,
                           const unsigned char *buf, size_t len)
{
    br_x509_nocheck_context *nc = (br_x509_nocheck_context *)ctx;
    if (nc->first_cert)
        br_x509_decoder_push(&nc->decoder, buf, len);
}

static void nocheck_end_cert(const br_x509_class **ctx)
{
    br_x509_nocheck_context *nc = (br_x509_nocheck_context *)ctx;
    if (nc->first_cert) {
        /*
         * Extract the server's public key.  The pkey pointers (modulus bytes,
         * etc.) remain valid for the lifetime of nc->decoder, which is
         * embedded in bearssl_stream and lives until bearssl_free().
         */
        const br_x509_pkey *pk = br_x509_decoder_get_pkey(&nc->decoder);
        if (pk) {
            nc->pkey = *pk;
            nc->pkey_valid = 1;
            fprintf(stderr, "[bearssl] cert pkey extracted (key_type=%d)\n",
                    (int)pk->key_type);
        } else {
            int derr = br_x509_decoder_last_error(&nc->decoder);
            fprintf(stderr, "[bearssl] cert pkey extraction FAILED (decoder err=%d)\n", derr);
        }
        nc->first_cert = 0;
    }
}

static unsigned nocheck_end_chain(const br_x509_class **ctx)
{
    /* Always succeed: skip all chain/trust-anchor validation. */
    (void)ctx;
    return 0;
}

static const br_x509_pkey *nocheck_get_pkey(const br_x509_class *const *ctx,
                                            unsigned *usages)
{
    const br_x509_nocheck_context *nc = (const br_x509_nocheck_context *)ctx;
    if (usages != NULL)
        *usages = BR_KEYTYPE_KEYX | BR_KEYTYPE_SIGN;
    if (!nc->pkey_valid) {
        fprintf(stderr, "[bearssl] get_pkey: no valid key (handshake will fail)\n");
    }
    return nc->pkey_valid ? &nc->pkey : NULL;
}

static const br_x509_class nocheck_vtable = {
    sizeof(br_x509_nocheck_context),
    nocheck_start_chain,
    nocheck_start_cert,
    nocheck_append,
    nocheck_end_cert,
    nocheck_end_chain,
    nocheck_get_pkey
};

/* ---------------------------------------------------------------------------
 * BearSSL git_stream implementation
 * --------------------------------------------------------------------------- */

typedef struct {
    git_stream            parent;
    char                 *host;
    char                 *port;
    int                   socket;
    br_ssl_client_context sc;
    br_x509_minimal_context xc_min; /* passed to br_ssl_client_init_full for
                                       cipher suite / PRNG setup, then
                                       overridden by xc below */
    br_x509_nocheck_context xc;     /* actual X.509 handler: accept-all */
    unsigned char         iobuf[BR_SSL_BUFSIZE_BIDI];
    br_sslio_context      ioc;
    int                   connected;
} bearssl_stream;

/* Low-level socket I/O callbacks used by br_sslio_init() */
static int sock_read(void *ctx, unsigned char *buf, size_t len)
{
    int fd = *(int *)ctx;
    ssize_t n = recv(fd, buf, len, 0);
    if (n <= 0) return -1;
    return (int)n;
}

static int sock_write(void *ctx, const unsigned char *buf, size_t len)
{
    int fd = *(int *)ctx;
    ssize_t n = send(fd, buf, len, 0);
    if (n <= 0) return -1;
    return (int)n;
}

static int bearssl_connect(git_stream *stream)
{
    bearssl_stream *bs = (bearssl_stream *)stream;
    struct addrinfo hints, *res = NULL;
    int err;

    fprintf(stderr, "[bearssl] connecting to %s:%s\n", bs->host, bs->port);

    memset(&hints, 0, sizeof(hints));
    hints.ai_family   = AF_INET;
    hints.ai_socktype = SOCK_STREAM;

    err = getaddrinfo(bs->host, bs->port, &hints, &res);
    if (err != 0 || !res) {
        fprintf(stderr, "[bearssl] DNS failed for %s: err=%d %s\n",
                bs->host, err, gai_strerror(err));
        git_error_set(GIT_ERROR_NET,
            "Failed to resolve host '%s': %s", bs->host, gai_strerror(err));
        return -1;
    }
    fprintf(stderr, "[bearssl] DNS OK for %s\n", bs->host);

    bs->socket = socket(res->ai_family, res->ai_socktype, res->ai_protocol);
    if (bs->socket < 0) {
        fprintf(stderr, "[bearssl] socket() failed: %s\n", strerror(errno));
        git_error_set(GIT_ERROR_NET,
            "Failed to create socket: %s", strerror(errno));
        freeaddrinfo(res);
        return -1;
    }

    if (connect(bs->socket, res->ai_addr, res->ai_addrlen) < 0) {
        fprintf(stderr, "[bearssl] connect() failed: %s\n", strerror(errno));
        git_error_set(GIT_ERROR_NET,
            "Failed to connect to '%s:%s': %s",
            bs->host, bs->port, strerror(errno));
        close(bs->socket);
        bs->socket = -1;
        freeaddrinfo(res);
        return -1;
    }
    freeaddrinfo(res);
    fprintf(stderr, "[bearssl] TCP connected to %s:%s (fd=%d)\n",
            bs->host, bs->port, bs->socket);

    /*
     * Initialize the TLS engine.  br_ssl_client_init_full sets up the cipher
     * suites and PRNG using xc_min; we then replace the X.509 verifier with
     * our no-check implementation so the handshake succeeds without a CA store.
     */
    br_ssl_client_init_full(&bs->sc, &bs->xc_min, NULL, 0);
    bs->xc.vtable = &nocheck_vtable;
    br_ssl_engine_set_x509(&bs->sc.eng, &bs->xc.vtable);

    br_ssl_engine_set_buffer(&bs->sc.eng, bs->iobuf, sizeof(bs->iobuf), 1);
    br_ssl_client_reset(&bs->sc, bs->host, 0);

    br_sslio_init(&bs->ioc, &bs->sc.eng,
                  sock_read,  &bs->socket,
                  sock_write, &bs->socket);

    bs->connected = 1;
    fprintf(stderr, "[bearssl] TLS engine initialized, handshake deferred\n");
    return 0;
}

static int bearssl_certificate(git_cert **out, git_stream *stream)
{
    (void)out; (void)stream;
    /* Certificate trust check is handled at the BearSSL engine level;
     * the no-check verifier above always reports success. */
    return 0;
}

static ssize_t bearssl_read(git_stream *stream, void *data, size_t len)
{
    bearssl_stream *bs = (bearssl_stream *)stream;
    int n = br_sslio_read(&bs->ioc, data, len);
    if (n <= 0) {
        /* n == 0 means clean close, n < 0 means error. Both are fatal. */
        int ssl_err = (int)br_ssl_engine_last_error(&bs->sc.eng);
        fprintf(stderr, "[bearssl] read failed n=%d BearSSL_err=%d\n",
                n, ssl_err);
        git_error_set(GIT_ERROR_SSL,
            "TLS read failed (BearSSL error %d)", ssl_err);
        return -1;
    }
    return (ssize_t)n;
}

static ssize_t bearssl_write(git_stream *stream, const char *data, size_t len,
                             int flags)
{
    bearssl_stream *bs = (bearssl_stream *)stream;
    (void)flags;

    int n = br_sslio_write_all(&bs->ioc, data, len);
    if (n < 0) {
        int ssl_err = (int)br_ssl_engine_last_error(&bs->sc.eng);
        fprintf(stderr, "[bearssl] write_all failed BearSSL_err=%d\n", ssl_err);
        git_error_set(GIT_ERROR_SSL,
            "TLS write failed (BearSSL error %d)", ssl_err);
        return -1;
    }

    if (br_sslio_flush(&bs->ioc) < 0) {
        int ssl_err = (int)br_ssl_engine_last_error(&bs->sc.eng);
        fprintf(stderr, "[bearssl] flush failed BearSSL_err=%d\n", ssl_err);
        git_error_set(GIT_ERROR_SSL,
            "TLS flush failed (BearSSL error %d)", ssl_err);
        return -1;
    }

    fprintf(stderr, "[bearssl] write OK (%zu bytes)\n", len);
    return (ssize_t)len;
}

static int bearssl_close(git_stream *stream)
{
    bearssl_stream *bs = (bearssl_stream *)stream;
    if (bs->connected) {
        br_sslio_close(&bs->ioc);
        close(bs->socket);
        bs->connected = 0;
    }
    return 0;
}

static void bearssl_free(git_stream *stream)
{
    bearssl_stream *bs = (bearssl_stream *)stream;
    if (bs->connected) bearssl_close(stream);
    free(bs->host);
    free(bs->port);
    free(bs);
}

int bearssl_stream_new(git_stream **out, const char *host, const char *port)
{
    fprintf(stderr, "[bearssl] stream_new(%s, %s)\n",
            host ? host : "(null)", port ? port : "(null)");

    bearssl_stream *bs = calloc(1, sizeof(bearssl_stream));
    if (!bs) {
        git_error_set_oom();
        return -1;
    }

    bs->host = strdup(host);
    bs->port = strdup(port ? port : "443");
    if (!bs->host || !bs->port) {
        free(bs->host);
        free(bs->port);
        free(bs);
        git_error_set_oom();
        return -1;
    }

    bs->parent.version       = GIT_STREAM_VERSION;
    bs->parent.encrypted     = 1;
    bs->parent.proxy_support = 0;
    bs->parent.connect       = bearssl_connect;
    bs->parent.certificate   = bearssl_certificate;
    bs->parent.read          = bearssl_read;
    bs->parent.write         = bearssl_write;
    bs->parent.close         = bearssl_close;
    bs->parent.free          = bearssl_free;
    bs->socket = -1;

    *out = &bs->parent;
    return 0;
}

/** Register BearSSL as the TLS stream provider for libgit2. */
void bearssl_stream_register(void)
{
    int ret = git_stream_register_tls(bearssl_stream_new);
    fprintf(stderr, "[bearssl] stream registered (ret=%d)\n", ret);
}
