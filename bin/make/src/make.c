/*
 * make - A minimal POSIX-compatible make utility for anyOS
 *
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * Supports:
 *   - Explicit rules (target: prereqs \n\trecipe)
 *   - Pattern rules (%.o: %.c)
 *   - Variables: =, :=, ?=, +=
 *   - Automatic variables: $@, $<, $^, $*, $(@D), $(@F)
 *   - Built-in functions: $(wildcard ...), $(patsubst ...), $(notdir ...),
 *     $(basename ...), $(addprefix ...), $(addsuffix ...), $(filter ...),
 *     $(filter-out ...), $(sort ...), $(word ...), $(words ...), $(shell ...)
 *   - .PHONY targets
 *   - include directive
 *   - -C dir (change directory)
 *   - -f file (alternate makefile)
 *   - -n (dry run), -s (silent), -B (unconditional)
 *   - Implicit rules for .c -> .o
 *   - Command-line variable overrides (VAR=value)
 *   - @ prefix (silent recipe line), - prefix (ignore errors)
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <dirent.h>
#include <errno.h>
#include <unistd.h>

/* =====================================================================
 * Configuration & limits
 * ===================================================================== */

#define MAX_TARGETS     512
#define MAX_VARS        256
#define MAX_PREREQS     128
#define MAX_RECIPES     64
#define MAX_PHONIES     128
#define MAX_PATTERNS    64
#define MAX_LINE        4096
#define MAX_EXPANDED    8192
#define MAX_INCLUDES    8

/* =====================================================================
 * Data structures
 * ===================================================================== */

typedef struct {
    char *name;
    char *prereqs[MAX_PREREQS];
    int  nprereqs;
    char *recipes[MAX_RECIPES];
    int  nrecipes;
    int  visited;    /* cycle detection & build state: 0=unseen, 1=visiting, 2=done */
    int  built;      /* was recipe executed? */
} Target;

typedef struct {
    char *stem;      /* pattern stem matched by % */
    char *target;    /* full target pattern e.g. "%.o" */
    char *prereq;    /* full prereq pattern e.g. "%.c" */
    char *recipes[MAX_RECIPES];
    int  nrecipes;
} Pattern;

typedef struct {
    char *name;
    char *value;
    int  override;   /* set via command line — cannot be overridden by Makefile */
} Variable;

static Target   targets[MAX_TARGETS];
static int      ntargets = 0;

static Pattern  patterns[MAX_PATTERNS];
static int      npatterns = 0;

static Variable vars[MAX_VARS];
static int      nvars = 0;

static char     *phonies[MAX_PHONIES];
static int      nphonies = 0;

static char     *default_target = NULL;

/* flags */
static int      flag_dry_run   = 0;   /* -n */
static int      flag_silent    = 0;   /* -s */
static int      flag_always    = 0;   /* -B */
static int      flag_keep_going = 0;  /* -k */

/* =====================================================================
 * String helpers
 * ===================================================================== */

static char *my_strdup(const char *s) {
    if (!s) return NULL;
    size_t len = strlen(s);
    char *p = malloc(len + 1);
    if (p) memcpy(p, s, len + 1);
    return p;
}

static char *trim(char *s) {
    while (*s == ' ' || *s == '\t') s++;
    char *end = s + strlen(s) - 1;
    while (end > s && (*end == ' ' || *end == '\t' || *end == '\n' || *end == '\r'))
        *end-- = '\0';
    return s;
}

/* Check if a string matches a pattern with a single '%' wildcard.
 * If match, returns the stem (part matched by %). Caller must free. */
static char *pattern_match(const char *pattern, const char *str) {
    const char *pct = strchr(pattern, '%');
    if (!pct) {
        return strcmp(pattern, str) == 0 ? my_strdup("") : NULL;
    }
    size_t prefix_len = pct - pattern;
    size_t suffix_len = strlen(pct + 1);
    size_t str_len = strlen(str);

    if (str_len < prefix_len + suffix_len) return NULL;
    if (prefix_len > 0 && strncmp(pattern, str, prefix_len) != 0) return NULL;
    if (suffix_len > 0 && strcmp(str + str_len - suffix_len, pct + 1) != 0) return NULL;

    size_t stem_len = str_len - prefix_len - suffix_len;
    char *stem = malloc(stem_len + 1);
    memcpy(stem, str + prefix_len, stem_len);
    stem[stem_len] = '\0';
    return stem;
}

/* Apply a pattern substitution: replace % in pattern with stem. */
static char *pattern_subst(const char *pattern, const char *stem) {
    const char *pct = strchr(pattern, '%');
    if (!pct) return my_strdup(pattern);

    size_t prefix_len = pct - pattern;
    size_t suffix_len = strlen(pct + 1);
    size_t stem_len = strlen(stem);
    char *result = malloc(prefix_len + stem_len + suffix_len + 1);
    memcpy(result, pattern, prefix_len);
    memcpy(result + prefix_len, stem, stem_len);
    memcpy(result + prefix_len + stem_len, pct + 1, suffix_len + 1);
    return result;
}

/* =====================================================================
 * Variable management
 * ===================================================================== */

static Variable *find_var(const char *name) {
    for (int i = 0; i < nvars; i++) {
        if (strcmp(vars[i].name, name) == 0) return &vars[i];
    }
    return NULL;
}

static void set_var(const char *name, const char *value, int override_flag) {
    Variable *v = find_var(name);
    if (v) {
        if (v->override && !override_flag) return; /* cmd-line vars win */
        free(v->value);
        v->value = my_strdup(value);
        if (override_flag) v->override = 1;
    } else if (nvars < MAX_VARS) {
        vars[nvars].name = my_strdup(name);
        vars[nvars].value = my_strdup(value);
        vars[nvars].override = override_flag;
        nvars++;
    }
}

static void append_var(const char *name, const char *value) {
    Variable *v = find_var(name);
    if (v) {
        size_t old_len = strlen(v->value);
        size_t add_len = strlen(value);
        char *new_val = malloc(old_len + 1 + add_len + 1);
        memcpy(new_val, v->value, old_len);
        new_val[old_len] = ' ';
        memcpy(new_val + old_len + 1, value, add_len + 1);
        free(v->value);
        v->value = new_val;
    } else {
        set_var(name, value, 0);
    }
}

