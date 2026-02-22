/*
 * track.c — File mtime tracking for amake
 *
 * Caches stat() results to avoid redundant syscalls during dirty detection.
 */
#include "amake.h"

/* ── Hash function (same as vars.c) ──────────────────────────────────── */

static unsigned int path_hash(const char *s) {
    unsigned int h = 5381;
    while (*s)
        h = h * 33 + (unsigned char)*s++;
    return h % HASH_BUCKETS;
}

/* ── Cache management ────────────────────────────────────────────────── */

void mtime_cache_init(MtimeCache *mc) {
    memset(mc->buckets, 0, sizeof(mc->buckets));
}

void mtime_cache_free(MtimeCache *mc) {
    int i;
    for (i = 0; i < HASH_BUCKETS; i++) {
        MtimeEntry *e = mc->buckets[i];
        while (e) {
            MtimeEntry *next = e->next;
            free(e->path);
            free(e);
            e = next;
        }
        mc->buckets[i] = NULL;
    }
}

/*
 * Get mtime for a path. Returns 0 if file does not exist.
 * Results are cached per session.
 */
time_t mtime_get(MtimeCache *mc, const char *path) {
    unsigned int h = path_hash(path);
    MtimeEntry *e = mc->buckets[h];

    /* Check cache */
    while (e) {
        if (strcmp(e->path, path) == 0)
            return e->mtime;
        e = e->next;
    }

    /* Cache miss — stat the file */
    struct stat st;
    time_t mt = 0;
    if (stat(path, &st) == 0) {
        mt = st.st_mtime;
    }

    /* Cache the result */
    e = amake_malloc(sizeof(MtimeEntry));
    e->path = amake_strdup(path);
    e->mtime = mt;
    e->valid = 1;
    e->next = mc->buckets[h];
    mc->buckets[h] = e;

    return mt;
}
