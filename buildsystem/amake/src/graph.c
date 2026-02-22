/*
 * graph.c — Dependency graph for amake
 *
 * Links BuildRules and BuildTargets into a DAG, performs topological sort,
 * and marks dirty nodes based on file mtimes.
 */
#include "amake.h"

/* ── Graph initialization ────────────────────────────────────────────── */

void graph_init(BuildGraph *g) {
    memset(g, 0, sizeof(BuildGraph));
}

BuildRule *graph_add_rule(BuildGraph *g) {
    if (g->rule_count >= g->rule_cap) {
        g->rule_cap = g->rule_cap ? g->rule_cap * 2 : 64;
        g->rules = amake_realloc(g->rules, sizeof(BuildRule *) * g->rule_cap);
    }
    BuildRule *r = amake_malloc(sizeof(BuildRule));
    memset(r, 0, sizeof(BuildRule));
    g->rules[g->rule_count++] = r;
    return r;
}

BuildTarget *graph_add_target(BuildGraph *g) {
    if (g->target_count >= g->target_cap) {
        g->target_cap = g->target_cap ? g->target_cap * 2 : 32;
        g->targets = amake_realloc(g->targets, sizeof(BuildTarget *) * g->target_cap);
    }
    BuildTarget *t = amake_malloc(sizeof(BuildTarget));
    memset(t, 0, sizeof(BuildTarget));
    g->targets[g->target_count++] = t;
    return t;
}

/* ── Output → Rule lookup ────────────────────────────────────────────── */

BuildRule *graph_find_rule_for_output(BuildGraph *g, const char *path) {
    int i, j;
    for (i = 0; i < g->rule_count; i++) {
        BuildRule *r = g->rules[i];
        for (j = 0; j < r->output_count; j++) {
            if (strcmp(r->outputs[j], path) == 0)
                return r;
        }
    }
    return NULL;
}

/* ── Helper: add edge between rules ──────────────────────────────────── */

static void add_blocker(BuildRule *rule, BuildRule *blocker) {
    /* Check for duplicates */
    int i;
    for (i = 0; i < rule->blocker_count; i++)
        if (rule->blockers[i] == blocker) return;

    if (rule->blocker_count >= rule->blocker_cap) {
        rule->blocker_cap = rule->blocker_cap ? rule->blocker_cap * 2 : 8;
        rule->blockers = amake_realloc(rule->blockers,
            sizeof(BuildRule *) * rule->blocker_cap);
    }
    rule->blockers[rule->blocker_count++] = blocker;
}

static void add_blocked(BuildRule *rule, BuildRule *blocked_by_us) {
    int i;
    for (i = 0; i < rule->blocked_count; i++)
        if (rule->blocked[i] == blocked_by_us) return;

    if (rule->blocked_count >= rule->blocked_cap) {
        rule->blocked_cap = rule->blocked_cap ? rule->blocked_cap * 2 : 8;
        rule->blocked = amake_realloc(rule->blocked,
            sizeof(BuildRule *) * rule->blocked_cap);
    }
    rule->blocked[rule->blocked_count++] = blocked_by_us;
}

/* ── Link dependencies ───────────────────────────────────────────────── */

void graph_link(BuildGraph *g) {
    int i, j;

    for (i = 0; i < g->rule_count; i++) {
        BuildRule *rule = g->rules[i];
        for (j = 0; j < rule->depend_count; j++) {
            BuildRule *dep = graph_find_rule_for_output(g, rule->depends[j]);
            if (dep && dep != rule) {
                add_blocker(rule, dep);
                add_blocked(dep, rule);
            }
        }
        rule->unresolved = rule->blocker_count;
    }
}

/* ── Dirty detection ─────────────────────────────────────────────────── */

/*
 * Check if a rule needs rebuilding based on file mtimes.
 * A rule is dirty if:
 *   - Any output file doesn't exist
 *   - Any source-file dependency is newer than the oldest output
 *   - Any blocker rule is dirty (transitive)
 */
static int check_rule_dirty(BuildRule *rule, MtimeCache *mc) {
    int j;

    /* Find oldest output mtime */
    time_t oldest_output = 0;
    int all_outputs_exist = 1;

    for (j = 0; j < rule->output_count; j++) {
        time_t mt = mtime_get(mc, rule->outputs[j]);
        if (mt == 0) {
            all_outputs_exist = 0;
            break;
        }
        if (oldest_output == 0 || mt < oldest_output)
            oldest_output = mt;
    }

    if (!all_outputs_exist)
        return 1;

    /* Check source dependencies */
    for (j = 0; j < rule->depend_count; j++) {
        time_t dep_mt = mtime_get(mc, rule->depends[j]);
        if (dep_mt > 0 && dep_mt > oldest_output)
            return 1;
    }

    return 0;
}