static const char *get_var(const char *name) {
    Variable *v = find_var(name);
    if (v) return v->value;
    /* Fall back to environment */
    const char *env = getenv(name);
    return env ? env : "";
}

/* =====================================================================
 * Built-in functions
 * ===================================================================== */

/* Forward declaration */
static void expand_vars(const char *input, char *output, size_t outsize,
                        const char *target, const char *first_prereq,
                        const char *all_prereqs, const char *stem);

/* $(wildcard pattern) — expand glob patterns */
static void func_wildcard(const char *arg, char *out, size_t outsize) {
    /* arg may contain multiple space-separated patterns */
    char buf[MAX_LINE];
    strncpy(buf, arg, sizeof(buf) - 1);
    buf[sizeof(buf) - 1] = '\0';

    size_t pos = 0;
    char *tok = strtok(buf, " \t");
    while (tok) {
        /* Extract directory and file pattern */
        char dir_path[MAX_LINE] = ".";
        const char *file_pat = tok;

        char *last_slash = strrchr(tok, '/');
        if (last_slash) {
            size_t dlen = last_slash - tok;
            memcpy(dir_path, tok, dlen);
            dir_path[dlen] = '\0';
            file_pat = last_slash + 1;
        }

        DIR *d = opendir(dir_path);
        if (d) {
            struct dirent *ent;
            while ((ent = readdir(d)) != NULL) {
                if (ent->d_name[0] == '.' && file_pat[0] != '.') continue;

                /* Simple wildcard matching: *.c matches files ending in .c */
                int match = 0;
                if (strcmp(file_pat, "*") == 0) {
                    match = 1;
                } else if (file_pat[0] == '*' && file_pat[1] == '.') {
                    /* *.ext pattern */
                    const char *ext = file_pat + 1;
                    size_t elen = strlen(ext);
                    size_t nlen = strlen(ent->d_name);
                    if (nlen >= elen && strcmp(ent->d_name + nlen - elen, ext) == 0)
                        match = 1;
                } else if (strcmp(file_pat, ent->d_name) == 0) {
                    match = 1;
                }

                if (match) {
                    char full[MAX_LINE];
                    if (strcmp(dir_path, ".") == 0)
                        snprintf(full, sizeof(full), "%s", ent->d_name);
                    else
                        snprintf(full, sizeof(full), "%s/%s", dir_path, ent->d_name);
                    size_t flen = strlen(full);
                    if (pos + flen + 2 < outsize) {
                        if (pos > 0) out[pos++] = ' ';
                        memcpy(out + pos, full, flen);
                        pos += flen;
                    }
                }
            }
            closedir(d);
        }
        tok = strtok(NULL, " \t");
    }
    out[pos] = '\0';
}

/* $(patsubst pattern,replacement,text) */
static void func_patsubst(const char *args, char *out, size_t outsize) {
    /* Parse 3 comma-separated arguments */
    char buf[MAX_EXPANDED];
    strncpy(buf, args, sizeof(buf) - 1);
    buf[sizeof(buf) - 1] = '\0';

    char *pattern = buf;
    char *comma1 = strchr(pattern, ',');
    if (!comma1) { out[0] = '\0'; return; }
    *comma1 = '\0';
    char *replacement = trim(comma1 + 1);
    char *comma2 = strchr(replacement, ',');
    if (!comma2) { out[0] = '\0'; return; }
    *comma2 = '\0';
    char *text = trim(comma2 + 1);
    pattern = trim(pattern);
    replacement = trim(replacement);

    size_t pos = 0;
    char *tok = strtok(text, " \t");
    while (tok) {
        char *stem = pattern_match(pattern, tok);
        const char *result;
        char subst[MAX_LINE];
        if (stem) {
            char *s = pattern_subst(replacement, stem);
            strncpy(subst, s, sizeof(subst) - 1);
            subst[sizeof(subst) - 1] = '\0';
            free(s);
            result = subst;
            free(stem);
        } else {
            result = tok;
        }
        size_t rlen = strlen(result);
        if (pos + rlen + 2 < outsize) {
            if (pos > 0) out[pos++] = ' ';
            memcpy(out + pos, result, rlen);
            pos += rlen;
        }
        tok = strtok(NULL, " \t");
    }
    out[pos] = '\0';
}

/* $(notdir names...) — extract file part of each name */
static void func_notdir(const char *arg, char *out, size_t outsize) {
    char buf[MAX_EXPANDED];
    strncpy(buf, arg, sizeof(buf) - 1);
    buf[sizeof(buf) - 1] = '\0';
    size_t pos = 0;
    char *tok = strtok(buf, " \t");
    while (tok) {
        char *slash = strrchr(tok, '/');
        const char *base = slash ? slash + 1 : tok;
        size_t blen = strlen(base);
        if (pos + blen + 2 < outsize) {
            if (pos > 0) out[pos++] = ' ';
            memcpy(out + pos, base, blen);
            pos += blen;
        }
        tok = strtok(NULL, " \t");
    }
    out[pos] = '\0';
}

/* $(basename names...) — remove suffix from each name */
static void func_basename(const char *arg, char *out, size_t outsize) {
    char buf[MAX_EXPANDED];
    strncpy(buf, arg, sizeof(buf) - 1);
    buf[sizeof(buf) - 1] = '\0';
    size_t pos = 0;
    char *tok = strtok(buf, " \t");
    while (tok) {
        char *dot = strrchr(tok, '.');
        char *slash = strrchr(tok, '/');
        /* Only strip suffix if dot is after last slash */
        if (dot && (!slash || dot > slash)) {
            size_t blen = dot - tok;
            if (pos + blen + 2 < outsize) {
                if (pos > 0) out[pos++] = ' ';
                memcpy(out + pos, tok, blen);
                pos += blen;
            }
        } else {
            size_t tlen = strlen(tok);
            if (pos + tlen + 2 < outsize) {
                if (pos > 0) out[pos++] = ' ';
                memcpy(out + pos, tok, tlen);
                pos += tlen;
            }
        }
        tok = strtok(NULL, " \t");
    }
    out[pos] = '\0';
}

