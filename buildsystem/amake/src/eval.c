/*
 * eval.c — CMake command evaluator for amake
 *
 * Walks the AST and executes each command, building variables and the
 * dependency graph. This is the core of amake — implements all ~20
 * CMake commands used by anyOS.
 */
#include "amake.h"
#include <ctype.h>

/* ── Forward declarations ────────────────────────────────────────────── */

static void eval_node(AmakeCtx *ctx, AstNode *node);
static void eval_nodes(AmakeCtx *ctx, AstNode *list);

/* ── Helper: case-insensitive string compare ─────────────────────────── */

static int streqi(const char *a, const char *b) {
    return strcasecmp(a, b) == 0;
}

/* ── Helper: find keyword argument index ─────────────────────────────── */

/*
 * Find position of keyword in expanded args. Returns -1 if not found.
 */
static int find_kwarg(char **args, int argc, const char *kw) {
    int i;
    for (i = 0; i < argc; i++)
        if (streqi(args[i], kw)) return i;
    return -1;
}

/* ── Helper: join args into semicolon-separated CMake list ───────────── */

static char *join_list(char **args, int argc) {
    if (argc == 0) return amake_strdup("");
    size_t total = 0;
    int i;
    for (i = 0; i < argc; i++)
        total += strlen(args[i]) + 1;
    char *out = amake_malloc(total);
    size_t pos = 0;
    for (i = 0; i < argc; i++) {
        if (i > 0) out[pos++] = ';';
        size_t len = strlen(args[i]);
        memcpy(out + pos, args[i], len);
        pos += len;
    }
    out[pos] = '\0';
    return out;
}

/* ── Helper: build a command string from COMMAND args ────────────────── */

/*
 * Build a single shell command string by joining args with spaces.
 * Handles quoting for args that contain spaces.
 */
static char *build_command_string(char **args, int argc) {
    size_t total = 0;
    int i;
    for (i = 0; i < argc; i++)
        total += strlen(args[i]) + 3; /* space + potential quotes */
    char *cmd = amake_malloc(total + 1);
    size_t pos = 0;
    int first = 1;
    for (i = 0; i < argc; i++) {
        /* Skip empty args (CMake drops unquoted empty variable expansions) */
        if (args[i][0] == '\0') continue;
        if (!first) cmd[pos++] = ' ';
        first = 0;
        int needs_quote = (strchr(args[i], ' ') != NULL ||
                          strchr(args[i], '\t') != NULL);
        if (needs_quote) cmd[pos++] = '"';
        size_t len = strlen(args[i]);
        memcpy(cmd + pos, args[i], len);
        pos += len;
        if (needs_quote) cmd[pos++] = '"';
    }
    cmd[pos] = '\0';
    return cmd;
}

/* ── Condition evaluation (for if/elseif) ────────────────────────────── */

static int eval_condition(AmakeCtx *ctx, char **args, int argc);

/*
 * Evaluate a single term or a compound condition.
 * CMake condition syntax (subset):
 *   NOT <expr>
 *   <expr> AND <expr>
 *   <expr> OR <expr>
 *   <var>                      — true if defined and not empty/"0"/"OFF"/"FALSE"/"NO"
 *   <a> STREQUAL <b>
 *   <a> MATCHES <b>            — basic string match (not regex for now)
 *   EXISTS <path>
 *   IS_DIRECTORY <path>
 *   DEFINED <var>
 *   <a> STRLESS <b>
 *   <a> NOTLIKE <b>            — not standard cmake but handle if encountered
 */
