/*
 * exec.c — Parallel build executor for amake
 *
 * Runs dirty build rules in parallel using fork/waitpid on Unix.
 * Respects dependency ordering: a rule only starts when all its
 * blockers have completed.
 */
#include "amake.h"

#ifndef _WIN32
#include <signal.h>
#include <errno.h>
#endif

/* ── Executor initialization ─────────────────────────────────────────── */

void exec_init(Executor *ex, int max_jobs, int verbose, const char *amake_path) {
    memset(ex, 0, sizeof(Executor));
    ex->max_jobs = max_jobs > 0 ? max_jobs : 4;
    ex->verbose = verbose;
    ex->amake_path = amake_path;
    ex->job_cap = max_jobs + 4;
    ex->jobs = amake_malloc(sizeof(RunningJob) * ex->job_cap);
    ex->ready_cap = 64;
    ex->ready = amake_malloc(sizeof(BuildRule *) * ex->ready_cap);
}

/* ── Ready queue management ──────────────────────────────────────────── */

static void ready_push(Executor *ex, BuildRule *rule) {
    if (ex->ready_count >= ex->ready_cap) {
        ex->ready_cap *= 2;
        ex->ready = amake_realloc(ex->ready, sizeof(BuildRule *) * ex->ready_cap);
    }
    ex->ready[ex->ready_count++] = rule;
}

static BuildRule *ready_pop(Executor *ex) {
    if (ex->ready_count == 0) return NULL;
    BuildRule *r = ex->ready[--ex->ready_count];
    return r;
}

/* ── In-process -E builtin handling ──────────────────────────────────── */

/*
 * Simple tokenizer: split a shell command string into argv-style tokens.
 * Handles double-quoted strings. Returns token count.
 */
static int split_shell_args(const char *cmd, char ***out_argv) {
    int cap = 16, argc = 0;
    char **argv = amake_malloc(sizeof(char *) * cap);
    const char *p = cmd;

    while (*p) {
        while (*p == ' ' || *p == '\t') p++;
        if (!*p) break;

        char buf[MAX_PATH_LEN];
        int len = 0;

        if (*p == '"') {
            p++;
            while (*p && *p != '"') {
                if (*p == '\\' && *(p+1)) { p++; }
                if (len < MAX_PATH_LEN - 1) buf[len++] = *p;
                p++;
            }
            if (*p == '"') p++;
        } else {
            while (*p && *p != ' ' && *p != '\t') {
                if (len < MAX_PATH_LEN - 1) buf[len++] = *p;
                p++;
            }
        }
        buf[len] = '\0';

        if (argc >= cap) {
            cap *= 2;
            argv = amake_realloc(argv, sizeof(char *) * cap);
        }
        argv[argc++] = amake_strdup(buf);
    }

    *out_argv = argv;
    return argc;
}

static void free_argv(char **argv, int argc) {
    int i;
    for (i = 0; i < argc; i++) free(argv[i]);
    free(argv);
}

/*
 * Try to handle a command in-process if it's an amake -E builtin.
 * Returns: -1 if not a builtin (use fork), 0 = success, >0 = error code.
 * For -E env, transforms into shell-native syntax and returns -2 with
 * *out_rewritten set to the rewritten command string.
 */
