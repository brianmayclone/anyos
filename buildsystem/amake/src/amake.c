/*
 * amake — anyOS build system (CMake + Ninja replacement)
 *
 * Parses CMakeLists.txt, builds a dependency graph, tracks file timestamps,
 * and executes builds in parallel. Replaces both cmake and ninja.
 *
 * Written in C99 for TCC compatibility (self-hosting on anyOS).
 *
 * Usage:
 *   amake [options] [target...]
 *   amake -E <command> [args...]
 *
 * Options:
 *   -B DIR          Build directory (default: build)
 *   -D VAR=VAL      Define variable
 *   -j N            Parallel jobs (default: 4)
 *   -f FILE         CMakeLists.txt path
 *   --clean         Force full rebuild
 *   --verbose       Show commands being executed
 *   --version       Print version
 *   --help          Print usage
 */
#include "amake.h"
#include <dirent.h>
#include <errno.h>
#include <limits.h>
#include <utime.h>

#ifdef ONE_SOURCE
/* Single-source compilation mode (for TCC on anyOS) */
#include "vars.c"
#include "lexer.c"
#include "parser.c"
#include "glob.c"
#include "track.c"
#include "eval.c"
#include "graph.c"
#include "exec.c"
#endif

/* ── Utility: fatal error ────────────────────────────────────────────── */

void amake_fatal(const char *fmt, ...) {
    va_list ap;
    fprintf(stderr, "amake: fatal: ");
    va_start(ap, fmt);
    vfprintf(stderr, fmt, ap);
    va_end(ap);
    fprintf(stderr, "\n");
    exit(1);
}

/* ── Utility: memory allocation ──────────────────────────────────────── */

void *amake_malloc(size_t n) {
    void *p = malloc(n);
    if (!p && n > 0) amake_fatal("out of memory (%zu bytes)", n);
    return p;
}

void *amake_realloc(void *p, size_t n) {
    void *q = realloc(p, n);
    if (!q && n > 0) amake_fatal("out of memory (realloc %zu bytes)", n);
    return q;
}

char *amake_strdup(const char *s) {
    if (!s) return NULL;
    size_t len = strlen(s);
    char *d = amake_malloc(len + 1);
    memcpy(d, s, len + 1);
    return d;
}

char *amake_strndup(const char *s, size_t n) {
    if (!s) return NULL;
    char *d = amake_malloc(n + 1);
    memcpy(d, s, n);
    d[n] = '\0';
    return d;
}

char *amake_sprintf(const char *fmt, ...) {
    va_list ap, ap2;
    va_start(ap, fmt);
    va_copy(ap2, ap);
    int len = vsnprintf(NULL, 0, fmt, ap);
    va_end(ap);
    char *buf = amake_malloc(len + 1);
    vsnprintf(buf, len + 1, fmt, ap2);
    va_end(ap2);
    return buf;
}

/* ── Utility: path operations ────────────────────────────────────────── */

char *amake_path_join(const char *a, const char *b) {
    if (!a || !a[0]) return amake_strdup(b);
    if (!b || !b[0]) return amake_strdup(a);
    size_t alen = strlen(a);
    size_t blen = strlen(b);
    int need_sep = (a[alen - 1] != '/');
    char *out = amake_malloc(alen + need_sep + blen + 1);
    memcpy(out, a, alen);
    if (need_sep) out[alen] = '/';
    memcpy(out + alen + need_sep, b, blen + 1);
    return out;
}

/* ── Utility: file operations ────────────────────────────────────────── */

int amake_file_exists(const char *path) {
    struct stat st;
    return stat(path, &st) == 0 && S_ISREG(st.st_mode);
}

int amake_is_directory(const char *path) {
    struct stat st;
    return stat(path, &st) == 0 && S_ISDIR(st.st_mode);
}