/* $(addprefix prefix,names...) */
static void func_addprefix(const char *args, char *out, size_t outsize) {
    char buf[MAX_EXPANDED];
    strncpy(buf, args, sizeof(buf) - 1);
    buf[sizeof(buf) - 1] = '\0';
    char *comma = strchr(buf, ',');
    if (!comma) { out[0] = '\0'; return; }
    *comma = '\0';
    char *prefix = trim(buf);
    char *names = trim(comma + 1);
    size_t plen = strlen(prefix);
    size_t pos = 0;
    char *tok = strtok(names, " \t");
    while (tok) {
        size_t tlen = strlen(tok);
        if (pos + plen + tlen + 2 < outsize) {
            if (pos > 0) out[pos++] = ' ';
            memcpy(out + pos, prefix, plen);
            pos += plen;
            memcpy(out + pos, tok, tlen);
            pos += tlen;
        }
        tok = strtok(NULL, " \t");
    }
    out[pos] = '\0';
}

/* $(addsuffix suffix,names...) */
static void func_addsuffix(const char *args, char *out, size_t outsize) {
    char buf[MAX_EXPANDED];
    strncpy(buf, args, sizeof(buf) - 1);
    buf[sizeof(buf) - 1] = '\0';
    char *comma = strchr(buf, ',');
    if (!comma) { out[0] = '\0'; return; }
    *comma = '\0';
    char *suffix = trim(buf);
    char *names = trim(comma + 1);
    size_t slen = strlen(suffix);
    size_t pos = 0;
    char *tok = strtok(names, " \t");
    while (tok) {
        size_t tlen = strlen(tok);
        if (pos + tlen + slen + 2 < outsize) {
            if (pos > 0) out[pos++] = ' ';
            memcpy(out + pos, tok, tlen);
            pos += tlen;
            memcpy(out + pos, suffix, slen);
            pos += slen;
        }
        tok = strtok(NULL, " \t");
    }
    out[pos] = '\0';
}

/* $(filter pattern...,text) */
static void func_filter(const char *args, char *out, size_t outsize) {
    char buf[MAX_EXPANDED];
    strncpy(buf, args, sizeof(buf) - 1);
    buf[sizeof(buf) - 1] = '\0';
    char *comma = strchr(buf, ',');
    if (!comma) { out[0] = '\0'; return; }
    *comma = '\0';
    char *pats = trim(buf);
    char *text = trim(comma + 1);

    /* Collect patterns */
    char pat_buf[MAX_LINE];
    strncpy(pat_buf, pats, sizeof(pat_buf) - 1);
    pat_buf[sizeof(pat_buf) - 1] = '\0';
    char *pat_list[64];
    int npats = 0;
    char *p = strtok(pat_buf, " \t");
    while (p && npats < 64) { pat_list[npats++] = p; p = strtok(NULL, " \t"); }

    size_t pos = 0;
    char *tok = strtok(text, " \t");
    while (tok) {
        for (int i = 0; i < npats; i++) {
            char *stem = pattern_match(pat_list[i], tok);
            if (stem) {
                free(stem);
                size_t tlen = strlen(tok);
                if (pos + tlen + 2 < outsize) {
                    if (pos > 0) out[pos++] = ' ';
                    memcpy(out + pos, tok, tlen);
                    pos += tlen;
                }
                break;
            }
        }
        tok = strtok(NULL, " \t");
    }
    out[pos] = '\0';
}

/* $(filter-out pattern...,text) */
static void func_filter_out(const char *args, char *out, size_t outsize) {
    char buf[MAX_EXPANDED];
    strncpy(buf, args, sizeof(buf) - 1);
    buf[sizeof(buf) - 1] = '\0';
    char *comma = strchr(buf, ',');
    if (!comma) { out[0] = '\0'; return; }
    *comma = '\0';
    char *pats = trim(buf);
    char *text = trim(comma + 1);

    char pat_buf[MAX_LINE];
    strncpy(pat_buf, pats, sizeof(pat_buf) - 1);
    pat_buf[sizeof(pat_buf) - 1] = '\0';
    char *pat_list[64];
    int npats = 0;
    char *p = strtok(pat_buf, " \t");
    while (p && npats < 64) { pat_list[npats++] = p; p = strtok(NULL, " \t"); }

    size_t pos = 0;
    char *tok = strtok(text, " \t");
    while (tok) {
        int excluded = 0;
        for (int i = 0; i < npats; i++) {
            char *stem = pattern_match(pat_list[i], tok);
            if (stem) { free(stem); excluded = 1; break; }
        }
        if (!excluded) {
            size_t tlen = strlen(tok);
            if (pos + tlen + 2 < outsize) {
                if (pos > 0) out[pos++] = ' ';
                memcpy(out + pos, tok, tlen);
                pos += tlen;
            }
        }
        tok = strtok(NULL, " \t");
    }
    out[pos] = '\0';
}

/* $(sort list) — sort and remove duplicates */
static void func_sort(const char *arg, char *out, size_t outsize) {
    char buf[MAX_EXPANDED];
    strncpy(buf, arg, sizeof(buf) - 1);
    buf[sizeof(buf) - 1] = '\0';
    char *words[512];
    int nwords = 0;
    char *tok = strtok(buf, " \t");
    while (tok && nwords < 512) { words[nwords++] = tok; tok = strtok(NULL, " \t"); }

    /* Simple insertion sort */
    for (int i = 1; i < nwords; i++) {
        char *key = words[i];
        int j = i - 1;
        while (j >= 0 && strcmp(words[j], key) > 0) {
            words[j + 1] = words[j];
            j--;
        }
        words[j + 1] = key;
    }

    size_t pos = 0;
    const char *prev = NULL;
    for (int i = 0; i < nwords; i++) {
        if (prev && strcmp(prev, words[i]) == 0) continue; /* skip dupes */
        size_t wlen = strlen(words[i]);
        if (pos + wlen + 2 < outsize) {
            if (pos > 0) out[pos++] = ' ';
            memcpy(out + pos, words[i], wlen);
            pos += wlen;
        }
        prev = words[i];
    }
    out[pos] = '\0';
}