static int eval_condition(AmakeCtx *ctx, char **raw_args, int argc) {
    if (argc == 0) return 0;

    /* Expand variables in condition args */
    char **args;
    int exp_argc;
    expand_args(ctx, raw_args, argc, &args, &exp_argc);
    argc = exp_argc;

    int result = 0;
    int i;

    /* Handle NOT prefix */
    if (argc >= 2 && streqi(args[0], "NOT")) {
        /* Recurse on remaining args */
        result = !eval_condition(ctx, args + 1, argc - 1);
        goto done;
    }

    /* Handle AND / OR (find first occurrence, split) */
    for (i = 1; i < argc - 1; i++) {
        if (streqi(args[i], "AND")) {
            int left = eval_condition(ctx, args, i);
            int right = eval_condition(ctx, args + i + 1, argc - i - 1);
            result = left && right;
            goto done;
        }
    }
    for (i = 1; i < argc - 1; i++) {
        if (streqi(args[i], "OR")) {
            int left = eval_condition(ctx, args, i);
            int right = eval_condition(ctx, args + i + 1, argc - i - 1);
            result = left || right;
            goto done;
        }
    }

    /* Binary operators */
    if (argc == 3) {
        if (streqi(args[1], "STREQUAL")) {
            result = (strcmp(args[0], args[2]) == 0);
            goto done;
        }
        if (streqi(args[1], "STRLESS")) {
            result = (strcmp(args[0], args[2]) < 0);
            goto done;
        }
        if (streqi(args[1], "STRGREATER")) {
            result = (strcmp(args[0], args[2]) > 0);
            goto done;
        }
        if (streqi(args[1], "EQUAL")) {
            result = (atoi(args[0]) == atoi(args[2]));
            goto done;
        }
        if (streqi(args[1], "LESS")) {
            result = (atoi(args[0]) < atoi(args[2]));
            goto done;
        }
        if (streqi(args[1], "GREATER")) {
            result = (atoi(args[0]) > atoi(args[2]));
            goto done;
        }
        if (streqi(args[1], "MATCHES")) {
            /* Simple substring match (not full regex) */
            result = (strstr(args[0], args[2]) != NULL);
            goto done;
        }
    }

    /* Unary operators */
    if (argc == 2) {
        if (streqi(args[0], "EXISTS")) {
            result = amake_file_exists(args[1]) || amake_is_directory(args[1]);
            goto done;
        }
        if (streqi(args[0], "IS_DIRECTORY")) {
            result = amake_is_directory(args[1]);
            goto done;
        }
        if (streqi(args[0], "DEFINED")) {
            result = (scope_get(ctx->current_scope, args[1]) != NULL);
            goto done;
        }
    }

    /* Single argument: truthiness test */
    if (argc == 1) {
        const char *v = args[0];
        /* False values: empty, 0, OFF, NO, FALSE, NOTFOUND, *-NOTFOUND */
        if (!v[0] || strcmp(v, "0") == 0 ||
            streqi(v, "OFF") || streqi(v, "NO") ||
            streqi(v, "FALSE") || streqi(v, "NOTFOUND") ||
            streqi(v, "IGNORE") || streqi(v, "N")) {
            result = 0;
        } else {
            /* Check if it's an undefined variable name */
            const char *val = scope_get(ctx->current_scope, v);
            if (val) {
                /* Variable exists — check its value */
                result = (val[0] != '\0' && strcmp(val, "0") != 0 &&
                          !streqi(val, "OFF") && !streqi(val, "NO") &&
                          !streqi(val, "FALSE") && !streqi(val, "NOTFOUND"));
            } else {
                /* Not a known variable — treat the literal as truthy */
                result = 1;
            }
        }
        goto done;
    }

    /* Fallback: non-empty = true */
    result = (argc > 0);

done:
    for (i = 0; i < argc; i++) free(args[i]);
    free(args);
    return result;
}

/* ── Command: set() ──────────────────────────────────────────────────── */

