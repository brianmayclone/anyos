/*
 * parser.c — CMake parser for amake
 *
 * Parses a token stream into a linked list of AstNode.
 * Handles command(), if/elseif/else/endif, foreach/endforeach,
 * and function/endfunction blocks.
 */
#include "amake.h"

/* ── Parser state ────────────────────────────────────────────────────── */

typedef struct {
    const Token *tokens;
    int          count;
    int          pos;
} ParseCtx;

static Token peek(ParseCtx *pc) {
    if (pc->pos < pc->count)
        return pc->tokens[pc->pos];
    return pc->tokens[pc->count - 1]; /* EOF */
}

static Token advance(ParseCtx *pc) {
    Token t = peek(pc);
    if (pc->pos < pc->count)
        pc->pos++;
    return t;
}

static void skip_newlines(ParseCtx *pc) {
    while (pc->pos < pc->count && pc->tokens[pc->pos].type == TOK_NEWLINE)
        pc->pos++;
}

static int at_eof(ParseCtx *pc) {
    return pc->pos >= pc->count || peek(pc).type == TOK_EOF;
}

/* ── AST node allocation ─────────────────────────────────────────────── */

static AstNode *node_new(AstType type, int line) {
    AstNode *n = amake_malloc(sizeof(AstNode));
    memset(n, 0, sizeof(AstNode));
    n->type = type;
    n->line = line;
    return n;
}

/* ── Parse arguments inside parentheses ──────────────────────────────── */

/*
 * Parse the content between ( and ), collecting WORD tokens into args.
 * Returns argument count. Handles nested parentheses (for generator expressions).
 */
static int parse_args(ParseCtx *pc, char ***out_args) {
    int cap = 16;
    char **args = amake_malloc(sizeof(char *) * cap);
    int argc = 0;
    int depth = 0;

    while (!at_eof(pc)) {
        Token t = peek(pc);
        if (t.type == TOK_RPAREN && depth == 0)
            break;
        if (t.type == TOK_RPAREN) {
            depth--;
            /* Include nested ) as part of the current word — rare in CMake */
        }
        if (t.type == TOK_LPAREN) {
            depth++;
        }
        if (t.type == TOK_NEWLINE) {
            advance(pc);
            continue;
        }
        if (t.type == TOK_WORD) {
            if (argc >= cap) {
                cap *= 2;
                args = amake_realloc(args, sizeof(char *) * cap);
            }
            args[argc++] = amake_strdup(t.text);
            advance(pc);
            continue;
        }
        /* Skip other tokens (LPAREN inside args is unusual but handle it) */
        advance(pc);
    }

    *out_args = args;
    return argc;
}

/* ── Forward declarations ────────────────────────────────────────────── */

static AstNode *parse_block(ParseCtx *pc, const char *end_cmd, int *out_count);
static AstNode *parse_if_block(ParseCtx *pc, const char *cmd,
                                char **cond_args, int cond_argc, int line);
static AstNode *parse_foreach_block(ParseCtx *pc, char **args, int argc, int line);
static AstNode *parse_function_block(ParseCtx *pc, const char *kw,
                                      char **args, int argc, int line);

/* ── Parse a single command ──────────────────────────────────────────── */

/*
 * Parse one command: NAME ( args... )
 * Returns NULL on EOF or if the command name matches end_cmd.
 */
static AstNode *parse_command(ParseCtx *pc, const char *end_cmd) {
    skip_newlines(pc);
    if (at_eof(pc)) return NULL;

    Token name_tok = peek(pc);
    if (name_tok.type != TOK_WORD)  {
        advance(pc); /* skip unexpected token */
        return NULL;
    }

    /* Check for block terminators */
    if (end_cmd && name_tok.text) {
        /* Check against end_cmd and related terminators */
        if (strcasecmp(name_tok.text, end_cmd) == 0)
            return NULL;
        /* Also check for elseif/else when parsing if-block body */
        if (strcasecmp(end_cmd, "endif") == 0 || strcasecmp(end_cmd, "else") == 0) {
            if (strcasecmp(name_tok.text, "else") == 0 ||
                strcasecmp(name_tok.text, "elseif") == 0 ||
                strcasecmp(name_tok.text, "endif") == 0)
                return NULL;
        }
    }

    advance(pc); /* consume command name */

    /* Expect ( */
    skip_newlines(pc);
    if (peek(pc).type != TOK_LPAREN) {
        /* Malformed — skip */
        return NULL;
    }
    advance(pc); /* consume ( */

    /* Parse arguments */
    char **args = NULL;
    int argc = parse_args(pc, &args);

    /* Expect ) */
    if (peek(pc).type == TOK_RPAREN)
        advance(pc);

    /* Handle structured blocks */
    if (strcasecmp(name_tok.text, "if") == 0) {
        return parse_if_block(pc, name_tok.text, args, argc, name_tok.line);
    }
    if (strcasecmp(name_tok.text, "foreach") == 0) {
        return parse_foreach_block(pc, args, argc, name_tok.line);
    }
    if (strcasecmp(name_tok.text, "function") == 0 ||
        strcasecmp(name_tok.text, "macro") == 0) {
        return parse_function_block(pc, name_tok.text, args, argc, name_tok.line);
    }

    /* Plain command */
    AstNode *n = node_new(AST_COMMAND, name_tok.line);
    n->cmd_name = amake_strdup(name_tok.text);
    n->args = args;
    n->argc = argc;
    return n;
}

