/*
 * anyld — anyOS ELF64 Shared Object Linker
 *
 * Takes ELF64 relocatable objects (.o) or static libraries (.a) and
 * produces an ELF64 shared object (ET_DYN) with a proper .dynsym
 * symbol table.
 *
 * Written in C99 for TCC compatibility (self-hosting on anyOS).
 *
 * Usage:
 *   anyld -o output.so -b 0x04000000 -e exports.def input.a [input.o ...]
 */
#include "anyld.h"

#ifdef ONE_SOURCE
/* Single-source compilation mode (for TCC on anyOS) */
#include "defs.c"
#include "input.c"
#include "link.c"
#include "output.c"
#endif

/* ── Utility: fatal error ───────────────────────────────────────────── */

void fatal(const char *fmt, ...) {
    va_list ap;
    fprintf(stderr, "anyld: fatal: ");
    va_start(ap, fmt);
    vfprintf(stderr, fmt, ap);
    va_end(ap);
    fprintf(stderr, "\n");
    exit(1);
}

/* ── Utility: read entire file into malloc'd buffer ─────────────────── */

uint8_t *read_file(const char *path, size_t *out_size) {
    FILE *fp = fopen(path, "rb");
    if (!fp) {
        fprintf(stderr, "anyld: cannot open '%s'\n", path);
        return NULL;
    }

    fseek(fp, 0, SEEK_END);
    long sz = ftell(fp);
    if (sz < 0) {
        fclose(fp);
        fprintf(stderr, "anyld: cannot determine size of '%s'\n", path);
        return NULL;
    }
    fseek(fp, 0, SEEK_SET);

    uint8_t *buf = malloc((size_t)sz);
    if (!buf) {
        fclose(fp);
        fprintf(stderr, "anyld: out of memory reading '%s'\n", path);
        return NULL;
    }

    size_t n = fread(buf, 1, (size_t)sz, fp);
    fclose(fp);

    if (n != (size_t)sz) {
        fprintf(stderr, "anyld: short read on '%s'\n", path);
        free(buf);
        return NULL;
    }

    *out_size = (size_t)sz;
    return buf;
}

/* ── Buffer operations ──────────────────────────────────────────────── */

void buf_init(Buf *b) {
    b->data = NULL;
    b->size = 0;
    b->cap  = 0;
}

static void buf_grow(Buf *b, size_t needed) {
    if (b->size + needed <= b->cap) return;
    size_t new_cap = b->cap ? b->cap * 2 : 4096;
    while (new_cap < b->size + needed)
        new_cap *= 2;
    b->data = realloc(b->data, new_cap);
    if (!b->data) fatal("out of memory (buf_grow %zu)", new_cap);
    b->cap = new_cap;
}

void buf_append(Buf *b, const void *src, size_t len) {
    if (len == 0) return;
    buf_grow(b, len);
    memcpy(b->data + b->size, src, len);
    b->size += len;
}

void buf_append_zero(Buf *b, size_t len) {
    if (len == 0) return;
    buf_grow(b, len);
    memset(b->data + b->size, 0, len);
    b->size += len;
}

void buf_align(Buf *b, size_t alignment) {
    if (alignment <= 1) return;
    size_t aligned = (b->size + alignment - 1) & ~(alignment - 1);
    if (aligned > b->size)
        buf_append_zero(b, aligned - b->size);
}

void buf_free(Buf *b) {
    free(b->data);
    b->data = NULL;
    b->size = 0;
    b->cap  = 0;
}

/* ── Symbol table operations ────────────────────────────────────────── */

int find_global_sym(Ctx *ctx, const char *name) {
    /* Linear search — fine for typical library sizes.
     * Could add a hash table if this becomes a bottleneck. */
    for (int i = 0; i < ctx->nsyms; i++) {
        if (ctx->syms[i].name && strcmp(ctx->syms[i].name, name) == 0 &&
            ctx->syms[i].bind != STB_LOCAL)
            return i;
    }
    return -1;
}