static void cmd_set(AmakeCtx *ctx, char **args, int argc) {
    if (argc < 1) return;
    char *name = args[0];
    int parent_scope = 0;

    /* Check for PARENT_SCOPE */
    if (argc >= 2 && streqi(args[argc - 1], "PARENT_SCOPE")) {
        parent_scope = 1;
        argc--;
    }

    char *value;
    if (argc == 1) {
        /* set(VAR) — unset */
        value = amake_strdup("");
    } else {
        /* set(VAR val1 val2...) — join with semicolons */
        value = join_list(args + 1, argc - 1);
    }

    if (parent_scope && ctx->current_scope->parent) {
        scope_set(ctx->current_scope->parent, name, value);
    } else {
        scope_set(ctx->current_scope, name, value);
    }
    free(value);
}

/* ── Command: option() ───────────────────────────────────────────────── */

static void cmd_option(AmakeCtx *ctx, char **args, int argc) {
    if (argc < 1) return;
    const char *name = args[0];
    /* args[1] is the description (ignored) */
    const char *default_val = (argc >= 3) ? args[2] : "OFF";

    /* Check if already set (e.g., via -D on command line) */
    if (scope_get(ctx->current_scope, name))
        return;

    scope_set(ctx->current_scope, name, default_val);
}

/* ── Command: message() ──────────────────────────────────────────────── */

static void cmd_message(AmakeCtx *ctx, char **args, int argc) {
    (void)ctx;
    if (argc < 1) return;

    const char *type = args[0];
    int is_fatal = streqi(type, "FATAL_ERROR");
    int is_status = streqi(type, "STATUS") || streqi(type, "WARNING");
    int start = is_fatal || is_status ? 1 : 0;

    /* Concatenate message args */
    int i;
    for (i = start; i < argc; i++) {
        if (i > start) fprintf(stderr, " ");
        fprintf(stderr, "%s", args[i]);
    }
    fprintf(stderr, "\n");

    if (is_fatal) exit(1);
}

/* ── Command: find_program() ─────────────────────────────────────────── */

static int is_executable(const char *path) {
    struct stat st;
    if (stat(path, &st) != 0) return 0;
    return (st.st_mode & S_IXUSR) || (st.st_mode & S_IXGRP) || (st.st_mode & S_IXOTH);
}

static char *find_in_path(const char *name) {
    const char *path_env = getenv("PATH");
    if (!path_env) return NULL;

    char *path_copy = amake_strdup(path_env);
    char *saveptr = NULL;
    char *dir = strtok_r(path_copy, ":", &saveptr);

    while (dir) {
        char *full = amake_path_join(dir, name);
        if (is_executable(full)) {
            free(path_copy);
            return full;
        }
        free(full);
        dir = strtok_r(NULL, ":", &saveptr);
    }
    free(path_copy);
    return NULL;
}

static void cmd_find_program(AmakeCtx *ctx, char **args, int argc) {
    if (argc < 2) return;

    const char *var = args[0];

    /* Collect NAMES and HINTS */
    int names_start = -1, hints_start = -1;
    int i;

    /* Check if second arg is NAMES */
    if (argc >= 3 && streqi(args[1], "NAMES")) {
        names_start = 2;
    } else {
        /* Second arg is the name directly */
        names_start = 1;
    }

    hints_start = find_kwarg(args, argc, "HINTS");
    int names_end = hints_start >= 0 ? hints_start : argc;

    /* Search hints first */
    if (hints_start >= 0) {
        int j;
        for (j = hints_start + 1; j < argc; j++) {
            /* Each hint is a directory */
            for (i = names_start; i < names_end; i++) {
                char *full = amake_path_join(args[j], args[i]);
                if (is_executable(full)) {
                    scope_set(ctx->current_scope, var, full);
                    free(full);
                    return;
                }
                free(full);
            }
        }
    }

    /* Search PATH */
    for (i = names_start; i < names_end; i++) {
        char *found = find_in_path(args[i]);
        if (found) {
            scope_set(ctx->current_scope, var, found);
            free(found);
            return;
        }
    }

    /* Not found — set to *-NOTFOUND */
    char *nf = amake_sprintf("%s-NOTFOUND", var);
    scope_set(ctx->current_scope, var, nf);
    free(nf);
}

