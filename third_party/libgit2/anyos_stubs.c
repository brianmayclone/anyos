/*
 * Stub symbols for libgit2 on anyOS — features we don't support.
 */

/* SSH transport stubs (no libssh2) */
int git_transport_ssh_libssh2_global_init(void) { return 0; }

/* TLS stream stubs — BearSSL registered at runtime, not via these global inits */
int git_openssl_stream_global_init(void) { return 0; }
int git_mbedtls_stream_global_init(void) { return 0; }

/* Fail allocator stubs (for debug/testing, never used in release) */
void *git_failalloc_malloc(unsigned int n, const char *f, int l) {
    (void)n; (void)f; (void)l;
    return (void *)0;
}

void *git_failalloc_realloc(void *p, unsigned int n, const char *f, int l) {
    (void)p; (void)n; (void)f; (void)l;
    return (void *)0;
}

void git_failalloc_free(void *p) {
    (void)p;
}