static int try_run_builtin(const char *cmd, const char *amake_path,
                           char **out_rewritten)
{
    *out_rewritten = NULL;

    if (!amake_path || !amake_path[0]) return -1;

    /* Check if command starts with the amake path */
    size_t path_len = strlen(amake_path);
    if (strncmp(cmd, amake_path, path_len) != 0) return -1;
    if (cmd[path_len] != ' ' && cmd[path_len] != '\0') return -1;

    /* Parse the full command */
    char **argv = NULL;
    int argc = split_shell_args(cmd, &argv);

    if (argc < 2 || strcmp(argv[1], "-E") != 0) {
        free_argv(argv, argc);
        return -1;
    }

    if (argc < 3) {
        free_argv(argv, argc);
        return 1; /* -E with no subcommand */
    }

    const char *subcmd = argv[2];

    /* Handle env: transform to shell-native VAR=VAL cmd args... */
    if (strcmp(subcmd, "env") == 0) {
        /* Build rewritten command: VAR=VAL VAR2=VAL2 command args...
         * Re-quote tokens that contain spaces or special chars. */
        size_t total = 0;
        int i;
        for (i = 3; i < argc; i++)
            total += strlen(argv[i]) + 4; /* space + potential quotes */
        char *rewritten = amake_malloc(total + 1);
        size_t pos = 0;
        for (i = 3; i < argc; i++) {
            if (i > 3) rewritten[pos++] = ' ';
            int needs_quote = (strchr(argv[i], ' ') != NULL ||
                              strchr(argv[i], '\t') != NULL);
            if (needs_quote) rewritten[pos++] = '"';
            size_t len = strlen(argv[i]);
            memcpy(rewritten + pos, argv[i], len);
            pos += len;
            if (needs_quote) rewritten[pos++] = '"';
        }
        rewritten[pos] = '\0';
        *out_rewritten = rewritten;
        free_argv(argv, argc);
        return -2; /* signal: use rewritten command with fork */
    }

    /* Handle make_directory */
    if (strcmp(subcmd, "make_directory") == 0) {
        int i;
        for (i = 3; i < argc; i++)
            amake_mkdir_p(argv[i]);
        free_argv(argv, argc);
        return 0;
    }

    /* Handle copy */
    if (strcmp(subcmd, "copy") == 0) {
        if (argc < 5) { free_argv(argv, argc); return 1; }
        /* Ensure destination directory exists */
        char *dst_dir = amake_strdup(argv[4]);
        char *sep = strrchr(dst_dir, '/');
        if (sep) { *sep = '\0'; amake_mkdir_p(dst_dir); }
        free(dst_dir);
        int rc = amake_copy_file(argv[3], argv[4]);
        free_argv(argv, argc);
        return rc == 0 ? 0 : 1;
    }

    /* Handle copy_directory */
    if (strcmp(subcmd, "copy_directory") == 0) {
        if (argc < 5) { free_argv(argv, argc); return 1; }
        int rc = amake_copy_directory(argv[3], argv[4]);
        free_argv(argv, argc);
        return rc == 0 ? 0 : 1;
    }

    /* Handle rm */
    if (strcmp(subcmd, "rm") == 0) {
        int i, start = 3;
        while (start < argc && argv[start][0] == '-') start++;
        for (i = start; i < argc; i++)
            amake_rm_rf(argv[i]);
        free_argv(argv, argc);
        return 0;
    }

    /* Handle touch */
    if (strcmp(subcmd, "touch") == 0) {
        int i;
        for (i = 3; i < argc; i++)
            amake_touch(argv[i]);
        free_argv(argv, argc);
        return 0;
    }

    /* Unknown -E subcommand */
    free_argv(argv, argc);
    return -1;
}

/* ── Run a single command ────────────────────────────────────────────── */

#ifndef _WIN32

/*
 * Fork and exec a shell command.
 * Returns the child PID, or -1 on error.
 */
static pid_t spawn_command(const char *cmd, const char *working_dir) {
    pid_t pid = fork();
    if (pid < 0) return -1;

    if (pid == 0) {
        /* Child */
        if (working_dir && working_dir[0]) {
            if (chdir(working_dir) != 0) {
                fprintf(stderr, "amake: chdir(%s) failed\n", working_dir);
                _exit(1);
            }
        }
        execl("/bin/sh", "sh", "-c", cmd, (char *)NULL);
        _exit(127);
    }

    return pid;
}

#endif

/* ── Complete a rule ─────────────────────────────────────────────────── */

static void rule_completed(Executor *ex, BuildRule *rule) {
    rule->done = 1;
    ex->built_count++;

    /* Unblock dependents */
    int i;
    for (i = 0; i < rule->blocked_count; i++) {
        BuildRule *dep = rule->blocked[i];
        dep->unresolved--;
        if (dep->unresolved <= 0 && dep->dirty && !dep->done && !dep->building) {
            ready_push(ex, dep);
        }
    }
}

static void rule_failed(Executor *ex, BuildRule *rule) {
    rule->failed = 1;
    rule->done = 1;
    ex->failed_count++;
}

/* ── Main execution loop ─────────────────────────────────────────────── */