void graph_mark_dirty(BuildGraph *g, MtimeCache *mc) {
    int i;

    /* First pass: check each rule's own files */
    for (i = 0; i < g->rule_count; i++) {
        g->rules[i]->dirty = check_rule_dirty(g->rules[i], mc);
    }

    /* Propagate dirty flag: if a blocker is dirty, the dependent is dirty too.
     * Iterate until no changes (simple fixpoint). */
    int changed = 1;
    while (changed) {
        changed = 0;
        for (i = 0; i < g->rule_count; i++) {
            BuildRule *rule = g->rules[i];
            if (rule->dirty) continue;
            int j;
            for (j = 0; j < rule->blocker_count; j++) {
                if (rule->blockers[j]->dirty) {
                    rule->dirty = 1;
                    changed = 1;
                    break;
                }
            }
        }
    }
}

/* ── Collect dirty rules for a named target ──────────────────────────── */

static void collect_reachable(BuildGraph *g, BuildRule *rule,
                               BuildRule ***out, int *count, int *cap)
{
    int i;
    if (!rule || !rule->dirty || rule->done) return;

    /* Check if already in list */
    for (i = 0; i < *count; i++)
        if ((*out)[i] == rule) return;

    /* Add blockers first (depth-first) */
    for (i = 0; i < rule->blocker_count; i++)
        collect_reachable(g, rule->blockers[i], out, count, cap);

    /* Add this rule */
    if (*count >= *cap) {
        *cap = *cap ? *cap * 2 : 64;
        *out = amake_realloc(*out, sizeof(BuildRule *) * *cap);
    }
    (*out)[(*count)++] = rule;
}

int graph_collect_dirty_for_target(BuildGraph *g, const char *target_name,
                                    BuildRule ***out_dirty, int *out_count)
{
    /* Find the named target */
    BuildTarget *tgt = NULL;
    int i;
    for (i = 0; i < g->target_count; i++) {
        if (strcmp(g->targets[i]->name, target_name) == 0) {
            tgt = g->targets[i];
            break;
        }
    }
    if (!tgt) return -1; /* target not found */

    int cap = 64;
    *out_dirty = amake_malloc(sizeof(BuildRule *) * cap);
    *out_count = 0;

    /* Collect rules reachable from target's dependencies */
    for (i = 0; i < tgt->depend_count; i++) {
        BuildRule *dep_rule = graph_find_rule_for_output(g, tgt->depends[i]);
        if (dep_rule)
            collect_reachable(g, dep_rule, out_dirty, out_count, &cap);
    }

    return 0;
}

int graph_collect_dirty_all(BuildGraph *g, BuildRule ***out_dirty, int *out_count) {
    /* Collect all default (ALL) targets */
    int cap = 64;
    *out_dirty = amake_malloc(sizeof(BuildRule *) * cap);
    *out_count = 0;
    int i;

    for (i = 0; i < g->target_count; i++) {
        BuildTarget *tgt = g->targets[i];
        if (!tgt->is_default) continue;
        int j;
        for (j = 0; j < tgt->depend_count; j++) {
            BuildRule *dep = graph_find_rule_for_output(g, tgt->depends[j]);
            if (dep)
                collect_reachable(g, dep, out_dirty, out_count, &cap);
        }
    }

    return 0;
}

/* ── Cleanup ─────────────────────────────────────────────────────────── */

static void free_rule(BuildRule *r) {
    int i;
    for (i = 0; i < r->output_count; i++) free(r->outputs[i]);
    free(r->outputs);
    for (i = 0; i < r->command_count; i++) free(r->commands[i]);
    free(r->commands);
    for (i = 0; i < r->depend_count; i++) free(r->depends[i]);
    free(r->depends);
    free(r->comment);
    free(r->working_dir);
    free(r->blockers);
    free(r->blocked);
    free(r);
}

static void free_target(BuildTarget *t) {
    int i;
    free(t->name);
    for (i = 0; i < t->depend_count; i++) free(t->depends[i]);
    free(t->depends);
    for (i = 0; i < t->command_count; i++) free(t->commands[i]);
    free(t->commands);
    free(t->comment);
    free(t);
}

void graph_free(BuildGraph *g) {
    int i;
    for (i = 0; i < g->rule_count; i++) free_rule(g->rules[i]);
    free(g->rules);
    for (i = 0; i < g->target_count; i++) free_target(g->targets[i]);
    free(g->targets);
    memset(g, 0, sizeof(BuildGraph));
}
