/*
 * amake.h — anyOS build system (CMake + Ninja replacement)
 *
 * Parses CMakeLists.txt (the subset used by anyOS), builds a dependency graph,
 * and executes builds in parallel with mtime-based dirty detection.
 *
 * Written in C99 for TCC compatibility (self-hosting on anyOS).
 */
#ifndef AMAKE_H
#define AMAKE_H

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <stdarg.h>
#include <sys/stat.h>
#include <time.h>

/* ── Constants ────────────────────────────────────────────────────────── */

#define AMAKE_VERSION "0.1.0"
#define MAX_ARGS      512
#define MAX_PATH_LEN  4096
#define HASH_BUCKETS  256
#define MAX_COMMANDS   64
#define MAX_OUTPUTS    32
#define MAX_DEPENDS   256

/* ── Tokens (lexer.c) ────────────────────────────────────────────────── */

typedef enum {
    TOK_WORD,
    TOK_LPAREN,
    TOK_RPAREN,
    TOK_NEWLINE,
    TOK_EOF
} TokenType;

typedef struct {
    TokenType type;
    char     *text;     /* heap-allocated for TOK_WORD */
    int       line;
} Token;

typedef struct {
    Token *tokens;
    int    count;
    int    cap;
} TokenList;

/* ── AST (parser.c) ──────────────────────────────────────────────────── */

typedef enum {
    AST_COMMAND,
    AST_IF_BLOCK,
    AST_FOREACH,
    AST_FUNCTION_DEF
} AstType;

typedef struct AstNode AstNode;
struct AstNode {
    AstType  type;
    int      line;

    /* AST_COMMAND */
    char    *cmd_name;
    char   **args;
    int      argc;

    /* AST_IF_BLOCK: condition + body + else chain */
    char   **cond_args;
    int      cond_argc;
    AstNode *if_body;
    int      if_body_count;
    AstNode *else_chain;    /* linked list of elseif/else blocks (each is AST_IF_BLOCK) */

    /* AST_FOREACH */
    char    *loop_var;
    char   **loop_values;
    int      loop_value_count;
    AstNode *loop_body;
    int      loop_body_count;

    /* AST_FUNCTION_DEF */
    char    *func_name;
    char   **func_params;
    int      func_param_count;
    AstNode *func_body;
    int      func_body_count;

    /* Linked list (sibling) */
    AstNode *next;
};

/* ── Variables (vars.c) ──────────────────────────────────────────────── */

typedef struct VarEntry {
    char            *name;
    char            *value;     /* semicolon-separated list */
    struct VarEntry *next;      /* hash chain */
} VarEntry;

typedef struct VarScope {
    VarEntry       *buckets[HASH_BUCKETS];
    struct VarScope *parent;
} VarScope;

/* ── Build Graph (graph.c) ───────────────────────────────────────────── */

typedef struct BuildRule BuildRule;
struct BuildRule {
    char   **outputs;       int output_count;
    char   **commands;      int command_count;
    char   **depends;       int depend_count;
    char    *comment;
    char    *working_dir;

    /* State */
    int      dirty;
    int      building;
    int      done;
    int      failed;

    /* Graph edges */
    BuildRule **blockers;   int blocker_count;  int blocker_cap;
    BuildRule **blocked;    int blocked_count;   int blocked_cap;
    int      unresolved;    /* blockers not yet done */
};

typedef struct {
    char    *name;
    char   **depends;       int depend_count;
    char   **commands;      int command_count;
    char    *comment;
    int      is_default;    /* ALL */
    int      uses_terminal;
} BuildTarget;

typedef struct {
    BuildRule   **rules;    int rule_count;   int rule_cap;
    BuildTarget **targets;  int target_count; int target_cap;
} BuildGraph;

/* ── Mtime Cache (track.c) ──────────────────────────────────────────── */

typedef struct MtimeEntry {
    char               *path;
    time_t              mtime;   /* 0 = file does not exist */
    int                 valid;
    struct MtimeEntry  *next;
} MtimeEntry;

typedef struct {
    MtimeEntry *buckets[HASH_BUCKETS];
} MtimeCache;

/* ── Executor (exec.c) ──────────────────────────────────────────────── */

#ifndef _WIN32
#include <unistd.h>
#include <sys/types.h>
#include <sys/wait.h>
#endif

typedef struct {
    int        pid;         /* pid_t on Unix */
    BuildRule *rule;
    int        cmd_index;
} RunningJob;