/* ── Parse if/elseif/else/endif block ────────────────────────────────── */

static AstNode *parse_if_block(ParseCtx *pc, const char *cmd,
                                char **cond_args, int cond_argc, int line)
{
    AstNode *node = node_new(AST_IF_BLOCK, line);
    node->cond_args = cond_args;
    node->cond_argc = cond_argc;

    /* Parse if body — stops at elseif, else, or endif */
    int body_count = 0;
    AstNode *body = parse_block(pc, "endif", &body_count);
    node->if_body = body;
    node->if_body_count = body_count;

    /* Check what stopped us */
    skip_newlines(pc);
    Token t = peek(pc);

    if (t.type == TOK_WORD && strcasecmp(t.text, "elseif") == 0) {
        advance(pc); /* consume "elseif" */
        skip_newlines(pc);
        if (peek(pc).type == TOK_LPAREN) advance(pc);
        char **else_args = NULL;
        int else_argc = parse_args(pc, &else_args);
        if (peek(pc).type == TOK_RPAREN) advance(pc);
        /* Recursively parse the elseif as another if block */
        node->else_chain = parse_if_block(pc, "elseif", else_args, else_argc, t.line);
    }
    else if (t.type == TOK_WORD && strcasecmp(t.text, "else") == 0) {
        advance(pc); /* consume "else" */
        skip_newlines(pc);
        if (peek(pc).type == TOK_LPAREN) advance(pc);
        /* else() has no args */
        char **dummy = NULL;
        int dc = parse_args(pc, &dummy);
        if (peek(pc).type == TOK_RPAREN) advance(pc);
        int i;
        for (i = 0; i < dc; i++) free(dummy[i]);
        free(dummy);

        /* Parse else body */
        int else_count = 0;
        AstNode *else_body = parse_block(pc, "endif", &else_count);

        /* Wrap in an AST_IF_BLOCK with always-true condition (no cond_args) */
        AstNode *else_node = node_new(AST_IF_BLOCK, t.line);
        else_node->cond_args = NULL;
        else_node->cond_argc = 0;  /* 0 args = always true (else clause) */
        else_node->if_body = else_body;
        else_node->if_body_count = else_count;
        node->else_chain = else_node;

        /* Now consume the endif */
        skip_newlines(pc);
        t = peek(pc);
        if (t.type == TOK_WORD && strcasecmp(t.text, "endif") == 0) {
            advance(pc);
            skip_newlines(pc);
            if (peek(pc).type == TOK_LPAREN) advance(pc);
            char **d2 = NULL;
            int d2c = parse_args(pc, &d2);
            if (peek(pc).type == TOK_RPAREN) advance(pc);
            for (i = 0; i < d2c; i++) free(d2[i]);
            free(d2);
        }
    }
    else if (t.type == TOK_WORD && strcasecmp(t.text, "endif") == 0) {
        advance(pc); /* consume "endif" */
        skip_newlines(pc);
        if (peek(pc).type == TOK_LPAREN) advance(pc);
        char **d2 = NULL;
        int d2c = parse_args(pc, &d2);
        if (peek(pc).type == TOK_RPAREN) advance(pc);
        int i;
        for (i = 0; i < d2c; i++) free(d2[i]);
        free(d2);
    }

    return node;
}

/* ── Parse foreach/endforeach block ──────────────────────────────────── */

