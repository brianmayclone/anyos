/*
 * glob.c — Glob pattern matching for amake
 *
 * Supports * (any chars in one segment), ? (one char), ** (recursive).
 * Used by file(GLOB) and file(GLOB_RECURSE) commands.
 */
#include "amake.h"
#include <dirent.h>

/* ── Result list helpers ─────────────────────────────────────────────── */

typedef struct {
    char **files;
    int    count;
    int    cap;
} FileList;

static void fl_init(FileList *fl) {
    fl->files = NULL;
    fl->count = 0;
    fl->cap = 0;
}

static void fl_push(FileList *fl, const char *path) {
    if (fl->count >= fl->cap) {
        fl->cap = fl->cap ? fl->cap * 2 : 64;
        fl->files = amake_realloc(fl->files, sizeof(char *) * fl->cap);
    }
    fl->files[fl->count++] = amake_strdup(path);
}

/* ── Pattern matching ────────────────────────────────────────────────── */

/*
 * Match a filename against a simple glob pattern (no path separators).
 * Supports * and ?. Not recursive (**).
 */
static int match_simple(const char *pattern, const char *name) {
    while (*pattern && *name) {
        if (*pattern == '*') {
            pattern++;
            if (!*pattern) return 1; /* trailing * matches everything */
            while (*name) {
                if (match_simple(pattern, name)) return 1;
                name++;
            }
            return match_simple(pattern, name);
        }
        if (*pattern == '?') {
            pattern++;
            name++;
            continue;
        }
        if (*pattern != *name) return 0;
        pattern++;
        name++;
    }
    /* Handle trailing * in pattern */
    while (*pattern == '*') pattern++;
    return *pattern == '\0' && *name == '\0';
}

/*
 * Extract directory and filename pattern from a glob path.
 * Returns heap-allocated strings. Sets out_recurse if path has **.
 */
static void split_glob_path(const char *glob_path, char **out_dir,
                             char **out_pattern, int *out_recurse)
{
    *out_recurse = 0;

    /* Find last path separator */
    const char *last_sep = strrchr(glob_path, '/');
    if (!last_sep) {
        *out_dir = amake_strdup(".");
        *out_pattern = amake_strdup(glob_path);
        return;
    }

    *out_pattern = amake_strdup(last_sep + 1);
    *out_dir = amake_strndup(glob_path, (size_t)(last_sep - glob_path));

    /* Check if directory contains ** */
    if (strstr(*out_dir, "**")) {
        *out_recurse = 1;
        /* Remove the ** segment from directory */
        char *dstar = strstr(*out_dir, "**");
        if (dstar == *out_dir) {
            free(*out_dir);
            *out_dir = amake_strdup(".");
        } else {
            /* Truncate at the ** */
            if (dstar > *out_dir && *(dstar - 1) == '/')
                dstar--;
            *dstar = '\0';
        }
    }
}

/* ── Directory scanning ──────────────────────────────────────────────── */

static void scan_dir(const char *dir, const char *pattern, int recurse,
                     FileList *fl)
{
    DIR *dp = opendir(dir);
    if (!dp) return;

    struct dirent *de;
    while ((de = readdir(dp)) != NULL) {
        /* Skip . and .. */
        if (de->d_name[0] == '.' &&
            (de->d_name[1] == '\0' ||
             (de->d_name[1] == '.' && de->d_name[2] == '\0')))
            continue;

        char *full = amake_path_join(dir, de->d_name);

        if (amake_is_directory(full)) {
            if (recurse) {
                scan_dir(full, pattern, 1, fl);
            }
        } else {
            /* Match filename against pattern */
            if (match_simple(pattern, de->d_name)) {
                fl_push(fl, full);
            }
        }
        free(full);
    }
    closedir(dp);
}

/* ── Public API ──────────────────────────────────────────────────────── */

/*
 * file(GLOB): Non-recursive glob matching.
 */
void amake_glob(const char *glob_path, char ***out_files, int *out_count) {
    FileList fl;
    fl_init(&fl);

    char *dir, *pattern;
    int recurse;
    split_glob_path(glob_path, &dir, &pattern, &recurse);

    scan_dir(dir, pattern, 0, &fl);

    free(dir);
    free(pattern);

    *out_files = fl.files;
    *out_count = fl.count;
}

/*
 * file(GLOB_RECURSE): Recursive glob matching.
 */
void amake_glob_recurse(const char *base_dir, const char *glob_path,
                         char ***out_files, int *out_count)
{
    FileList fl;
    fl_init(&fl);

    char *dir, *pattern;
    int recurse;
    split_glob_path(glob_path, &dir, &pattern, &recurse);

    /* For GLOB_RECURSE, always recurse regardless of ** */
    scan_dir(dir, pattern, 1, &fl);

    free(dir);
    free(pattern);

    *out_files = fl.files;
    *out_count = fl.count;

    (void)base_dir; /* unused — dir is extracted from pattern */
}