int add_global_sym(Ctx *ctx, const char *name, uint8_t bind, uint8_t type,
                   int defined, int obj_idx, int sec_idx,
                   uint64_t sec_off, uint64_t size) {
    if (ctx->nsyms >= ctx->syms_cap) {
        ctx->syms_cap = ctx->syms_cap ? ctx->syms_cap * 2 : 1024;
        ctx->syms = realloc(ctx->syms, ctx->syms_cap * sizeof(Symbol));
        if (!ctx->syms) fatal("out of memory (symbols)");
    }

    int idx = ctx->nsyms++;
    Symbol *s = &ctx->syms[idx];
    memset(s, 0, sizeof(*s));
    s->name     = strdup(name ? name : "");
    s->bind     = bind;
    s->type     = type;
    s->defined  = defined;
    s->obj_idx  = obj_idx;
    s->sec_idx  = sec_idx;
    s->sec_off  = sec_off;
    s->size     = size;
    s->out_sec  = SEC_NONE;
    return idx;
}

/* ── Detect file type by content ────────────────────────────────────── */

static int is_archive(const uint8_t *data, size_t size) {
    return size >= AR_MAGIC_LEN &&
           memcmp(data, AR_MAGIC, AR_MAGIC_LEN) == 0;
}

static int is_elf_object(const uint8_t *data, size_t size) {
    return size >= 4 &&
           data[0] == ELFMAG0 && data[1] == ELFMAG1 &&
           data[2] == ELFMAG2 && data[3] == ELFMAG3;
}

/* ── Parse a hex address string (0x prefix optional) ────────────────── */

static uint64_t parse_address(const char *str) {
    uint64_t val = 0;
    if (str[0] == '0' && (str[1] == 'x' || str[1] == 'X'))
        str += 2;
    while (*str) {
        char c = *str++;
        uint8_t d;
        if (c >= '0' && c <= '9')      d = c - '0';
        else if (c >= 'a' && c <= 'f')  d = c - 'a' + 10;
        else if (c >= 'A' && c <= 'F')  d = c - 'A' + 10;
        else break;
        val = (val << 4) | d;
    }
    return val;
}

/* ── Usage ──────────────────────────────────────────────────────────── */

static void usage(void) {
    fprintf(stderr,
        "anyld — anyOS ELF64 Shared Object Linker\n"
        "\n"
        "Usage: anyld [options] <input.o|input.a> ...\n"
        "\n"
        "Options:\n"
        "  -o <file>    Output file (required)\n"
        "  -b <addr>    Base virtual address (default: 0x04000000)\n"
        "  -e <file>    Export symbol definition file (.def)\n"
        "  -v           Verbose output\n"
        "  -h           Show this help\n"
        "\n"
        "Input files can be ELF64 relocatable objects (.o) or\n"
        "AR archives (.a) containing such objects.\n"
        "\n"
        "The .def file format:\n"
        "  LIBRARY <name>\n"
        "  EXPORTS\n"
        "    symbol_name_1\n"
        "    symbol_name_2\n"
    );
    exit(1);
}

/* ── Main ───────────────────────────────────────────────────────────── */