/* $(words text) — count words */
static void func_words(const char *arg, char *out, size_t outsize) {
    char buf[MAX_EXPANDED];
    strncpy(buf, arg, sizeof(buf) - 1);
    buf[sizeof(buf) - 1] = '\0';
    int n = 0;
    char *tok = strtok(buf, " \t");
    while (tok) { n++; tok = strtok(NULL, " \t"); }
    snprintf(out, outsize, "%d", n);
}

/* $(word n,text) — extract nth word (1-based) */
static void func_word(const char *args, char *out, size_t outsize) {
    char buf[MAX_EXPANDED];
    strncpy(buf, args, sizeof(buf) - 1);
    buf[sizeof(buf) - 1] = '\0';
    char *comma = strchr(buf, ',');
    if (!comma) { out[0] = '\0'; return; }
    *comma = '\0';
    int n = atoi(trim(buf));
    char *text = trim(comma + 1);
    int idx = 0;
    char *tok = strtok(text, " \t");
    while (tok) {
        idx++;
        if (idx == n) {
            strncpy(out, tok, outsize - 1);
            out[outsize - 1] = '\0';
            return;
        }
        tok = strtok(NULL, " \t");
    }
    out[0] = '\0';
}

/* $(shell command) — run command and capture stdout */
static void func_shell(const char *cmd, char *out, size_t outsize) {
    /* On anyOS, we use a temp file approach since popen may not exist */
    const char *tmpfile = "/tmp/.make_shell_out";
    char full_cmd[MAX_LINE];
    snprintf(full_cmd, sizeof(full_cmd), "%s > %s", cmd, tmpfile);
    system(full_cmd);

    FILE *f = fopen(tmpfile, "r");
    if (!f) { out[0] = '\0'; return; }
    size_t pos = 0;
    int c;
    while ((c = fgetc(f)) != EOF && pos + 1 < outsize) {
        /* Replace newlines with spaces */
        if (c == '\n') c = ' ';
        out[pos++] = (char)c;
    }
    /* Trim trailing spaces */
    while (pos > 0 && out[pos - 1] == ' ') pos--;
    out[pos] = '\0';
    fclose(f);
    unlink(tmpfile);
}

/* =====================================================================
 * Variable expansion
 * ===================================================================== */

/* Find matching closing paren, handling nesting */
static const char *find_close_paren(const char *s) {
    int depth = 1;
    while (*s) {
        if (*s == '(') depth++;
        else if (*s == ')') { depth--; if (depth == 0) return s; }
        s++;
    }
    return NULL;
}

