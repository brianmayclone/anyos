/*
 * lexer.c — CMake tokenizer for amake
 *
 * Tokenizes CMakeLists.txt into WORD, LPAREN, RPAREN, NEWLINE, EOF.
 * Handles comments, quoted strings, bracket strings, and line continuations.
 */
#include "amake.h"

/* ── Token list helpers ──────────────────────────────────────────────── */

static void tl_push(TokenList *tl, TokenType type, const char *text, int line) {
    if (tl->count >= tl->cap) {
        tl->cap = tl->cap ? tl->cap * 2 : 256;
        tl->tokens = amake_realloc(tl->tokens, sizeof(Token) * tl->cap);
    }
    tl->tokens[tl->count].type = type;
    tl->tokens[tl->count].text = text ? amake_strdup(text) : NULL;
    tl->tokens[tl->count].line = line;
    tl->count++;
}

static void tl_push_n(TokenList *tl, TokenType type, const char *text,
                       size_t len, int line) {
    if (tl->count >= tl->cap) {
        tl->cap = tl->cap ? tl->cap * 2 : 256;
        tl->tokens = amake_realloc(tl->tokens, sizeof(Token) * tl->cap);
    }
    tl->tokens[tl->count].type = type;
    tl->tokens[tl->count].text = amake_strndup(text, len);
    tl->tokens[tl->count].line = line;
    tl->count++;
}

/* ── Character tests ─────────────────────────────────────────────────── */

static int is_space(char c) {
    return c == ' ' || c == '\t' || c == '\r';
}

static int is_word_char(char c) {
    return c && c != '(' && c != ')' && c != '#' && c != '"'
        && c != '\n' && c != '\r' && c != ' ' && c != '\t';
}

/* ── Tokenizer ───────────────────────────────────────────────────────── */

void lexer_tokenize(const char *src, size_t len, TokenList *out) {
    const char *p = src;
    const char *end = src + len;
    int line = 1;

    out->tokens = NULL;
    out->count = 0;
    out->cap = 0;

    while (p < end) {
        /* Skip horizontal whitespace */
        while (p < end && is_space(*p))
            p++;

        if (p >= end)
            break;

        /* Newline */
        if (*p == '\n') {
            tl_push(out, TOK_NEWLINE, NULL, line);
            line++;
            p++;
            continue;
        }

        /* Comment: # to end of line */
        if (*p == '#') {
            /* Check for bracket comment #[[ ... ]] */
            if (p + 1 < end && p[1] == '[') {
                const char *q = p + 2;
                /* Count = signs: #[==[ ... ]==] */
                int eq = 0;
                while (q < end && *q == '=') { eq++; q++; }
                if (q < end && *q == '[') {
                    /* Bracket comment — find matching ]==] */
                    q++;
                    while (q < end) {
                        if (*q == ']') {
                            const char *t = q + 1;
                            int eq2 = 0;
                            while (t < end && *t == '=') { eq2++; t++; }
                            if (eq2 == eq && t < end && *t == ']') {
                                /* Count newlines inside */
                                const char *nl;
                                for (nl = p; nl <= t; nl++)
                                    if (*nl == '\n') line++;
                                p = t + 1;
                                goto next_token;
                            }
                        }
                        if (*q == '\n') line++;
                        q++;
                    }
                    /* Unterminated bracket comment — skip to end */
                    p = end;
                    continue;
                }
            }
            /* Regular line comment */
            while (p < end && *p != '\n')
                p++;
            continue;
        }

        /* Parentheses */
        if (*p == '(') {
            tl_push(out, TOK_LPAREN, NULL, line);
            p++;
            continue;
        }
        if (*p == ')') {
            tl_push(out, TOK_RPAREN, NULL, line);
            p++;
            continue;
        }

        /* Quoted string: "..." — prefix with \x01 so expand_args preserves semicolons */
        if (*p == '"') {
            p++; /* skip opening quote */
            size_t cap = 256;
            char *buf = amake_malloc(cap);
            size_t blen = 0;
            buf[blen++] = '\x01'; /* quoted marker */
            while (p < end && *p != '"') {
                if (*p == '\\' && p + 1 < end) {
                    char esc = p[1];
                    if (esc == '"' || esc == '\\' || esc == '$') {
                        if (blen + 1 >= cap) { cap *= 2; buf = amake_realloc(buf, cap); }
                        buf[blen++] = esc;
                        p += 2;
                        continue;
                    }
                    if (esc == 'n') {
                        if (blen + 1 >= cap) { cap *= 2; buf = amake_realloc(buf, cap); }
                        buf[blen++] = '\n';
                        p += 2;
                        continue;
                    }
                    if (esc == 't') {
                        if (blen + 1 >= cap) { cap *= 2; buf = amake_realloc(buf, cap); }
                        buf[blen++] = '\t';
                        p += 2;
                        continue;
                    }
                    /* Unknown escape — keep both characters */
                }
                if (*p == '\n') line++;
                if (blen + 1 >= cap) { cap *= 2; buf = amake_realloc(buf, cap); }
                buf[blen++] = *p++;
            }
            if (p < end) p++; /* skip closing quote */
            buf[blen] = '\0';
            tl_push(out, TOK_WORD, buf, line);
            free(buf);
            continue;
        }

        /* Bracket string: [[ ... ]] or [=[ ... ]=] */
        if (*p == '[') {
            const char *q = p + 1;
            int eq = 0;
            while (q < end && *q == '=') { eq++; q++; }
            if (q < end && *q == '[') {
                q++; /* skip inner [ */
                const char *content_start = q;
                while (q < end) {
                    if (*q == ']') {
                        const char *t = q + 1;
                        int eq2 = 0;
                        while (t < end && *t == '=') { eq2++; t++; }
                        if (eq2 == eq && t < end && *t == ']') {
                            tl_push_n(out, TOK_WORD, content_start,
                                      (size_t)(q - content_start), line);
                            /* Count newlines */
                            const char *nl;
                            for (nl = p; nl <= t; nl++)
                                if (*nl == '\n') line++;
                            p = t + 1;
                            goto next_token;
                        }
                    }
                    q++;
                }
                /* Unterminated — treat [ as word start and fall through */
            }
        }

        /* Line continuation: backslash + newline */
        if (*p == '\\' && p + 1 < end && p[1] == '\n') {
            p += 2;
            line++;
            continue;
        }

        /* Unquoted word */
        if (is_word_char(*p)) {
            const char *start = p;
            while (p < end && is_word_char(*p)) {
                /* Handle line continuation inside words */
                if (*p == '\\' && p + 1 < end && p[1] == '\n') {
                    tl_push_n(out, TOK_WORD, start, (size_t)(p - start), line);
                    p += 2;
                    line++;
                    start = p;
                    continue;
                }
                p++;
            }
            if (p > start)
                tl_push_n(out, TOK_WORD, start, (size_t)(p - start), line);
            continue;
        }

        /* Unknown character — skip */
        p++;
next_token:;
    }

    /* EOF token */
    tl_push(out, TOK_EOF, NULL, line);
}

void token_list_free(TokenList *tl) {
    int i;
    if (!tl) return;
    for (i = 0; i < tl->count; i++)
        free(tl->tokens[i].text);
    free(tl->tokens);
    tl->tokens = NULL;
    tl->count = 0;
    tl->cap = 0;
}