/* ── Command: file() ─────────────────────────────────────────────────── */

static void cmd_file(AmakeCtx *ctx, char **args, int argc) {
    if (argc < 1) return;

    if (streqi(args[0], "GLOB") || streqi(args[0], "GLOB_RECURSE")) {
        int is_recurse = streqi(args[0], "GLOB_RECURSE");
        if (argc < 3) return;

        const char *var = args[1];
        int pattern_start = 2;

        /* Skip CONFIGURE_DEPENDS if present */
        if (pattern_start < argc && streqi(args[pattern_start], "CONFIGURE_DEPENDS"))
            pattern_start++;

        /* Glob each pattern and collect results */
        char **all_files = NULL;
        int total = 0;
        int i;

        for (i = pattern_start; i < argc; i++) {
            char **files;
            int count;
            if (is_recurse)
                amake_glob_recurse(ctx->source_dir, args[i], &files, &count);
            else
                amake_glob(args[i], &files, &count);

            /* Append to total list */
            int j;
            for (j = 0; j < count; j++) {
                int new_total = total + 1;
                all_files = amake_realloc(all_files, sizeof(char *) * new_total);
                all_files[total] = files[j];
                total = new_total;
            }
            free(files);
        }

        /* Set variable to semicolon-separated list */
        char *value = join_list(all_files, total);
        scope_set(ctx->current_scope, var, value);
        free(value);
        for (i = 0; i < total; i++) free(all_files[i]);
        free(all_files);
    }
    else if (streqi(args[0], "MAKE_DIRECTORY")) {
        int i;
        for (i = 1; i < argc; i++)
            amake_mkdir_p(args[i]);
    }
}

/* ── Command: add_custom_command() ───────────────────────────────────── */

static void cmd_add_custom_command(AmakeCtx *ctx, char **args, int argc) {
    BuildRule *rule = graph_add_rule(&ctx->graph);

    /* Parse keyword arguments: OUTPUT, COMMAND, DEPENDS, COMMENT, WORKING_DIRECTORY */
    int i = 0;
    while (i < argc) {
        if (streqi(args[i], "OUTPUT")) {
            i++;
            while (i < argc && !streqi(args[i], "COMMAND") &&
                   !streqi(args[i], "DEPENDS") && !streqi(args[i], "COMMENT") &&
                   !streqi(args[i], "WORKING_DIRECTORY")) {
                if (rule->output_count < MAX_OUTPUTS) {
                    rule->outputs = amake_realloc(rule->outputs,
                        sizeof(char *) * (rule->output_count + 1));
                    rule->outputs[rule->output_count++] = amake_strdup(args[i]);
                }
                i++;
            }
        }
        else if (streqi(args[i], "COMMAND")) {
            i++;
            /* Collect args until next keyword */
            int cmd_start = i;
            while (i < argc && !streqi(args[i], "COMMAND") &&
                   !streqi(args[i], "OUTPUT") && !streqi(args[i], "DEPENDS") &&
                   !streqi(args[i], "COMMENT") && !streqi(args[i], "WORKING_DIRECTORY")) {
                i++;
            }
            if (i > cmd_start && rule->command_count < MAX_COMMANDS) {
                char *cmd = build_command_string(args + cmd_start, i - cmd_start);
                rule->commands = amake_realloc(rule->commands,
                    sizeof(char *) * (rule->command_count + 1));
                rule->commands[rule->command_count++] = cmd;
            }
        }
        else if (streqi(args[i], "DEPENDS")) {
            i++;
            while (i < argc && !streqi(args[i], "COMMAND") &&
                   !streqi(args[i], "OUTPUT") && !streqi(args[i], "COMMENT") &&
                   !streqi(args[i], "WORKING_DIRECTORY")) {
                if (rule->depend_count < MAX_DEPENDS) {
                    rule->depends = amake_realloc(rule->depends,
                        sizeof(char *) * (rule->depend_count + 1));
                    rule->depends[rule->depend_count++] = amake_strdup(args[i]);
                }
                i++;
            }
        }
        else if (streqi(args[i], "COMMENT")) {
            i++;
            if (i < argc) {
                free(rule->comment);
                rule->comment = amake_strdup(args[i]);
                i++;
            }
        }
        else if (streqi(args[i], "WORKING_DIRECTORY")) {
            i++;
            if (i < argc) {
                free(rule->working_dir);
                rule->working_dir = amake_strdup(args[i]);
                i++;
            }
        }
        else {
            i++; /* skip unknown keyword */
        }
    }
}

