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

void exec_init(Executor *ex, int max_jobs, int verbose) {
    memset(ex, 0, sizeof(Executor));
    ex->max_jobs = max_jobs > 0 ? max_jobs : 4;
    ex->verbose = verbose;
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

            /* Spawn first command */
            pid_t pid = spawn_command(rule->commands[0], rule->working_dir);
            if (pid < 0) {
                fprintf(stderr, "amake: fork failed for: %s\n", rule->commands[0]);
                rule_failed(ex, rule);
                continue;
            }

            ex->jobs[ex->job_count].pid = pid;
            ex->jobs[ex->job_count].rule = rule;
            ex->jobs[ex->job_count].cmd_index = 0;
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
            /* More commands in this rule — spawn next */
            int next_idx = cmd_idx + 1;
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