/* Expand $(VAR), $(func args), $@, $<, $^, $* */
static void expand_vars(const char *input, char *output, size_t outsize,
                        const char *target, const char *first_prereq,
                        const char *all_prereqs, const char *stem) {
    size_t pos = 0;
    const char *p = input;

    while (*p && pos + 1 < outsize) {
        if (*p == '$') {
            p++;
            if (*p == '$') {
                /* $$ → literal $ */
                output[pos++] = '$';
                p++;
            } else if (*p == '@') {
                p++;
                if (*p == 'D' || *p == 'F') {
                    /* $(@D) or $(@F) — but without parens */
                    /* Actually these need parens: $(@D) */
                    /* This handles the no-paren case which isn't standard, skip */
                    const char *val = target ? target : "";
                    size_t vlen = strlen(val);
                    if (pos + vlen < outsize) { memcpy(output + pos, val, vlen); pos += vlen; }
                } else {
                    /* $@ = target */
                    const char *val = target ? target : "";
                    size_t vlen = strlen(val);
                    if (pos + vlen < outsize) { memcpy(output + pos, val, vlen); pos += vlen; }
                }
            } else if (*p == '<') {
                /* $< = first prerequisite */
                p++;
                const char *val = first_prereq ? first_prereq : "";
                size_t vlen = strlen(val);
                if (pos + vlen < outsize) { memcpy(output + pos, val, vlen); pos += vlen; }
            } else if (*p == '^') {
                /* $^ = all prerequisites */
                p++;
                const char *val = all_prereqs ? all_prereqs : "";
                size_t vlen = strlen(val);
                if (pos + vlen < outsize) { memcpy(output + pos, val, vlen); pos += vlen; }
            } else if (*p == '*') {
                /* $* = stem (pattern match) */
                p++;
                const char *val = stem ? stem : "";
                size_t vlen = strlen(val);
                if (pos + vlen < outsize) { memcpy(output + pos, val, vlen); pos += vlen; }
            } else if (*p == '(') {
                /* $(NAME) or $(func args) */
                p++;
                const char *close = find_close_paren(p);
                if (!close) {
                    output[pos++] = '$';
                    output[pos++] = '(';
                    continue;
                }
                size_t inner_len = close - p;
                char inner[MAX_EXPANDED];
                if (inner_len >= sizeof(inner)) inner_len = sizeof(inner) - 1;
                memcpy(inner, p, inner_len);
                inner[inner_len] = '\0';
                p = close + 1;

                /* Recursively expand the inner content first */
                char expanded_inner[MAX_EXPANDED];
                expand_vars(inner, expanded_inner, sizeof(expanded_inner),
                           target, first_prereq, all_prereqs, stem);

                /* Check for automatic variables with modifiers */
                if (strcmp(expanded_inner, "@D") == 0) {
                    /* $(@D) = directory part of $@ */
                    const char *t = target ? target : "";
                    char tmp[MAX_LINE];
                    strncpy(tmp, t, sizeof(tmp) - 1);
                    tmp[sizeof(tmp) - 1] = '\0';
                    char *sl = strrchr(tmp, '/');
                    const char *val = sl ? (tmp[sl == tmp ? 1 : 0] = '\0', *tmp ? tmp : "/") : ".";
                    if (sl && sl != tmp) *sl = '\0';
                    else if (sl) { tmp[0] = '/'; tmp[1] = '\0'; }
                    size_t vlen = strlen(sl ? tmp : val);
                    if (pos + vlen < outsize) { memcpy(output + pos, sl ? tmp : val, vlen); pos += vlen; }
                    continue;
                }
                if (strcmp(expanded_inner, "@F") == 0) {
                    /* $(@F) = file part of $@ */
                    const char *t = target ? target : "";
                    const char *sl = strrchr(t, '/');
                    const char *val = sl ? sl + 1 : t;
                    size_t vlen = strlen(val);
                    if (pos + vlen < outsize) { memcpy(output + pos, val, vlen); pos += vlen; }
                    continue;
                }

                /* Check for function calls: first word is function name */
                char func_name[64] = {0};
                const char *func_args = NULL;
                {
                    const char *sp = strchr(expanded_inner, ' ');
                    if (sp) {
                        size_t nlen = sp - expanded_inner;
                        if (nlen < sizeof(func_name)) {
                            memcpy(func_name, expanded_inner, nlen);
                            func_name[nlen] = '\0';
                            func_args = sp + 1;
                            while (*func_args == ' ') func_args++;
                        }
                    }
                }

                char func_result[MAX_EXPANDED];
                func_result[0] = '\0';
                int is_func = 0;

                if (strcmp(func_name, "wildcard") == 0 && func_args) {
                    func_wildcard(func_args, func_result, sizeof(func_result));
                    is_func = 1;
                } else if (strcmp(func_name, "patsubst") == 0 && func_args) {
                    func_patsubst(func_args, func_result, sizeof(func_result));
                    is_func = 1;
                } else if (strcmp(func_name, "notdir") == 0 && func_args) {
                    func_notdir(func_args, func_result, sizeof(func_result));
                    is_func = 1;
                } else if (strcmp(func_name, "basename") == 0 && func_args) {
                    func_basename(func_args, func_result, sizeof(func_result));
                    is_func = 1;
                } else if (strcmp(func_name, "addprefix") == 0 && func_args) {
                    func_addprefix(func_args, func_result, sizeof(func_result));
                    is_func = 1;
                } else if (strcmp(func_name, "addsuffix") == 0 && func_args) {
                    func_addsuffix(func_args, func_result, sizeof(func_result));
                    is_func = 1;
                } else if (strcmp(func_name, "filter") == 0 && func_args) {
                    func_filter(func_args, func_result, sizeof(func_result));
                    is_func = 1;
                } else if (strcmp(func_name, "filter-out") == 0 && func_args) {
                    func_filter_out(func_args, func_result, sizeof(func_result));
                    is_func = 1;
                } else if (strcmp(func_name, "sort") == 0 && func_args) {
                    func_sort(func_args, func_result, sizeof(func_result));
                    is_func = 1;
                } else if (strcmp(func_name, "words") == 0 && func_args) {
                    func_words(func_args, func_result, sizeof(func_result));
                    is_func = 1;
                } else if (strcmp(func_name, "word") == 0 && func_args) {
                    func_word(func_args, func_result, sizeof(func_result));
                    is_func = 1;
                } else if (strcmp(func_name, "shell") == 0 && func_args) {
                    func_shell(func_args, func_result, sizeof(func_result));
                    is_func = 1;
                }

                if (is_func) {
                    size_t rlen = strlen(func_result);
                    if (pos + rlen < outsize) { memcpy(output + pos, func_result, rlen); pos += rlen; }
                } else {
                    /* Plain variable lookup */
                    const char *val = get_var(expanded_inner);
                    /* Recursively expand the value */
                    char rec[MAX_EXPANDED];
                    expand_vars(val, rec, sizeof(rec), target, first_prereq, all_prereqs, stem);
                    size_t vlen = strlen(rec);
                    if (pos + vlen < outsize) { memcpy(output + pos, rec, vlen); pos += vlen; }
                }
            } else if (*p == '{') {
                /* ${NAME} — same as $(NAME) */
                p++;
                const char *close = strchr(p, '}');
                if (!close) {
                    output[pos++] = '$';
                    output[pos++] = '{';
                    continue;
                }
                size_t nlen = close - p;
                char name[MAX_LINE];
                if (nlen >= sizeof(name)) nlen = sizeof(name) - 1;
                memcpy(name, p, nlen);
                name[nlen] = '\0';
                p = close + 1;

                const char *val = get_var(name);
                char rec[MAX_EXPANDED];
                expand_vars(val, rec, sizeof(rec), target, first_prereq, all_prereqs, stem);
                size_t vlen = strlen(rec);
                if (pos + vlen < outsize) { memcpy(output + pos, rec, vlen); pos += vlen; }
            } else {
                /* Single-char variable like $X (not a special char) */
                char name[2] = { *p, '\0' };
                p++;
                const char *val = get_var(name);
                size_t vlen = strlen(val);
                if (pos + vlen < outsize) { memcpy(output + pos, val, vlen); pos += vlen; }
            }
        } else {
            output[pos++] = *p++;
        }
    }
    output[pos] = '\0';
}

/* =====================================================================
 * Target management
 * ===================================================================== */

static Target *find_target(const char *name) {
    for (int i = 0; i < ntargets; i++) {
        if (strcmp(targets[i].name, name) == 0) return &targets[i];
    }
    return NULL;
}

static Target *add_target(const char *name) {
    Target *t = find_target(name);
    if (t) return t;
    if (ntargets >= MAX_TARGETS) {
        fprintf(stderr, "make: too many targets\n");
        exit(2);
    }
    t = &targets[ntargets++];
    t->name = my_strdup(name);
    t->nprereqs = 0;
    t->nrecipes = 0;
    t->visited = 0;
    t->built = 0;
    return t;
}

static int is_phony(const char *name) {
    for (int i = 0; i < nphonies; i++) {
        if (strcmp(phonies[i], name) == 0) return 1;
    }
    return 0;
}

/* Get file mtime. Returns 0 if file doesn't exist. */
static unsigned int file_mtime(const char *path) {
    struct stat st;
    if (stat(path, &st) != 0) return 0;
    return st.st_mtime;
}

/* =====================================================================
 * Makefile parser
 * ===================================================================== */

static void parse_makefile(const char *filename, int depth);

static void parse_line_continuation(FILE *f, char *line, size_t maxlen) {
    /* Handle backslash continuation */
    for (;;) {
        size_t len = strlen(line);
        if (len > 0 && line[len - 1] == '\n') { line[--len] = '\0'; }
        if (len > 0 && line[len - 1] == '\r') { line[--len] = '\0'; }
        if (len > 0 && line[len - 1] == '\\') {
            line[--len] = ' '; /* replace \ with space */
            char next[MAX_LINE];
            if (fgets(next, sizeof(next), f)) {
                size_t nlen = strlen(next);
                if (len + nlen < maxlen) {
                    memcpy(line + len, next, nlen + 1);
                }
            } else {
                break;
            }
        } else {
            break;
        }
    }
}