/* ── Command: add_custom_target() ────────────────────────────────────── */

static void cmd_add_custom_target(AmakeCtx *ctx, char **args, int argc) {
    if (argc < 1) return;

    BuildTarget *tgt = graph_add_target(&ctx->graph);
    tgt->name = amake_strdup(args[0]);

    int i = 1;

    /* Check for ALL */
    if (i < argc && streqi(args[i], "ALL")) {
        tgt->is_default = 1;
        i++;
    }

    while (i < argc) {
        if (streqi(args[i], "DEPENDS")) {
            i++;
            while (i < argc && !streqi(args[i], "COMMAND") &&
                   !streqi(args[i], "COMMENT") && !streqi(args[i], "USES_TERMINAL")) {
                tgt->depends = amake_realloc(tgt->depends,
                    sizeof(char *) * (tgt->depend_count + 1));
                tgt->depends[tgt->depend_count++] = amake_strdup(args[i]);
                i++;
            }
        }
        else if (streqi(args[i], "COMMAND")) {
            i++;
            int cmd_start = i;
            while (i < argc && !streqi(args[i], "COMMAND") &&
                   !streqi(args[i], "DEPENDS") && !streqi(args[i], "COMMENT") &&
                   !streqi(args[i], "USES_TERMINAL")) {
                i++;
            }
            if (i > cmd_start) {
                char *cmd = build_command_string(args + cmd_start, i - cmd_start);
                tgt->commands = amake_realloc(tgt->commands,
                    sizeof(char *) * (tgt->command_count + 1));
                tgt->commands[tgt->command_count++] = cmd;
            }
        }
        else if (streqi(args[i], "COMMENT")) {
            i++;
            if (i < argc) {
                free(tgt->comment);
                tgt->comment = amake_strdup(args[i]);
                i++;
            }
        }
        else if (streqi(args[i], "USES_TERMINAL")) {
            tgt->uses_terminal = 1;
            i++;
        }
        else {
            i++;
        }
    }
}

/* ── Command: get_filename_component() ───────────────────────────────── */

static void cmd_get_filename_component(AmakeCtx *ctx, char **args, int argc) {
    if (argc < 3) return;

    const char *var = args[0];
    const char *path = args[1];
    const char *mode = args[2];

    if (streqi(mode, "NAME_WE") || streqi(mode, "NAME_WLE")) {
        /* Strip directory and last extension */
        const char *base = strrchr(path, '/');
        base = base ? base + 1 : path;
        const char *dot = strrchr(base, '.');
        if (dot && dot > base) {
            char *result = amake_strndup(base, (size_t)(dot - base));
            scope_set(ctx->current_scope, var, result);
            free(result);
        } else {
            scope_set(ctx->current_scope, var, base);
        }
    }
    else if (streqi(mode, "DIRECTORY") || streqi(mode, "PATH")) {
        const char *sep = strrchr(path, '/');
        if (sep) {
            char *dir = amake_strndup(path, (size_t)(sep - path));
            scope_set(ctx->current_scope, var, dir);
            free(dir);
        } else {
            scope_set(ctx->current_scope, var, ".");
        }
    }
    else if (streqi(mode, "NAME")) {
        const char *base = strrchr(path, '/');
        scope_set(ctx->current_scope, var, base ? base + 1 : path);
    }
    else if (streqi(mode, "EXT") || streqi(mode, "LAST_EXT")) {
        const char *base = strrchr(path, '/');
        base = base ? base + 1 : path;
        const char *dot = strrchr(base, '.');
        scope_set(ctx->current_scope, var, dot ? dot : "");
    }
}