static AstNode *parse_foreach_block(ParseCtx *pc, char **args, int argc, int line) {
    AstNode *node = node_new(AST_FOREACH, line);

    if (argc < 1)
        amake_fatal("line %d: foreach() requires at least a loop variable", line);

    node->loop_var = amake_strdup(args[0]);
    /* Remaining args are the values to iterate */
    node->loop_value_count = argc - 1;
    node->loop_values = amake_malloc(sizeof(char *) * (argc - 1 + 1));
    int i;
    for (i = 1; i < argc; i++)
        node->loop_values[i - 1] = amake_strdup(args[i]);

    /* Free the original args since we copied what we need */
    for (i = 0; i < argc; i++) free(args[i]);
    free(args);

    /* Parse body */
    int body_count = 0;
    node->loop_body = parse_block(pc, "endforeach", &body_count);
    node->loop_body_count = body_count;

    /* Consume endforeach() */
    skip_newlines(pc);
    Token t = peek(pc);
    if (t.type == TOK_WORD && strcasecmp(t.text, "endforeach") == 0) {
        advance(pc);
        skip_newlines(pc);
        if (peek(pc).type == TOK_LPAREN) advance(pc);
        char **d = NULL;
        int dc = parse_args(pc, &d);
        if (peek(pc).type == TOK_RPAREN) advance(pc);
        for (i = 0; i < dc; i++) free(d[i]);
        free(d);
    }

    return node;
}

/* ── Parse function/endfunction block ────────────────────────────────── */

static AstNode *parse_function_block(ParseCtx *pc, const char *kw,
                                      char **args, int argc, int line)
{
    int is_macro = (strcasecmp(kw, "macro") == 0);
    const char *end_kw = is_macro ? "endmacro" : "endfunction";

    AstNode *node = node_new(AST_FUNCTION_DEF, line);

    if (argc < 1)
        amake_fatal("line %d: %s() requires a name", line, kw);

    node->func_name = amake_strdup(args[0]);
    node->func_param_count = argc - 1;
    node->func_params = amake_malloc(sizeof(char *) * (argc - 1 + 1));
    int i;
    for (i = 1; i < argc; i++)
        node->func_params[i - 1] = amake_strdup(args[i]);

    for (i = 0; i < argc; i++) free(args[i]);
    free(args);

    /* Parse body */
    int body_count = 0;
    node->func_body = parse_block(pc, end_kw, &body_count);
    node->func_body_count = body_count;

    /* Consume endfunction/endmacro */
    skip_newlines(pc);
    Token t = peek(pc);
    if (t.type == TOK_WORD && strcasecmp(t.text, end_kw) == 0) {
        advance(pc);
        skip_newlines(pc);
        if (peek(pc).type == TOK_LPAREN) advance(pc);
        char **d = NULL;
        int dc = parse_args(pc, &d);
        if (peek(pc).type == TOK_RPAREN) advance(pc);
        for (i = 0; i < dc; i++) free(d[i]);
        free(d);
    }

    return node;
}

/* ── Parse a block of commands ───────────────────────────────────────── */

/*
 * Parse commands until EOF or until a command matching end_cmd is found.
 * Returns a linked list of AstNode and sets *out_count.
 */
static AstNode *parse_block(ParseCtx *pc, const char *end_cmd, int *out_count) {
    AstNode *head = NULL;
    AstNode *tail = NULL;
    int count = 0;

    while (!at_eof(pc)) {
        skip_newlines(pc);
        if (at_eof(pc)) break;

        /* Peek to see if we hit the terminator */
        Token t = peek(pc);
        if (end_cmd && t.type == TOK_WORD) {
            if (strcasecmp(t.text, end_cmd) == 0)
                break;
            /* For if blocks, also stop at elseif/else */
            if (strcasecmp(end_cmd, "endif") == 0) {
                if (strcasecmp(t.text, "elseif") == 0 ||
                    strcasecmp(t.text, "else") == 0)
                    break;
            }
        }

        AstNode *n = parse_command(pc, end_cmd);
        if (!n) break;

        if (!head) head = n;
        else tail->next = n;
        tail = n;
        count++;
    }

    if (out_count) *out_count = count;
    return head;
}

/* ── Public API ──────────────────────────────────────────────────────── */

AstNode *parser_parse(const TokenList *tl) {
    ParseCtx pc;
    pc.tokens = tl->tokens;
    pc.count = tl->count;
    pc.pos = 0;

    int count = 0;
    return parse_block(&pc, NULL, &count);
}

/* ── AST cleanup ─────────────────────────────────────────────────────── */

static void free_string_array(char **arr, int count) {
    int i;
    if (!arr) return;
    for (i = 0; i < count; i++)
        free(arr[i]);
    free(arr);
}

void ast_free(AstNode *list) {
    while (list) {
        AstNode *next = list->next;

        free(list->cmd_name);
        free_string_array(list->args, list->argc);
        free_string_array(list->cond_args, list->cond_argc);

        ast_free(list->if_body);
        ast_free(list->else_chain);

        free(list->loop_var);
        free_string_array(list->loop_values, list->loop_value_count);
        ast_free(list->loop_body);

        free(list->func_name);
        free_string_array(list->func_params, list->func_param_count);
        ast_free(list->func_body);

        free(list);
        list = next;
    }
}