void amake_mkdir_p(const char *path) {
    char *tmp = amake_strdup(path);
    char *p;
    for (p = tmp + 1; *p; p++) {
        if (*p == '/') {
            *p = '\0';
            mkdir(tmp, 0755);
            *p = '/';
        }
    }
    mkdir(tmp, 0755);
    free(tmp);
}

char *amake_read_file(const char *path, size_t *out_size) {
    FILE *fp = fopen(path, "rb");
    if (!fp) return NULL;
    fseek(fp, 0, SEEK_END);
    long sz = ftell(fp);
    if (sz < 0) { fclose(fp); return NULL; }
    fseek(fp, 0, SEEK_SET);
    char *buf = amake_malloc((size_t)sz + 1);
    size_t n = fread(buf, 1, (size_t)sz, fp);
    fclose(fp);
    buf[n] = '\0';
    if (out_size) *out_size = n;
    return buf;
}

int amake_copy_file(const char *src, const char *dst) {
    FILE *in = fopen(src, "rb");
    if (!in) return -1;
    FILE *out = fopen(dst, "wb");
    if (!out) { fclose(in); return -1; }
    char buf[8192];
    size_t n;
    while ((n = fread(buf, 1, sizeof(buf), in)) > 0)
        fwrite(buf, 1, n, out);
    fclose(in);
    fclose(out);
    /* Preserve executable permission */
    struct stat st;
    if (stat(src, &st) == 0)
        chmod(dst, st.st_mode);
    return 0;
}

int amake_copy_directory(const char *src, const char *dst) {
    amake_mkdir_p(dst);
    DIR *dp = opendir(src);
    if (!dp) return -1;
    struct dirent *de;
    while ((de = readdir(dp)) != NULL) {
        if (de->d_name[0] == '.' &&
            (de->d_name[1] == '\0' ||
             (de->d_name[1] == '.' && de->d_name[2] == '\0')))
            continue;
        char *src_path = amake_path_join(src, de->d_name);
        char *dst_path = amake_path_join(dst, de->d_name);
        if (amake_is_directory(src_path))
            amake_copy_directory(src_path, dst_path);
        else
            amake_copy_file(src_path, dst_path);
        free(src_path);
        free(dst_path);
    }
    closedir(dp);
    return 0;
}

int amake_rm_rf(const char *path) {
    struct stat st;
    if (stat(path, &st) != 0) return 0; /* doesn't exist */
    if (S_ISDIR(st.st_mode)) {
        DIR *dp = opendir(path);
        if (dp) {
            struct dirent *de;
            while ((de = readdir(dp)) != NULL) {
                if (de->d_name[0] == '.' &&
                    (de->d_name[1] == '\0' ||
                     (de->d_name[1] == '.' && de->d_name[2] == '\0')))
                    continue;
                char *child = amake_path_join(path, de->d_name);
                amake_rm_rf(child);
                free(child);
            }
            closedir(dp);
        }
        return rmdir(path);
    }
    return remove(path);
}

int amake_touch(const char *path) {
    FILE *fp = fopen(path, "a");
    if (!fp) {
        fp = fopen(path, "w");
        if (!fp) return -1;
    }
    fclose(fp);
    /* Update mtime to now */
    utime(path, NULL);
    return 0;
}

/* ── Built-in -E command handler ─────────────────────────────────────── */