/* ── Command: list() ─────────────────────────────────────────────────── */

static void cmd_list(AmakeCtx *ctx, char **args, int argc) {
    if (argc < 2) return;

    if (streqi(args[0], "APPEND")) {
        const char *var = args[1];
        const char *existing = scope_get(ctx->current_scope, var);
        char *current = existing ? amake_strdup(existing) : amake_strdup("");

        int i;
        for (i = 2; i < argc; i++) {
            size_t clen = strlen(current);
            size_t alen = strlen(args[i]);
            char *new_val = amake_malloc(clen + 1 + alen + 1);
            if (clen > 0) {
                memcpy(new_val, current, clen);
                new_val[clen] = ';';
                memcpy(new_val + clen + 1, args[i], alen + 1);
            } else {
                memcpy(new_val, args[i], alen + 1);
            }
            free(current);
            current = new_val;
        }
        scope_set(ctx->current_scope, var, current);
        free(current);
    }
    else if (streqi(args[0], "LENGTH")) {
        if (argc < 3) return;
        const char *list_val = scope_get(ctx->current_scope, args[1]);
        int count = 0;
        if (list_val && list_val[0]) {
            count = 1;
            const char *p = list_val;
            while (*p) { if (*p == ';') count++; p++; }
        }
        char buf[32];
        snprintf(buf, sizeof(buf), "%d", count);
        scope_set(ctx->current_scope, args[2], buf);
    }
}

/* ── Command: string() ───────────────────────────────────────────────── */

static void cmd_string(AmakeCtx *ctx, char **args, int argc) {
    if (argc < 1) return;

    if (streqi(args[0], "REPLACE") && argc >= 5) {
        /* string(REPLACE old new VAR input...) */
        const char *old_str = args[1];
        const char *new_str = args[2];
        const char *var = args[3];

        /* Join remaining args as input */
        char *input = join_list(args + 4, argc - 4);

        /* Simple string replace */
        size_t old_len = strlen(old_str);
        size_t new_len = strlen(new_str);
        if (old_len == 0) {
            scope_set(ctx->current_scope, var, input);
            free(input);
            return;
        }

        size_t cap = strlen(input) * 2 + 64;
        char *result = amake_malloc(cap);
        size_t rlen = 0;
        const char *p = input;

        while (*p) {
            if (strncmp(p, old_str, old_len) == 0) {
                while (rlen + new_len + 1 > cap) { cap *= 2; result = amake_realloc(result, cap); }
                memcpy(result + rlen, new_str, new_len);
                rlen += new_len;
                p += old_len;
            } else {
                if (rlen + 1 >= cap) { cap *= 2; result = amake_realloc(result, cap); }
                result[rlen++] = *p++;
            }
        }
        result[rlen] = '\0';
        scope_set(ctx->current_scope, var, result);
        free(result);
        free(input);
    }
}

/* ── Command: cmake_minimum_required() ───────────────────────────────── */

static void cmd_cmake_minimum_required(AmakeCtx *ctx, char **args, int argc) {
    (void)ctx; (void)args; (void)argc;
    /* Ignored — amake doesn't enforce CMake version */
}

/* ── Command: project() ──────────────────────────────────────────────── */

static void cmd_project(AmakeCtx *ctx, char **args, int argc) {
    if (argc >= 1) {
        scope_set(ctx->current_scope, "PROJECT_NAME", args[0]);
        scope_set(ctx->current_scope, "CMAKE_PROJECT_NAME", args[0]);
    }
}