typedef struct {
    int          max_jobs;
    RunningJob  *jobs;
    int          job_count;
    int          job_cap;
    BuildRule  **ready;
    int          ready_count;
    int          ready_cap;
    int          failed_count;
    int          built_count;
    int          total_dirty;
    int          verbose;
} Executor;

/* ── User-defined function ───────────────────────────────────────────── */

typedef struct {
    char     *name;
    char    **params;
    int       param_count;
    AstNode  *body;
    int       body_count;
} FuncDef;

/* ── Top-level context ───────────────────────────────────────────────── */

typedef struct {
    /* Paths */
    char       *source_dir;
    char       *binary_dir;
    char       *amake_path;
    char       *cmake_file;

    /* Variables */
    VarScope   *global_scope;
    VarScope   *current_scope;

    /* User-defined functions */
    FuncDef   **functions;
    int         func_count;
    int         func_cap;

    /* Build graph */
    BuildGraph  graph;

    /* CLI overrides (-D) */
    char      **cli_defines;    /* "VAR=VAL" strings */
    int         cli_define_count;

    /* Options */
    int         verbose;
    int         max_jobs;
    int         clean;
    char      **targets;        /* requested target names */
    int         target_count;
} AmakeCtx;

/* ── Utility (amake.c) ───────────────────────────────────────────────── */

void  amake_fatal(const char *fmt, ...);
char *amake_strdup(const char *s);
char *amake_strndup(const char *s, size_t n);
char *amake_sprintf(const char *fmt, ...);
void *amake_malloc(size_t n);
void *amake_realloc(void *p, size_t n);
char *amake_path_join(const char *a, const char *b);
void  amake_mkdir_p(const char *path);
int   amake_file_exists(const char *path);
int   amake_is_directory(const char *path);
char *amake_read_file(const char *path, size_t *out_size);
int   amake_copy_file(const char *src, const char *dst);
int   amake_copy_directory(const char *src, const char *dst);
int   amake_rm_rf(const char *path);
int   amake_touch(const char *path);

/* ── Built-in -E command handler ─────────────────────────────────────── */

int   amake_builtin_E(int argc, char **argv);

/* ── Lexer (lexer.c) ─────────────────────────────────────────────────── */

void  lexer_tokenize(const char *source, size_t len, TokenList *out);
void  token_list_free(TokenList *tl);

/* ── Parser (parser.c) ───────────────────────────────────────────────── */

AstNode *parser_parse(const TokenList *tl);
void     ast_free(AstNode *list);

/* ── Variables (vars.c) ──────────────────────────────────────────────── */

VarScope *scope_new(VarScope *parent);
void      scope_free(VarScope *scope);
void      scope_set(VarScope *scope, const char *name, const char *value);
const char *scope_get(VarScope *scope, const char *name);
char     *expand_vars(AmakeCtx *ctx, const char *input);
void      expand_args(AmakeCtx *ctx, char **args, int argc,
                      char ***out_args, int *out_argc);

/* ── Glob (glob.c) ───────────────────────────────────────────────────── */

void  amake_glob(const char *pattern, char ***out_files, int *out_count);
void  amake_glob_recurse(const char *base_dir, const char *pattern,
                         char ***out_files, int *out_count);

/* ── Track (track.c) ─────────────────────────────────────────────────── */

void    mtime_cache_init(MtimeCache *mc);
void    mtime_cache_free(MtimeCache *mc);
time_t  mtime_get(MtimeCache *mc, const char *path);

/* ── Eval (eval.c) ───────────────────────────────────────────────────── */

void  eval_run(AmakeCtx *ctx, AstNode *nodes);

/* ── Graph (graph.c) ─────────────────────────────────────────────────── */

void       graph_init(BuildGraph *g);
BuildRule *graph_add_rule(BuildGraph *g);
BuildTarget *graph_add_target(BuildGraph *g);
BuildRule *graph_find_rule_for_output(BuildGraph *g, const char *path);
void       graph_link(BuildGraph *g);
void       graph_mark_dirty(BuildGraph *g, MtimeCache *mc);
int        graph_collect_dirty_for_target(BuildGraph *g, const char *target_name,
                                          BuildRule ***out_dirty, int *out_count);
int        graph_collect_dirty_all(BuildGraph *g,
                                   BuildRule ***out_dirty, int *out_count);
void       graph_free(BuildGraph *g);

/* ── Executor (exec.c) ──────────────────────────────────────────────── */

void  exec_init(Executor *ex, int max_jobs, int verbose);
int   exec_run(Executor *ex, BuildRule **dirty, int dirty_count);
void  exec_free(Executor *ex);

#endif /* AMAKE_H */