int amake_builtin_E(int argc, char **argv) {
    if (argc < 1) {
        fprintf(stderr, "amake -E: no command specified\n");
        return 1;
    }

    const char *cmd = argv[0];

    if (strcmp(cmd, "make_directory") == 0) {
        int i;
        for (i = 1; i < argc; i++)
            amake_mkdir_p(argv[i]);
        return 0;
    }

    if (strcmp(cmd, "copy") == 0) {
        if (argc < 3) {
            fprintf(stderr, "amake -E copy: need source and destination\n");
            return 1;
        }
        return amake_copy_file(argv[1], argv[2]) == 0 ? 0 : 1;
    }

    if (strcmp(cmd, "copy_directory") == 0) {
        if (argc < 3) {
            fprintf(stderr, "amake -E copy_directory: need source and destination\n");
            return 1;
        }
        return amake_copy_directory(argv[1], argv[2]) == 0 ? 0 : 1;
    }

    if (strcmp(cmd, "rm") == 0) {
        int i;
        int start = 1;
        /* Skip -rf flags */
        while (start < argc && argv[start][0] == '-') start++;
        for (i = start; i < argc; i++)
            amake_rm_rf(argv[i]);
        return 0;
    }

    if (strcmp(cmd, "touch") == 0) {
        int i;
        for (i = 1; i < argc; i++)
            amake_touch(argv[i]);
        return 0;
    }

    if (strcmp(cmd, "env") == 0) {
        /* env VAR=VAL... command args... */
        int i = 1;
        while (i < argc && strchr(argv[i], '=') != NULL) {
            char *dup = amake_strdup(argv[i]);
            char *eq = strchr(dup, '=');
            *eq = '\0';
            setenv(dup, eq + 1, 1);
            free(dup);
            i++;
        }
        if (i >= argc) return 0;
        /* Execute remaining as command */
#ifndef _WIN32
        execvp(argv[i], &argv[i]);
        fprintf(stderr, "amake -E env: exec failed: %s\n", argv[i]);
        return 127;
#else
        return system(argv[i]);
#endif
    }

    fprintf(stderr, "amake -E: unknown command '%s'\n", cmd);
    return 1;
}

/* ── Get number of CPUs ──────────────────────────────────────────────── */

static int get_cpu_count(void) {
#ifdef _SC_NPROCESSORS_ONLN
    long n = sysconf(_SC_NPROCESSORS_ONLN);
    if (n > 0) return (int)n;
#endif
    return 4;
}

/* ── Get absolute path ───────────────────────────────────────────────── */

static char *get_absolute_path(const char *path) {
    char resolved[PATH_MAX];
    if (realpath(path, resolved))
        return amake_strdup(resolved);
    return amake_strdup(path);
}

/* ── Usage ───────────────────────────────────────────────────────────── */

static void usage(void) {
    fprintf(stderr,
        "amake v" AMAKE_VERSION " — anyOS build system\n\n"
        "Usage: amake [options] [target...]\n"
        "       amake -E <command> [args...]\n\n"
        "Options:\n"
        "  -B DIR          Build directory (default: build)\n"
        "  -D VAR=VAL      Define variable\n"
        "  -j N            Parallel jobs (default: CPU count)\n"
        "  -f FILE         CMakeLists.txt path (default: ./CMakeLists.txt)\n"
        "  --clean         Force full rebuild\n"
        "  --verbose       Show commands being executed\n"
        "  --version       Print version\n"
        "  --help          Print this help\n\n"
        "Built-in -E commands:\n"
        "  env, make_directory, copy, copy_directory, rm, touch\n"
    );
}

/* ── Handle COMMAND-style targets (like "run") ───────────────────────── */

static int run_target_commands(AmakeCtx *ctx, const char *target_name) {
    int i;
    for (i = 0; i < ctx->graph.target_count; i++) {
        BuildTarget *tgt = ctx->graph.targets[i];
        if (strcmp(tgt->name, target_name) == 0 && tgt->command_count > 0) {
            int j;
            for (j = 0; j < tgt->command_count; j++) {
                if (ctx->verbose)
                    fprintf(stderr, "  > %s\n", tgt->commands[j]);
                int rc = system(tgt->commands[j]);
                if (rc != 0) return rc;
            }
            return 0;
        }
    }
    return 0;
}

/* ── Main ────────────────────────────────────────────────────────────── */