static void parse_makefile(const char *filename, int depth) {
    if (depth > MAX_INCLUDES) {
        fprintf(stderr, "make: too many include levels\n");
        return;
    }

    FILE *f = fopen(filename, "r");
    if (!f) {
        if (depth > 0) {
            fprintf(stderr, "make: %s: No such file\n", filename);
        }
        return;
    }

    char line[MAX_LINE];
    Target *current_target = NULL;

    while (fgets(line, sizeof(line), f)) {
        parse_line_continuation(f, line, sizeof(line));

        /* Recipe lines start with tab */
        if (line[0] == '\t') {
            if (current_target) {
                if (current_target->nrecipes < MAX_RECIPES) {
                    char *recipe = trim(line + 1);
                    if (*recipe) {
                        current_target->recipes[current_target->nrecipes++] = my_strdup(recipe);
                    }
                }
            }
            continue;
        }

        char *trimmed = trim(line);

        /* Skip empty lines and comments */
        if (*trimmed == '\0' || *trimmed == '#') {
            current_target = NULL;
            continue;
        }

        /* include directive */
        if (strncmp(trimmed, "include ", 8) == 0 || strncmp(trimmed, "-include ", 9) == 0) {
            int optional = (trimmed[0] == '-');
            char *inc_file = trim(trimmed + (optional ? 9 : 8));
            char expanded[MAX_EXPANDED];
            expand_vars(inc_file, expanded, sizeof(expanded), NULL, NULL, NULL, NULL);
            /* May be multiple files */
            char *tok = strtok(expanded, " \t");
            while (tok) {
                parse_makefile(tok, depth + 1);
                tok = strtok(NULL, " \t");
            }
            current_target = NULL;
            continue;
        }

        /* .PHONY: targets */
        if (strncmp(trimmed, ".PHONY:", 7) == 0 || strncmp(trimmed, ".PHONY :", 8) == 0) {
            char *list = strchr(trimmed, ':') + 1;
            char expanded[MAX_EXPANDED];
            expand_vars(trim(list), expanded, sizeof(expanded), NULL, NULL, NULL, NULL);
            char *tok = strtok(expanded, " \t");
            while (tok && nphonies < MAX_PHONIES) {
                phonies[nphonies++] = my_strdup(tok);
                tok = strtok(NULL, " \t");
            }
            current_target = NULL;
            continue;
        }

        /* Variable assignment: check for =, :=, ?=, += */
        {
            char *eq = NULL;
            int assign_type = 0; /* 0=none, 1==, 2=:=, 3=?=, 4=+= */
            char *p = trimmed;

            /* Skip to first = that's part of an assignment */
            while (*p) {
                if (*p == ':' && *(p + 1) == '=') { eq = p; assign_type = 2; break; }
                if (*p == '?' && *(p + 1) == '=') { eq = p; assign_type = 3; break; }
                if (*p == '+' && *(p + 1) == '=') { eq = p; assign_type = 4; break; }
                if (*p == '=' && assign_type == 0) { eq = p; assign_type = 1; break; }
                if (*p == ':') break; /* This is a rule, not an assignment */
                p++;
            }

            if (eq && assign_type > 0) {
                char var_name[MAX_LINE];
                size_t nlen = eq - trimmed;
                if (nlen >= sizeof(var_name)) nlen = sizeof(var_name) - 1;
                memcpy(var_name, trimmed, nlen);
                var_name[nlen] = '\0';
                char *vn = trim(var_name);

                char *val = (assign_type == 1) ? eq + 1 : eq + 2;
                val = trim(val);

                /* Expand value for := (immediate), keep literal for = (deferred) */
                char expanded_val[MAX_EXPANDED];
                if (assign_type == 2) {
                    expand_vars(val, expanded_val, sizeof(expanded_val), NULL, NULL, NULL, NULL);
                    val = expanded_val;
                }

                if (assign_type == 3) {
                    /* ?= only set if not already defined */
                    if (!find_var(vn)) set_var(vn, val, 0);
                } else if (assign_type == 4) {
                    /* += append */
                    char exp_val[MAX_EXPANDED];
                    expand_vars(val, exp_val, sizeof(exp_val), NULL, NULL, NULL, NULL);
                    append_var(vn, exp_val);
                } else {
                    set_var(vn, val, 0);
                }
                current_target = NULL;
                continue;
            }
        }

        /* Rule: target(s): prereqs */
        {
            char *colon = strchr(trimmed, ':');
            if (colon) {
                char target_part[MAX_LINE];
                size_t tlen = colon - trimmed;
                if (tlen >= sizeof(target_part)) tlen = sizeof(target_part) - 1;
                memcpy(target_part, trimmed, tlen);
                target_part[tlen] = '\0';

                char *prereq_part = colon + 1;
                /* Skip double-colon (::) for now — treat as single colon */
                if (*prereq_part == ':') prereq_part++;

                /* Expand variables in both parts */
                char exp_targets[MAX_EXPANDED];
                char exp_prereqs[MAX_EXPANDED];
                expand_vars(trim(target_part), exp_targets, sizeof(exp_targets), NULL, NULL, NULL, NULL);
                expand_vars(trim(prereq_part), exp_prereqs, sizeof(exp_prereqs), NULL, NULL, NULL, NULL);

                /* Check if this is a pattern rule */
                int is_pattern = (strchr(exp_targets, '%') != NULL);

                if (is_pattern && npatterns < MAX_PATTERNS) {
                    Pattern *pat = &patterns[npatterns++];
                    pat->target = my_strdup(trim(exp_targets));
                    /* Pattern prereq (first word) */
                    char prbuf[MAX_EXPANDED];
                    strncpy(prbuf, exp_prereqs, sizeof(prbuf) - 1);
                    prbuf[sizeof(prbuf) - 1] = '\0';
                    char *first = strtok(trim(prbuf), " \t");
                    pat->prereq = first ? my_strdup(first) : my_strdup("");
                    pat->stem = NULL;
                    pat->nrecipes = 0;
                    current_target = NULL;
                    /* Read subsequent recipe lines for this pattern */
                    /* We'll store recipes via a fake target mechanism */
                    /* Actually, we need to read recipes. Let's use a trick:
                       store recipe lines in the pattern directly */
                    {
                        long saved_pos = ftell(f);
                        char rline[MAX_LINE];
                        while (fgets(rline, sizeof(rline), f)) {
                            parse_line_continuation(f, rline, sizeof(rline));
                            if (rline[0] == '\t') {
                                char *recipe = trim(rline + 1);
                                if (*recipe && pat->nrecipes < MAX_RECIPES) {
                                    pat->recipes[pat->nrecipes++] = my_strdup(recipe);
                                }
                            } else {
                                /* Not a recipe line — seek back */
                                fseek(f, saved_pos, SEEK_SET);
                                break;
                            }
                            saved_pos = ftell(f);
                        }
                    }
                    continue;
                }

                /* Explicit rule — may have multiple targets */
                char tgt_buf[MAX_EXPANDED];
                strncpy(tgt_buf, exp_targets, sizeof(tgt_buf) - 1);
                tgt_buf[sizeof(tgt_buf) - 1] = '\0';
                char *tgt_tok = strtok(tgt_buf, " \t");
                Target *first_tgt = NULL;
                while (tgt_tok) {
                    Target *t = add_target(tgt_tok);
                    if (!first_tgt) first_tgt = t;

                    /* Add prerequisites (manual tokenize to avoid nested strtok) */
                    char pr_buf[MAX_EXPANDED];
                    strncpy(pr_buf, exp_prereqs, sizeof(pr_buf) - 1);
                    pr_buf[sizeof(pr_buf) - 1] = '\0';
                    char *pp = pr_buf;
                    while (*pp) {
                        while (*pp == ' ' || *pp == '\t') pp++;
                        if (!*pp) break;
                        char *pr_start = pp;
                        while (*pp && *pp != ' ' && *pp != '\t') pp++;
                        if (*pp) *pp++ = '\0';
                        if (t->nprereqs < MAX_PREREQS) {
                            t->prereqs[t->nprereqs++] = my_strdup(pr_start);
                        }
                    }

                    /* Set first explicit target as default */
                    if (!default_target && tgt_tok[0] != '.') {
                        default_target = t->name;
                    }

                    tgt_tok = strtok(NULL, " \t");
                }
                current_target = first_tgt;
                continue;
            }
        }

        current_target = NULL;
    }

    fclose(f);
}

