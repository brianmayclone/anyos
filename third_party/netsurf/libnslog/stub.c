/* Minimal libnslog stub for anyOS - logging is a no-op */
#include "include/nslog/nslog.h"

nslog_error nslog_set_render_callback(nslog_callback cb, void *context) {
    (void)cb; (void)context;
    return NSLOG_NO_ERROR;
}

nslog_error nslog_uncork(void) {
    return NSLOG_NO_ERROR;
}

nslog_error nslog_filter_category_new(const char *catname, nslog_filter_t **filter) {
    (void)catname; *filter = (void*)0; return NSLOG_NO_ERROR;
}

nslog_error nslog_filter_level_new(nslog_level level, nslog_filter_t **filter) {
    (void)level; *filter = (void*)0; return NSLOG_NO_ERROR;
}

nslog_error nslog_filter_filename_new(const char *filename, nslog_filter_t **filter) {
    (void)filename; *filter = (void*)0; return NSLOG_NO_ERROR;
}

nslog_error nslog_filter_dirname_new(const char *dirname, nslog_filter_t **filter) {
    (void)dirname; *filter = (void*)0; return NSLOG_NO_ERROR;
}

nslog_error nslog_filter_funcname_new(const char *funcname, nslog_filter_t **filter) {
    (void)funcname; *filter = (void*)0; return NSLOG_NO_ERROR;
}

nslog_error nslog_filter_and_new(nslog_filter_t *left, nslog_filter_t *right, nslog_filter_t **filter) {
    (void)left; (void)right; *filter = (void*)0; return NSLOG_NO_ERROR;
}

nslog_error nslog_filter_or_new(nslog_filter_t *left, nslog_filter_t *right, nslog_filter_t **filter) {
    (void)left; (void)right; *filter = (void*)0; return NSLOG_NO_ERROR;
}

nslog_error nslog_filter_xor_new(nslog_filter_t *left, nslog_filter_t *right, nslog_filter_t **filter) {
    (void)left; (void)right; *filter = (void*)0; return NSLOG_NO_ERROR;
}

nslog_error nslog_filter_not_new(nslog_filter_t *input, nslog_filter_t **filter) {
    (void)input; *filter = (void*)0; return NSLOG_NO_ERROR;
}

nslog_filter_t *nslog_filter_ref(nslog_filter_t *filter) {
    return filter;
}

nslog_filter_t *nslog_filter_unref(nslog_filter_t *filter) {
    (void)filter;
    return (void*)0;
}

nslog_error nslog_filter_set_active(nslog_filter_t *filter, nslog_filter_t **prev) {
    (void)filter;
    if (prev) *prev = (void*)0;
    return NSLOG_NO_ERROR;
}

nslog_error nslog_filter_from_text(const char *input, nslog_filter_t **filter) {
    (void)input; *filter = (void*)0; return NSLOG_NO_ERROR;
}

/* The core __nslog function that NSLOG() macros call */
void __nslog(nslog_entry_context_t *ctx, const char *pattern, ...) {
    (void)ctx; (void)pattern;
    /* No-op: bare metal, no logging */
}
