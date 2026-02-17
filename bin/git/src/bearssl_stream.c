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
#include <bearssl.h>

/* Trust anchor: we skip certificate validation for now (like curl --insecure).
 * A proper implementation would embed root CA certificates. */

typedef struct {
    git_stream parent;
    char *host;
    char *port;
    int socket;
    br_ssl_client_context sc;
    br_x509_minimal_context xc;
    unsigned char iobuf[BR_SSL_BUFSIZE_BIDI];
    br_sslio_context ioc;
    int connected;
} bearssl_stream;

/* Low-level I/O callbacks for BearSSL */
static int sock_read(void *ctx, unsigned char *buf, size_t len) {
    int fd = *(int *)ctx;
    ssize_t n = recv(fd, buf, len, 0);
    if (n <= 0) return -1;
    return (int)n;
}

static int sock_write(void *ctx, const unsigned char *buf, size_t len) {
    int fd = *(int *)ctx;
    ssize_t n = send(fd, buf, len, 0);
    if (n <= 0) return -1;
    return (int)n;
}

static int bearssl_connect(git_stream *stream) {
    bearssl_stream *bs = (bearssl_stream *)stream;
    struct addrinfo hints, *res = NULL;
    int err;

    memset(&hints, 0, sizeof(hints));
    hints.ai_family = AF_INET;
    hints.ai_socktype = SOCK_STREAM;

    err = getaddrinfo(bs->host, bs->port, &hints, &res);
    if (err != 0 || !res) {
        return -1;
    }

    bs->socket = socket(res->ai_family, res->ai_socktype, res->ai_protocol);
    if (bs->socket < 0) {
        freeaddrinfo(res);
        return -1;
    }

    if (connect(bs->socket, res->ai_addr, res->ai_addrlen) < 0) {
        close(bs->socket);
        bs->socket = -1;
        freeaddrinfo(res);
        return -1;
    }
    freeaddrinfo(res);

    /* Initialize BearSSL TLS client */
    br_ssl_client_init_full(&bs->sc, &bs->xc, NULL, 0);
    br_ssl_engine_set_buffer(&bs->sc.eng, bs->iobuf, sizeof(bs->iobuf), 1);
    br_ssl_client_reset(&bs->sc, bs->host, 0);

    /* Initialize I/O wrapper */
    br_sslio_init(&bs->ioc, &bs->sc.eng,
                  sock_read, &bs->socket,
                  sock_write, &bs->socket);

    bs->connected = 1;
    return 0;
}

static int bearssl_certificate(git_cert **out, git_stream *stream) {
    (void)out; (void)stream;
    /* Skip certificate validation — anyOS has no CA store */
    return 0;
}

static ssize_t bearssl_read(git_stream *stream, void *data, size_t len) {
    bearssl_stream *bs = (bearssl_stream *)stream;
    int n = br_sslio_read(&bs->ioc, data, len);
    if (n < 0) return -1;
    return (ssize_t)n;
}

static ssize_t bearssl_write(git_stream *stream, const char *data, size_t len, int flags) {
    bearssl_stream *bs = (bearssl_stream *)stream;
    (void)flags;
    int n = br_sslio_write_all(&bs->ioc, data, len);
    if (n < 0) return -1;
    br_sslio_flush(&bs->ioc);
    return (ssize_t)len;
}

static int bearssl_close(git_stream *stream) {
    bearssl_stream *bs = (bearssl_stream *)stream;
    if (bs->connected) {
        br_sslio_close(&bs->ioc);
        close(bs->socket);
        bs->connected = 0;
    }
    return 0;
}

static void bearssl_free(git_stream *stream) {
    bearssl_stream *bs = (bearssl_stream *)stream;
    if (bs->connected) bearssl_close(stream);
    free(bs->host);
    free(bs->port);
    free(bs);
}

int bearssl_stream_new(git_stream **out, const char *host, const char *port) {
    bearssl_stream *bs = calloc(1, sizeof(bearssl_stream));
    if (!bs) return -1;

    bs->parent.version = GIT_STREAM_VERSION;
    bs->parent.encrypted = 1;
    bs->parent.proxy_support = 0;
    bs->parent.connect = bearssl_connect;
    bs->parent.certificate = bearssl_certificate;
    bs->parent.read = bearssl_read;
    bs->parent.write = bearssl_write;
    bs->parent.close = bearssl_close;
    bs->parent.free = bearssl_free;
    bs->socket = -1;
    bs->host = strdup(host);
    bs->port = strdup(port ? port : "443");

    *out = &bs->parent;
    return 0;
}

/* Call this at startup to register BearSSL as the TLS provider */
void bearssl_stream_register(void) {
    git_stream_register_tls(bearssl_stream_new);
}