/* ── Command: set_property() ─────────────────────────────────────────── */

static void cmd_set_property(AmakeCtx *ctx, char **args, int argc) {
    (void)ctx; (void)args; (void)argc;
    /* Informational only (ADDITIONAL_CLEAN_FILES) — skip */
}

/* ── User-defined function call ──────────────────────────────────────── */

static FuncDef *find_function(AmakeCtx *ctx, const char *name) {
    int i;
    for (i = 0; i < ctx->func_count; i++)
        if (streqi(ctx->functions[i]->name, name))
            return ctx->functions[i];
    return NULL;
}

static void call_function(AmakeCtx *ctx, FuncDef *func, char **args, int argc) {
    /* Push new scope */
    VarScope *func_scope = scope_new(ctx->current_scope);
    VarScope *prev_scope = ctx->current_scope;
    ctx->current_scope = func_scope;

    /* Set parameters */
    int i;
    for (i = 0; i < func->param_count && i < argc; i++)
        scope_set(func_scope, func->params[i], args[i]);

    /* Set ARGC and ARGV */
    char argc_str[16];
    snprintf(argc_str, sizeof(argc_str), "%d", argc);
    scope_set(func_scope, "ARGC", argc_str);
    char *argv_val = join_list(args, argc);
    scope_set(func_scope, "ARGV", argv_val);
    free(argv_val);

    /* Set ARGN (extra args beyond defined params) */
    if (argc > func->param_count) {
        char *argn = join_list(args + func->param_count, argc - func->param_count);
        scope_set(func_scope, "ARGN", argn);
        free(argn);
    } else {
        scope_set(func_scope, "ARGN", "");
    }

    /* Set ARGV0, ARGV1, etc. */
    for (i = 0; i < argc; i++) {
        char argn_name[16];
        snprintf(argn_name, sizeof(argn_name), "ARGV%d", i);
        scope_set(func_scope, argn_name, args[i]);
    }

    /* Execute body */
    eval_nodes(ctx, func->body);

    /* Pop scope */
    ctx->current_scope = prev_scope;
    scope_free(func_scope);
}

/* ── Node evaluation ─────────────────────────────────────────────────── */