int exec_run(Executor *ex, BuildRule **dirty, int dirty_count) {
    int i;

    if (dirty_count == 0) {
        fprintf(stderr, "Nothing to do.\n");
        return 0;
    }

    ex->total_dirty = dirty_count;

    /* Seed ready queue with rules that have no unresolved blockers */
    for (i = 0; i < dirty_count; i++) {
        BuildRule *r = dirty[i];
        /* Recalculate unresolved count considering only dirty blockers */
        r->unresolved = 0;
        int j;
        for (j = 0; j < r->blocker_count; j++) {
            if (r->blockers[j]->dirty && !r->blockers[j]->done)
                r->unresolved++;
        }
        if (r->unresolved == 0)
            ready_push(ex, r);
    }

#ifndef _WIN32
    /* Main loop */
    while (ex->ready_count > 0 || ex->job_count > 0) {
        /* Launch jobs while we have capacity and ready rules */
        while (ex->ready_count > 0 && ex->job_count < ex->max_jobs) {
            BuildRule *rule = ready_pop(ex);
            if (!rule) break;

            if (rule->command_count == 0) {
                /* No commands — just mark done (phony-like target) */
                rule_completed(ex, rule);
                continue;
            }

            /* Print comment or first output */
            if (rule->comment) {
                fprintf(stderr, "[%d/%d] %s\n",
                        ex->built_count + ex->job_count + 1,
                        ex->total_dirty, rule->comment);
            } else if (rule->output_count > 0) {
                fprintf(stderr, "[%d/%d] Building %s\n",
                        ex->built_count + ex->job_count + 1,
                        ex->total_dirty, rule->outputs[0]);
            }

            if (ex->verbose && rule->command_count > 0) {
                fprintf(stderr, "  > %s\n", rule->commands[0]);
            }

            rule->building = 1;

            /* Try to run commands in-process (builtins) before forking */
            int cmd_start = 0;
            while (cmd_start < rule->command_count) {
                char *rewritten = NULL;
                int brc = try_run_builtin(rule->commands[cmd_start],
                                          ex->amake_path, &rewritten);
                if (brc == 0) {
                    /* Builtin succeeded in-process, advance to next command */
                    cmd_start++;
                    continue;
                } else if (brc > 0) {
                    /* Builtin failed */
                    fprintf(stderr, "amake: FAILED (builtin): %s\n",
                            rule->commands[cmd_start]);
                    rule_failed(ex, rule);
                    break;
                } else if (brc == -2 && rewritten) {
                    /* env: use rewritten command with fork */
                    free(rule->commands[cmd_start]);
                    rule->commands[cmd_start] = rewritten;
                    break; /* fall through to fork path */
                } else {
                    break; /* not a builtin, use fork */
                }
            }

            if (rule->failed) continue;
            if (cmd_start >= rule->command_count) {
                /* All commands were builtins and completed in-process */
                rule_completed(ex, rule);
                continue;
            }

            /* Spawn the command that needs forking */
            if (ex->verbose && cmd_start > 0) {
                fprintf(stderr, "  > %s\n", rule->commands[cmd_start]);
            }

            pid_t pid = spawn_command(rule->commands[cmd_start], rule->working_dir);
            if (pid < 0) {
                fprintf(stderr, "amake: fork failed for: %s\n", rule->commands[cmd_start]);
                rule_failed(ex, rule);
                continue;
            }

            ex->jobs[ex->job_count].pid = pid;
            ex->jobs[ex->job_count].rule = rule;
            ex->jobs[ex->job_count].cmd_index = cmd_start;
            ex->job_count++;
        }

        if (ex->job_count == 0) break;

        /* Wait for any child to complete */
        int status;
        pid_t pid = waitpid(-1, &status, 0);
        if (pid <= 0) {
            if (errno == EINTR) continue;
            break;
        }

        /* Find the matching job */
        int found = -1;
        for (i = 0; i < ex->job_count; i++) {
            if (ex->jobs[i].pid == pid) {
                found = i;
                break;
            }
        }
        if (found < 0) continue;

        RunningJob *job = &ex->jobs[found];
        BuildRule *rule = job->rule;
        int cmd_idx = job->cmd_index;

        /* Check exit status */
        int ok = WIFEXITED(status) && WEXITSTATUS(status) == 0;

        if (!ok) {
            fprintf(stderr, "amake: FAILED: %s\n", rule->commands[cmd_idx]);
            if (WIFEXITED(status))
                fprintf(stderr, "  exit code: %d\n", WEXITSTATUS(status));
            rule_failed(ex, rule);
        }
        else if (cmd_idx + 1 < rule->command_count) {
            /* More commands in this rule — run builtins in-process first */
            int next_idx = cmd_idx + 1;
            int need_fork = 0;
            while (next_idx < rule->command_count) {
                char *rewritten = NULL;
                int brc = try_run_builtin(rule->commands[next_idx],
                                          ex->amake_path, &rewritten);
                if (brc == 0) {
                    next_idx++;
                    continue;
                } else if (brc > 0) {
                    fprintf(stderr, "amake: FAILED (builtin): %s\n",
                            rule->commands[next_idx]);
                    rule_failed(ex, rule);
                    break;
                } else if (brc == -2 && rewritten) {
                    free(rule->commands[next_idx]);
                    rule->commands[next_idx] = rewritten;
                    need_fork = 1;
                    break;
                } else {
                    need_fork = 1;
                    break;
                }
            }

            if (rule->failed) {
                /* already marked failed above */
            } else if (next_idx >= rule->command_count) {
                /* All remaining commands were builtins */
                rule_completed(ex, rule);
            } else if (need_fork) {
                if (ex->verbose)
                    fprintf(stderr, "  > %s\n", rule->commands[next_idx]);

                pid_t next_pid = spawn_command(rule->commands[next_idx], rule->working_dir);
                if (next_pid < 0) {
                    fprintf(stderr, "amake: fork failed for: %s\n", rule->commands[next_idx]);
                    rule_failed(ex, rule);
                } else {
                    job->pid = next_pid;
                    job->cmd_index = next_idx;
                    continue; /* don't remove from jobs array */
                }
            }
        }
        else {
            /* All commands completed successfully */
            rule_completed(ex, rule);
        }

        /* Remove job from array */
        ex->jobs[found] = ex->jobs[--ex->job_count];
    }
#endif

    /* Summary */
    if (ex->failed_count > 0) {
        fprintf(stderr, "\namake: %d of %d rules FAILED\n",
                ex->failed_count, ex->total_dirty);
        return 1;
    }

    fprintf(stderr, "Build complete: %d rules executed.\n", ex->built_count);
    return 0;
}

void exec_free(Executor *ex) {
    free(ex->jobs);
    free(ex->ready);
    memset(ex, 0, sizeof(Executor));
}