int main(int argc, char **argv) {
    Ctx ctx;
    memset(&ctx, 0, sizeof(ctx));
    ctx.base_addr = 0x04000000;  /* Default: anyOS DLL base */

    const char *def_path = NULL;
    int verbose = 0;

    /* Collect input file paths */
    const char *inputs[512];
    int ninputs = 0;

    /* Parse arguments */
    int i = 1;
    while (i < argc) {
        if (argv[i][0] == '-') {
            if (strcmp(argv[i], "-o") == 0) {
                if (++i >= argc) fatal("-o requires an argument");
                ctx.output_path = argv[i];
            } else if (strcmp(argv[i], "-b") == 0) {
                if (++i >= argc) fatal("-b requires an argument");
                ctx.base_addr = parse_address(argv[i]);
            } else if (strcmp(argv[i], "-e") == 0) {
                if (++i >= argc) fatal("-e requires an argument");
                def_path = argv[i];
            } else if (strcmp(argv[i], "-v") == 0) {
                verbose = 1;
            } else if (strcmp(argv[i], "-h") == 0 ||
                       strcmp(argv[i], "--help") == 0) {
                usage();
            } else {
                fprintf(stderr, "anyld: unknown option '%s'\n", argv[i]);
                usage();
            }
        } else {
            if (ninputs >= 512) fatal("too many input files");
            inputs[ninputs++] = argv[i];
        }
        i++;
    }

    if (!ctx.output_path) {
        fprintf(stderr, "anyld: no output file specified (-o)\n");
        usage();
    }
    if (ninputs == 0) {
        fprintf(stderr, "anyld: no input files\n");
        usage();
    }

    /* ── Step 1: Parse export definitions ───────────────────────────── */
    if (def_path) {
        if (parse_def_file(&ctx, def_path) != 0)
            fatal("failed to parse '%s'", def_path);
        if (verbose)
            printf("anyld: %d export symbols from '%s'\n",
                   ctx.nexports, def_path);
    }

    /* ── Step 2: Read input files ───────────────────────────────────── */
    for (int j = 0; j < ninputs; j++) {
        size_t probe_size;
        uint8_t *probe = read_file(inputs[j], &probe_size);
        if (!probe) fatal("cannot read '%s'", inputs[j]);

        if (is_archive(probe, probe_size)) {
            free(probe);  /* read_archive reads it again */
            if (read_archive(&ctx, inputs[j]) != 0)
                fatal("failed to read archive '%s'", inputs[j]);
        } else if (is_elf_object(probe, probe_size)) {
            if (parse_object(&ctx, inputs[j], probe, probe_size, 1) != 0)
                fatal("failed to parse '%s'", inputs[j]);
        } else {
            fprintf(stderr, "anyld: '%s': unrecognized file format\n",
                    inputs[j]);
            free(probe);
            return 1;
        }
    }

    if (verbose)
        printf("anyld: %d objects loaded\n", ctx.nobjs);

    /* ── Step 3: Merge sections ─────────────────────────────────────── */
    if (merge_sections(&ctx) != 0)
        fatal("section merge failed");

    if (verbose) {
        printf("anyld: merged sections:\n");
        printf("  .text:   %zu bytes\n", ctx.text.size);
        printf("  .rodata: %zu bytes\n", ctx.rodata.size);
        printf("  .data:   %zu bytes\n", ctx.data.size);
        printf("  .bss:    %llu bytes\n", (unsigned long long)ctx.bss_size);
    }

    /* ── Step 4: Collect and resolve symbols ─────────────────────────── */
    if (collect_symbols(&ctx) != 0)
        fatal("symbol collection failed");

    if (verbose)
        printf("anyld: %d global symbols\n", ctx.nsyms);

    if (resolve_symbols(&ctx) != 0)
        fatal("unresolved symbols (see above)");

    /* ── Step 5: Mark exported symbols ──────────────────────────────── */
    if (ctx.nexports > 0) {
        mark_exports(&ctx);
    } else {
        /* No .def file: export all global defined symbols */
        for (int j = 0; j < ctx.nsyms; j++) {
            Symbol *s = &ctx.syms[j];
            if (s->defined && s->bind == STB_GLOBAL &&
                s->type != STT_SECTION && s->name[0] != '\0')
                s->is_export = 1;
        }
    }

    /* ── Step 6: Compute section layout (VMAs) ──────────────────────── */
    if (compute_layout(&ctx) != 0)
        fatal("layout computation failed");

    /* ── Step 7: Apply relocations ──────────────────────────────────── */
    if (apply_relocations(&ctx) != 0)
        fatal("relocation failed");

    if (verbose)
        printf("anyld: %d relocations applied\n", ctx.nrelocs);

    /* ── Step 8: Write output ───────────────────────────────────────── */
    if (write_output(&ctx) != 0)
        fatal("output generation failed");

    return 0;
}
