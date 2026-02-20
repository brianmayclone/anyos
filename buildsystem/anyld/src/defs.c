/*
 * defs.c — Parse .def symbol definition files.
 *
 * Format:
 *   # comment
 *   LIBRARY <name>
 *   EXPORTS
 *     symbol_name_1
 *     symbol_name_2
 */
#include "anyld.h"

int parse_def_file(Ctx *ctx, const char *path) {
    size_t size;
    uint8_t *raw = read_file(path, &size);
    if (!raw) return -1;

    char *text = (char *)raw;
    int in_exports = 0;
    size_t pos = 0;

    while (pos < size) {
        /* Find line boundaries */
        size_t line_start = pos;
        while (pos < size && text[pos] != '\n' && text[pos] != '\r')
            pos++;

        size_t line_end = pos;

        /* Skip line endings */
        if (pos < size && text[pos] == '\r') pos++;
        if (pos < size && text[pos] == '\n') pos++;

        /* Trim leading whitespace */
        while (line_start < line_end &&
               (text[line_start] == ' ' || text[line_start] == '\t'))
            line_start++;

        /* Skip empty lines and comments */
        if (line_start >= line_end) continue;
        if (text[line_start] == '#') continue;

        /* NUL-terminate this line for easy parsing */
        char saved = text[line_end];
        text[line_end] = '\0';
        char *line = text + line_start;

        /* Trim trailing whitespace */
        size_t len = strlen(line);
        while (len > 0 && (line[len - 1] == ' ' || line[len - 1] == '\t'))
            line[--len] = '\0';

        if (len == 0) {
            text[line_end] = saved;
            continue;
        }

        /* Parse directives */
        if (strncmp(line, "LIBRARY", 7) == 0 &&
            (line[7] == ' ' || line[7] == '\t')) {
            char *name = line + 8;
            while (*name == ' ' || *name == '\t') name++;
            if (*name) {
                free(ctx->lib_name);
                ctx->lib_name = strdup(name);
            }
        } else if (strcmp(line, "EXPORTS") == 0) {
            in_exports = 1;
        } else if (in_exports) {
            /* Each line is a symbol name to export */
            if (ctx->nexports >= ctx->exports_cap) {
                ctx->exports_cap = ctx->exports_cap ? ctx->exports_cap * 2 : 256;
                ctx->exports = realloc(ctx->exports,
                                       ctx->exports_cap * sizeof(char *));
            }
            ctx->exports[ctx->nexports++] = strdup(line);
        }

        text[line_end] = saved;
    }

    free(raw);
    return 0;
}

void mark_exports(Ctx *ctx) {
    for (int i = 0; i < ctx->nsyms; i++) {
        ctx->syms[i].is_export = 0;
    }
    for (int e = 0; e < ctx->nexports; e++) {
        int found = find_global_sym(ctx, ctx->exports[e]);
        if (found >= 0) {
            ctx->syms[found].is_export = 1;
        } else {
            fprintf(stderr, "anyld: warning: export symbol '%s' not found\n",
                    ctx->exports[e]);
        }
    }
}