/* =====================================================================
 * Build engine
 * ===================================================================== */

static int build_target(const char *name);

/* Try to find a matching pattern rule for a target */
static Pattern *find_pattern_rule(const char *name, char **stem_out) {
    for (int i = 0; i < npatterns; i++) {
        char *stem = pattern_match(patterns[i].target, name);
        if (stem) {
            /* Check if the prerequisite would exist */
            char *prereq = pattern_subst(patterns[i].prereq, stem);
            struct stat st;
            int prereq_exists = (stat(prereq, &st) == 0) || find_target(prereq);
            free(prereq);
            if (prereq_exists) {
                *stem_out = stem;
                return &patterns[i];
            }
            free(stem);
        }
    }
    return NULL;
}

/* Build a single target. Returns 0 on success, non-zero on error. */
static int build_target(const char *name) {
    Target *t = find_target(name);
    int phony = is_phony(name);

    /* Check for pattern rule if no explicit target */
    Pattern *pat = NULL;
    char *stem = NULL;
    if (!t || t->nrecipes == 0) {
        pat = find_pattern_rule(name, &stem);
    }

    /* If target doesn't exist and no pattern rule, try implicit .c -> .o */
    if (!t && !pat) {
        /* Check if name ends in .o */
        size_t nlen = strlen(name);
        if (nlen > 2 && name[nlen - 2] == '.' && name[nlen - 1] == 'o') {
            char cfile[MAX_LINE];
            memcpy(cfile, name, nlen - 2);
            cfile[nlen - 2] = '\0';
            strcat(cfile, ".c");
            struct stat st;
            if (stat(cfile, &st) == 0) {
                /* Create an implicit target */
                t = add_target(name);
                t->prereqs[t->nprereqs++] = my_strdup(cfile);
                char recipe[MAX_LINE];
                snprintf(recipe, sizeof(recipe), "$(CC) $(CFLAGS) -c $< -o $@");
                t->recipes[t->nrecipes++] = my_strdup(recipe);
            }
        }
    }

    if (!t && !pat) {
        /* File exists? If so, nothing to build. */
        struct stat st;
        if (stat(name, &st) == 0) return 0;
        fprintf(stderr, "make: *** No rule to make target '%s'. Stop.\n", name);
        return 2;
    }

    /* If we have a pattern match, create a synthetic target */
    if (pat && (!t || t->nrecipes == 0)) {
        if (!t) t = add_target(name);
        char *prereq = pattern_subst(pat->prereq, stem);
        /* Add pattern prereqs if not already there */
        if (t->nprereqs == 0) {
            t->prereqs[t->nprereqs++] = prereq;
        } else {
            free(prereq);
        }
        /* Copy recipes from pattern */
        if (t->nrecipes == 0) {
            for (int i = 0; i < pat->nrecipes && t->nrecipes < MAX_RECIPES; i++) {
                t->recipes[t->nrecipes++] = my_strdup(pat->recipes[i]);
            }
        }
    }

    /* Cycle detection */
    if (t->visited == 1) {
        fprintf(stderr, "make: circular dependency for '%s'\n", name);
        return 2;
    }
    if (t->visited == 2) return 0; /* already built */
    t->visited = 1;

    /* Build all prerequisites first */
    int any_prereq_newer = 0;
    unsigned int target_mtime = phony ? 0 : file_mtime(name);

    for (int i = 0; i < t->nprereqs; i++) {
        int ret = build_target(t->prereqs[i]);
        if (ret != 0) {
            if (!flag_keep_going) return ret;
        }
        unsigned int prereq_mt = file_mtime(t->prereqs[i]);
        if (prereq_mt > target_mtime) {
            any_prereq_newer = 1;
        }
    }

    /* Decide if we need to rebuild */
    int needs_build = 0;
    if (phony) {
        needs_build = 1;
    } else if (flag_always) {
        needs_build = 1;
    } else if (target_mtime == 0 && t->nrecipes > 0) {
        needs_build = 1;  /* target doesn't exist */
    } else if (any_prereq_newer) {
        needs_build = 1;
    }

    if (needs_build && t->nrecipes > 0) {
        /* Build prerequisites string for $^ */
        char all_prereqs[MAX_EXPANDED] = "";
        size_t ap_pos = 0;
        for (int i = 0; i < t->nprereqs; i++) {
            size_t plen = strlen(t->prereqs[i]);
            if (ap_pos + plen + 2 < sizeof(all_prereqs)) {
                if (ap_pos > 0) all_prereqs[ap_pos++] = ' ';
                memcpy(all_prereqs + ap_pos, t->prereqs[i], plen);
                ap_pos += plen;
            }
        }
        all_prereqs[ap_pos] = '\0';

        const char *first_prereq = t->nprereqs > 0 ? t->prereqs[0] : "";

        /* Execute recipes */
        for (int i = 0; i < t->nrecipes; i++) {
            char expanded[MAX_EXPANDED];
            expand_vars(t->recipes[i], expanded, sizeof(expanded),
                       name, first_prereq, all_prereqs, stem);

            const char *cmd = expanded;
            int silent_line = flag_silent;
            int ignore_error = 0;

            /* Check for @ (silent) and - (ignore error) prefixes */
            while (*cmd == '@' || *cmd == '-' || *cmd == ' ') {
                if (*cmd == '@') silent_line = 1;
                else if (*cmd == '-') ignore_error = 1;
                cmd++;
            }

            if (!silent_line) {
                printf("%s\n", cmd);
            }

            if (!flag_dry_run) {
                int ret = system(cmd);
                if (ret != 0 && !ignore_error) {
                    fprintf(stderr, "make: *** [%s] Error %d\n", name, ret);
                    if (!flag_keep_going) { if (stem) free(stem); return 2; }
                }
            }
        }
        t->built = 1;
    }

    t->visited = 2;
    if (stem) free(stem);
    return 0;
}

