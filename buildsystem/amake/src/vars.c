/*
 * vars.c — Variable storage and expansion for amake
 *
 * Scoped hash table for CMake variables.
 * Handles ${VAR}, $ENV{VAR}, and nested expansion.
 */
#include "amake.h"

/* ── Hash function ───────────────────────────────────────────────────── */

static unsigned int var_hash(const char *s) {
    unsigned int h = 5381;
    while (*s)
        h = h * 33 + (unsigned char)*s++;
    return h % HASH_BUCKETS;
}

/* ── Scope management ────────────────────────────────────────────────── */

VarScope *scope_new(VarScope *parent) {
    VarScope *s = amake_malloc(sizeof(VarScope));
    memset(s->buckets, 0, sizeof(s->buckets));
    s->parent = parent;
    return s;
}

void scope_free(VarScope *scope) {
    int i;
    if (!scope) return;
    for (i = 0; i < HASH_BUCKETS; i++) {
        VarEntry *e = scope->buckets[i];
        while (e) {
            VarEntry *next = e->next;
            free(e->name);
            free(e->value);
            free(e);
            e = next;
        }
    }
    free(scope);
}

void scope_set(VarScope *scope, const char *name, const char *value) {
    unsigned int h = var_hash(name);
    VarEntry *e = scope->buckets[h];
    while (e) {
        if (strcmp(e->name, name) == 0) {
            free(e->value);
            e->value = amake_strdup(value ? value : "");
            return;
        }
        e = e->next;
    }
    /* New entry */
    e = amake_malloc(sizeof(VarEntry));
    e->name = amake_strdup(name);
    e->value = amake_strdup(value ? value : "");
    e->next = scope->buckets[h];
    scope->buckets[h] = e;
}

const char *scope_get(VarScope *scope, const char *name) {
    while (scope) {
        unsigned int h = var_hash(name);
        VarEntry *e = scope->buckets[h];
        while (e) {
            if (strcmp(e->name, name) == 0)
                return e->value;
            e = e->next;
        }
        scope = scope->parent;
    }
    return NULL;
}

/* ── Variable expansion ──────────────────────────────────────────────── */

/*
 * Expand ${VAR} and $ENV{VAR} references in a string.
 * Returns a heap-allocated string.
 */
char *expand_vars(AmakeCtx *ctx, const char *input) {
    if (!input) return amake_strdup("");

    size_t cap = strlen(input) * 2 + 64;
    char *out = amake_malloc(cap);
    size_t len = 0;
    const char *p = input;

    while (*p) {
        /* Ensure space */
        if (len + 256 > cap) {
            cap *= 2;
            out = amake_realloc(out, cap);
        }

        if (p[0] == '$' && p[1] == '{') {
            /* ${VAR} — find matching } */
            const char *start = p + 2;
            int depth = 1;
            const char *q = start;
            while (*q && depth > 0) {
                if (q[0] == '$' && q[1] == '{') { depth++; q += 2; }
                else if (*q == '}') { depth--; if (depth > 0) q++; }
                else q++;
            }
            if (depth != 0) {
                /* Unmatched — copy literally */
                out[len++] = *p++;
                continue;
            }
            /* Extract variable name (may contain nested ${}) */
            char *varname = amake_strndup(start, (size_t)(q - start));
            /* Recursively expand the variable name itself (for ${${INNER}}) */
            char *expanded_name = expand_vars(ctx, varname);
            free(varname);

            const char *val = scope_get(ctx->current_scope, expanded_name);
            free(expanded_name);

            if (val) {
                size_t vlen = strlen(val);
                while (len + vlen + 1 > cap) { cap *= 2; out = amake_realloc(out, cap); }
                memcpy(out + len, val, vlen);
                len += vlen;
            }
            p = q + 1; /* skip past '}' */
        }
        else if (p[0] == '$' && p[1] == 'E' && p[2] == 'N' && p[3] == 'V' && p[4] == '{') {
            /* $ENV{VAR} */
            const char *start = p + 5;
            const char *q = strchr(start, '}');
            if (!q) {
                out[len++] = *p++;
                continue;
            }
            char *envname = amake_strndup(start, (size_t)(q - start));
            const char *val = getenv(envname);
            free(envname);
            if (val) {
                size_t vlen = strlen(val);
                while (len + vlen + 1 > cap) { cap *= 2; out = amake_realloc(out, cap); }
                memcpy(out + len, val, vlen);
                len += vlen;
            }
            p = q + 1;
        }
        else {
            out[len++] = *p++;
        }
    }
    out[len] = '\0';
    return out;
}

/*
 * Expand variables in an argument list.
 * Unquoted arguments with semicolons are split into multiple args.
 * Returns heap-allocated arrays.
 */
void expand_args(AmakeCtx *ctx, char **args, int argc,
                 char ***out_args, int *out_argc)
{
    int cap = argc * 2 + 16;
    char **result = amake_malloc(sizeof(char *) * cap);
    int count = 0;
    int i;

    for (i = 0; i < argc; i++) {
        /* Check if arg was quoted (lexer marks with \x01 prefix) */
        int quoted = (args[i][0] == '\x01');
        const char *raw = quoted ? args[i] + 1 : args[i];
        char *expanded = expand_vars(ctx, raw);

        if (quoted) {
            /* Quoted arg: preserve semicolons, no splitting */
            if (count + 1 >= cap) {
                cap *= 2;
                result = amake_realloc(result, sizeof(char *) * cap);
            }
            result[count++] = expanded; /* take ownership */
        } else {
            /* Unquoted arg: split on semicolons (CMake list separator) */
            char *tok = expanded;
            char *semi;
            while ((semi = strchr(tok, ';')) != NULL) {
                if (count + 1 >= cap) {
                    cap *= 2;
                    result = amake_realloc(result, sizeof(char *) * cap);
                }
                result[count++] = amake_strndup(tok, (size_t)(semi - tok));
                tok = semi + 1;
            }
            /* Last segment (or only segment if no semicolons) */
            if (count + 1 >= cap) {
                cap *= 2;
                result = amake_realloc(result, sizeof(char *) * cap);
            }
            result[count++] = amake_strdup(tok);
            free(expanded);
        }
    }

    *out_args = result;
    *out_argc = count;
}