static void eval_node(AmakeCtx *ctx, AstNode *node) {
    if (!node) return;

    switch (node->type) {
    case AST_COMMAND: {
        /* Expand variables in all arguments */
        char **exp_args;
        int exp_argc;
        expand_args(ctx, node->args, node->argc, &exp_args, &exp_argc);

        /* Dispatch to handler */
        const char *cmd = node->cmd_name;

        if (streqi(cmd, "set"))
            cmd_set(ctx, exp_args, exp_argc);
        else if (streqi(cmd, "option"))
            cmd_option(ctx, exp_args, exp_argc);
        else if (streqi(cmd, "message"))
            cmd_message(ctx, exp_args, exp_argc);
        else if (streqi(cmd, "find_program"))
            cmd_find_program(ctx, exp_args, exp_argc);
        else if (streqi(cmd, "file"))
            cmd_file(ctx, exp_args, exp_argc);
        else if (streqi(cmd, "add_custom_command"))
            cmd_add_custom_command(ctx, exp_args, exp_argc);
        else if (streqi(cmd, "add_custom_target"))
            cmd_add_custom_target(ctx, exp_args, exp_argc);
        else if (streqi(cmd, "get_filename_component"))
            cmd_get_filename_component(ctx, exp_args, exp_argc);
        else if (streqi(cmd, "list"))
            cmd_list(ctx, exp_args, exp_argc);
        else if (streqi(cmd, "string"))
            cmd_string(ctx, exp_args, exp_argc);
        else if (streqi(cmd, "cmake_minimum_required"))
            cmd_cmake_minimum_required(ctx, exp_args, exp_argc);
        else if (streqi(cmd, "project"))
            cmd_project(ctx, exp_args, exp_argc);
        else if (streqi(cmd, "set_property"))
            cmd_set_property(ctx, exp_args, exp_argc);
        else {
            /* Check user-defined functions */
            FuncDef *func = find_function(ctx, cmd);
            if (func) {
                call_function(ctx, func, exp_args, exp_argc);
            }
            /* else: unknown command — silently ignore */
        }

        int i;
        for (i = 0; i < exp_argc; i++) free(exp_args[i]);
        free(exp_args);
        break;
    }

    case AST_IF_BLOCK: {
        /* Evaluate condition with raw (unexpanded) args — eval_condition expands them */
        int cond = (node->cond_argc == 0) ? 1 : /* else clause: always true */
                   eval_condition(ctx, node->cond_args, node->cond_argc);
        if (cond) {
            eval_nodes(ctx, node->if_body);
        } else if (node->else_chain) {
            eval_node(ctx, node->else_chain);
        }
        break;
    }

    case AST_FOREACH: {
        /* Expand loop values */
        char **exp_vals;
        int exp_count;
        expand_args(ctx, node->loop_values, node->loop_value_count,
                    &exp_vals, &exp_count);

        int i;
        for (i = 0; i < exp_count; i++) {
            scope_set(ctx->current_scope, node->loop_var, exp_vals[i]);
            eval_nodes(ctx, node->loop_body);
        }

        for (i = 0; i < exp_count; i++) free(exp_vals[i]);
        free(exp_vals);
        break;
    }

    case AST_FUNCTION_DEF: {
        /* Store function definition for later invocation */
        if (ctx->func_count >= ctx->func_cap) {
            ctx->func_cap = ctx->func_cap ? ctx->func_cap * 2 : 16;
            ctx->functions = amake_realloc(ctx->functions,
                sizeof(FuncDef *) * ctx->func_cap);
        }
        FuncDef *fd = amake_malloc(sizeof(FuncDef));
        fd->name = amake_strdup(node->func_name);
        fd->param_count = node->func_param_count;
        fd->params = amake_malloc(sizeof(char *) * (fd->param_count + 1));
        int i;
        for (i = 0; i < fd->param_count; i++)
            fd->params[i] = amake_strdup(node->func_params[i]);
        fd->body = node->func_body;   /* reference, not copy */
        fd->body_count = node->func_body_count;
        ctx->functions[ctx->func_count++] = fd;
        break;
    }
    }
}

static void eval_nodes(AmakeCtx *ctx, AstNode *list) {
    AstNode *n = list;
    while (n) {
        eval_node(ctx, n);
        n = n->next;
    }
}

/* ── Public API ──────────────────────────────────────────────────────── */

void eval_run(AmakeCtx *ctx, AstNode *nodes) {
    /* Set built-in variables */
    scope_set(ctx->global_scope, "CMAKE_SOURCE_DIR", ctx->source_dir);
    scope_set(ctx->global_scope, "CMAKE_BINARY_DIR", ctx->binary_dir);
    scope_set(ctx->global_scope, "CMAKE_COMMAND", ctx->amake_path);
    scope_set(ctx->global_scope, "CMAKE_CURRENT_SOURCE_DIR", ctx->source_dir);
    scope_set(ctx->global_scope, "CMAKE_CURRENT_BINARY_DIR", ctx->binary_dir);
    scope_set(ctx->global_scope, "CMAKE_EXECUTABLE_SUFFIX", "");

    /* Apply CLI -D overrides */
    int i;
    for (i = 0; i < ctx->cli_define_count; i++) {
        char *def = amake_strdup(ctx->cli_defines[i]);
        char *eq = strchr(def, '=');
        if (eq) {
            *eq = '\0';
            scope_set(ctx->global_scope, def, eq + 1);
        }
        free(def);
    }

    ctx->current_scope = ctx->global_scope;
    eval_nodes(ctx, nodes);
}