/* =====================================================================
 * Default variables
 * ===================================================================== */

static void set_default_vars(void) {
    set_var("CC", "cc", 0);
    set_var("AR", "cc -ar", 0);
    set_var("AS", "nasm", 0);
    set_var("CFLAGS", "", 0);
    set_var("LDFLAGS", "", 0);
    set_var("ASFLAGS", "", 0);
    set_var("RM", "rm -f", 0);
    set_var("MAKE", "make", 0);
}

/* =====================================================================
 * Main
 * ===================================================================== */

int main(int argc, char *argv[]) {
    const char *makefile = NULL;
    const char *directory = NULL;
    char *cmd_targets[64];
    int ncmd_targets = 0;

    set_default_vars();

    /* Parse command-line arguments */
    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], "-f") == 0 && i + 1 < argc) {
            makefile = argv[++i];
        } else if (strcmp(argv[i], "-C") == 0 && i + 1 < argc) {
            directory = argv[++i];
        } else if (strcmp(argv[i], "-n") == 0 || strcmp(argv[i], "--dry-run") == 0) {
            flag_dry_run = 1;
        } else if (strcmp(argv[i], "-s") == 0 || strcmp(argv[i], "--silent") == 0) {
            flag_silent = 1;
        } else if (strcmp(argv[i], "-B") == 0 || strcmp(argv[i], "--always-make") == 0) {
            flag_always = 1;
        } else if (strcmp(argv[i], "-k") == 0 || strcmp(argv[i], "--keep-going") == 0) {
            flag_keep_going = 1;
        } else if (strcmp(argv[i], "--version") == 0) {
            printf("anyOS make 1.0\n");
            return 0;
        } else if (strcmp(argv[i], "--help") == 0 || strcmp(argv[i], "-h") == 0) {
            printf("Usage: make [options] [target...] [VAR=value...]\n");
            printf("Options:\n");
            printf("  -f FILE   Read FILE as a makefile\n");
            printf("  -C DIR    Change to DIR before reading makefile\n");
            printf("  -n        Dry run (print commands without executing)\n");
            printf("  -s        Silent (don't print commands)\n");
            printf("  -B        Unconditionally build all targets\n");
            printf("  -k        Keep going after errors\n");
            printf("  -h        Show this help\n");
            return 0;
        } else if (strchr(argv[i], '=')) {
            /* VAR=value on command line */
            char *eq = strchr(argv[i], '=');
            char var_name[MAX_LINE];
            size_t nlen = eq - argv[i];
            if (nlen >= sizeof(var_name)) nlen = sizeof(var_name) - 1;
            memcpy(var_name, argv[i], nlen);
            var_name[nlen] = '\0';
            set_var(var_name, eq + 1, 1); /* override=1 */
        } else {
            if (ncmd_targets < 64) {
                cmd_targets[ncmd_targets++] = argv[i];
            }
        }
    }

    /* Change directory if -C specified */
    if (directory) {
        if (chdir(directory) != 0) {
            fprintf(stderr, "make: *** chdir: %s: No such directory\n", directory);
            return 2;
        }
    }

    /* Find and parse makefile */
    if (!makefile) {
        if (access("Makefile", F_OK) == 0) makefile = "Makefile";
        else if (access("makefile", F_OK) == 0) makefile = "makefile";
        else if (access("GNUmakefile", F_OK) == 0) makefile = "GNUmakefile";
        else {
            fprintf(stderr, "make: *** No makefile found. Stop.\n");
            return 2;
        }
    }

    parse_makefile(makefile, 0);

    /* Build targets */
    if (ncmd_targets == 0) {
        if (!default_target) {
            fprintf(stderr, "make: *** No targets. Stop.\n");
            return 2;
        }
        return build_target(default_target);
    }

    int ret = 0;
    for (int i = 0; i < ncmd_targets; i++) {
        int r = build_target(cmd_targets[i]);
        if (r != 0) ret = r;
        if (ret != 0 && !flag_keep_going) break;
    }
    return ret;
}