int main(int argc, char **argv) {
    /* Defaults */
    const char *build_dir = "build";
    const char *cmake_file = NULL;
    int max_jobs = get_cpu_count();
    int verbose = 0;
    int clean = 0;
    char **defines = NULL;
    int define_count = 0;
    char **targets = NULL;
    int target_count = 0;

    /* Parse CLI */
    int i;
    for (i = 1; i < argc; i++) {
        if (strcmp(argv[i], "-E") == 0) {
            /* Built-in command mode */
            return amake_builtin_E(argc - i - 1, argv + i + 1);
        }
        else if (strcmp(argv[i], "-B") == 0 && i + 1 < argc) {
            build_dir = argv[++i];
        }
        else if (strncmp(argv[i], "-D", 2) == 0) {
            const char *def = argv[i] + 2;
            if (*def == '\0' && i + 1 < argc) def = argv[++i];
            defines = amake_realloc(defines, sizeof(char *) * (define_count + 1));
            defines[define_count++] = amake_strdup(def);
        }
        else if (strcmp(argv[i], "-j") == 0 && i + 1 < argc) {
            max_jobs = atoi(argv[++i]);
            if (max_jobs < 1) max_jobs = 1;
        }
        else if (strncmp(argv[i], "-j", 2) == 0 && argv[i][2] >= '0' && argv[i][2] <= '9') {
            max_jobs = atoi(argv[i] + 2);
            if (max_jobs < 1) max_jobs = 1;
        }
        else if (strcmp(argv[i], "-f") == 0 && i + 1 < argc) {
            cmake_file = argv[++i];
        }
        else if (strcmp(argv[i], "--clean") == 0) {
            clean = 1;
        }
        else if (strcmp(argv[i], "--verbose") == 0 || strcmp(argv[i], "-v") == 0) {
            verbose = 1;
        }
        else if (strcmp(argv[i], "--version") == 0) {
            printf("amake v" AMAKE_VERSION "\n");
            return 0;
        }
        else if (strcmp(argv[i], "--help") == 0 || strcmp(argv[i], "-h") == 0) {
            usage();
            return 0;
        }
        else if (argv[i][0] != '-') {
            /* Target name */
            targets = amake_realloc(targets, sizeof(char *) * (target_count + 1));
            targets[target_count++] = amake_strdup(argv[i]);
        }
        else {
            fprintf(stderr, "amake: unknown option '%s'\n", argv[i]);
            usage();
            return 1;
        }
    }

    /* Clean build */
    if (clean) {
        fprintf(stderr, "Cleaning %s...\n", build_dir);
        amake_rm_rf(build_dir);
    }

    /* Find CMakeLists.txt */
    char *source_dir;
    if (cmake_file) {
        /* Extract directory from cmake file path */
        const char *sep = strrchr(cmake_file, '/');
        if (sep)
            source_dir = amake_strndup(cmake_file, (size_t)(sep - cmake_file));
        else
            source_dir = amake_strdup(".");
    } else {
        source_dir = amake_strdup(".");
        cmake_file = "CMakeLists.txt";
    }

    /* Resolve absolute paths */
    char *abs_source = get_absolute_path(source_dir);
    char *abs_build = get_absolute_path(build_dir);
    free(source_dir);

    /* Read CMakeLists.txt */
    char *cmake_path;
    if (cmake_file[0] == '/')
        cmake_path = amake_strdup(cmake_file);
    else
        cmake_path = amake_path_join(abs_source, cmake_file);

    size_t file_size;
    char *source = amake_read_file(cmake_path, &file_size);
    if (!source)
        amake_fatal("cannot read %s", cmake_path);

    /* Ensure build directory exists */
    amake_mkdir_p(abs_build);

    /* Phase 1: Tokenize */
    TokenList tokens;
    lexer_tokenize(source, file_size, &tokens);
    free(source);

    /* Phase 2: Parse */
    AstNode *ast = parser_parse(&tokens);

    /* Phase 3: Evaluate */
    AmakeCtx ctx;
    memset(&ctx, 0, sizeof(ctx));
    ctx.source_dir = abs_source;
    ctx.binary_dir = abs_build;
    ctx.amake_path = get_absolute_path(argv[0]);
    ctx.cmake_file = cmake_path;
    ctx.global_scope = scope_new(NULL);
    ctx.current_scope = ctx.global_scope;
    ctx.verbose = verbose;
    ctx.max_jobs = max_jobs;
    ctx.cli_defines = defines;
    ctx.cli_define_count = define_count;
    ctx.targets = targets;
    ctx.target_count = target_count;
    graph_init(&ctx.graph);

    eval_run(&ctx, ast);

    if (verbose) {
        fprintf(stderr, "amake: evaluated %d rules, %d targets\n",
                ctx.graph.rule_count, ctx.graph.target_count);
    }

    /* Phase 4: Link dependency graph */
    graph_link(&ctx.graph);

    /* Phase 5: Dirty detection */
    MtimeCache mc;
    mtime_cache_init(&mc);
    graph_mark_dirty(&ctx.graph, &mc);

    /* Phase 6: Collect dirty rules */
    BuildRule **dirty = NULL;
    int dirty_count = 0;

    if (target_count > 0) {
        /* Build specific targets */
        for (i = 0; i < target_count; i++) {
            BuildRule **td = NULL;
            int tc = 0;
            int rc = graph_collect_dirty_for_target(&ctx.graph, targets[i], &td, &tc);
            if (rc < 0) {
                /* Maybe it's a COMMAND target (like "run") — still need to build deps first */
                fprintf(stderr, "amake: target '%s' — checking for COMMAND target\n", targets[i]);
            }
            /* Merge into dirty list */
            int j;
            for (j = 0; j < tc; j++) {
                /* Check not already in dirty list */
                int k, dup = 0;
                for (k = 0; k < dirty_count; k++)
                    if (dirty[k] == td[j]) { dup = 1; break; }
                if (!dup) {
                    dirty = amake_realloc(dirty, sizeof(BuildRule *) * (dirty_count + 1));
                    dirty[dirty_count++] = td[j];
                }
            }
            free(td);
        }
    } else {
        /* Build all default targets */
        graph_collect_dirty_all(&ctx.graph, &dirty, &dirty_count);
    }

    /* Phase 7: Execute */
    int result = 0;
    if (dirty_count > 0) {
        Executor ex;
        exec_init(&ex, max_jobs, verbose);
        result = exec_run(&ex, dirty, dirty_count);
        exec_free(&ex);
    } else {
        fprintf(stderr, "Nothing to do.\n");
    }

    /* Run COMMAND targets (like "run", "debug") after building */
    if (result == 0 && target_count > 0) {
        for (i = 0; i < target_count; i++) {
            int j;
            for (j = 0; j < ctx.graph.target_count; j++) {
                BuildTarget *tgt = ctx.graph.targets[j];
                if (strcmp(tgt->name, targets[i]) == 0 && tgt->command_count > 0) {
                    result = run_target_commands(&ctx, targets[i]);
                    break;
                }
            }
        }
    }

    /* Cleanup */
    free(dirty);
    mtime_cache_free(&mc);
    graph_free(&ctx.graph);
    ast_free(ast);
    token_list_free(&tokens);
    scope_free(ctx.global_scope);
    free(ctx.amake_path);
    free(abs_source);
    free(abs_build);
    free(cmake_path);
    for (i = 0; i < define_count; i++) free(defines[i]);
    free(defines);
    for (i = 0; i < target_count; i++) free(targets[i]);
    free(targets);
    /* Note: functions reference AST nodes, don't double-free */
    for (i = 0; i < ctx.func_count; i++) {
        free(ctx.functions[i]->name);
        int j;
        for (j = 0; j < ctx.functions[i]->param_count; j++)
            free(ctx.functions[i]->params[j]);
        free(ctx.functions[i]->params);
        free(ctx.functions[i]);
    }
    free(ctx.functions);

    return result;
}
