/*
 * anyld.h — anyOS ELF64 shared object linker: context and prototypes.
 *
 * C99 / TCC compatible for self-hosting on anyOS.
 */
#ifndef ANYLD_H
#define ANYLD_H

#include "elf64.h"

#include <stdint.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* ── Output section classification ──────────────────────────────────── */

#define SEC_NONE    0
#define SEC_TEXT    1
#define SEC_RODATA  2
#define SEC_DATA    3
#define SEC_BSS     4

/* ── Growable byte buffer ───────────────────────────────────────────── */

typedef struct {
    uint8_t *data;
    size_t   size;
    size_t   cap;
} Buf;

void buf_init(Buf *b);
void buf_append(Buf *b, const void *src, size_t len);
void buf_append_zero(Buf *b, size_t len);
void buf_align(Buf *b, size_t alignment);
void buf_free(Buf *b);

/* ── Input section → output mapping ─────────────────────────────────── */

typedef struct {
    int      out_sec;   /* SEC_TEXT, SEC_RODATA, etc. */
    uint64_t out_off;   /* Byte offset into merged output section */
} SecMap;

/* ── Input object file ──────────────────────────────────────────────── */

typedef struct {
    char       *filename;
    uint8_t    *data;       /* Raw file content (owned if from archive) */
    size_t      size;
    int         data_owned; /* 1 if we should free data */

    Elf64_Ehdr  ehdr;
    Elf64_Shdr *shdrs;
    uint16_t    nshdr;
    char       *shstrtab;   /* Section name string table */

    Elf64_Sym  *symtab;     /* .symtab symbols */
    uint32_t    nsym;
    char       *strtab;     /* .strtab strings */
    uint32_t    symtab_shndx; /* Section index of .symtab */

    SecMap     *sec_map;    /* Array[nshdr]: input section → output */
    uint32_t   *sym_map;    /* Array[nsym]:  local sym idx → global idx */
} InputObj;

/* ── Global symbol ──────────────────────────────────────────────────── */

typedef struct {
    char       *name;       /* strdup'd */
    uint64_t    value;      /* Virtual address (set during layout) */
    uint64_t    size;
    uint8_t     bind;       /* STB_LOCAL, STB_GLOBAL, STB_WEAK */
    uint8_t     type;       /* STT_FUNC, STT_OBJECT, etc. */
    int         defined;    /* 1 = has definition, 0 = undefined */
    int         obj_idx;    /* Defining input object index */
    int         sec_idx;    /* Section index in defining object */
    int         out_sec;    /* Output section (SEC_TEXT etc.) */
    uint64_t    sec_off;    /* Offset within the input section */
    int         is_export;  /* 1 = listed in .def export file */
} Symbol;

/* ── Pending relocation ─────────────────────────────────────────────── */

typedef struct {
    int         out_sec;    /* Target output section to patch */
    uint64_t    offset;     /* Byte offset within that section */
    uint32_t    type;       /* R_X86_64_* relocation type */
    int64_t     addend;
    uint32_t    sym_idx;    /* Index in global symbol table */
} Reloc;

/* ── Linker context (all state lives here) ──────────────────────────── */

typedef struct {
    /* Input objects */
    InputObj   *objs;
    int         nobjs;
    int         objs_cap;

    /* Global symbol table */
    Symbol     *syms;
    int         nsyms;
    int         syms_cap;

    /* Pending relocations */
    Reloc      *relocs;
    int         nrelocs;
    int         relocs_cap;

    /* Merged output sections */
    Buf         text;
    Buf         rodata;
    Buf         data;
    uint64_t    bss_size;
    uint32_t    bss_align;

    /* Virtual address layout (set by layout) */
    uint64_t    base_addr;
    uint64_t    text_vaddr;
    uint64_t    rodata_vaddr;
    uint64_t    data_vaddr;
    uint64_t    bss_vaddr;
    uint64_t    dynamic_vaddr;

    /* Runtime relocations (.rela.dyn) */
    Buf         rela_dyn;
    int         nrela_dyn;

    /* Export definitions from .def file */
    char      **exports;
    int         nexports;
    int         exports_cap;
    char       *lib_name;

    /* Paths */
    const char *output_path;
    int         quiet;
} Ctx;

/* ── input.c ────────────────────────────────────────────────────────── */

int  parse_object(Ctx *ctx, const char *filename, uint8_t *data,
                  size_t size, int data_owned);
int  read_object_file(Ctx *ctx, const char *path);
int  read_archive(Ctx *ctx, const char *path);

/* ── link.c ─────────────────────────────────────────────────────────── */

int  classify_section(const char *name, uint64_t flags);
int  merge_sections(Ctx *ctx);
int  collect_symbols(Ctx *ctx);
int  resolve_symbols(Ctx *ctx);
int  apply_relocations(Ctx *ctx);

/* ── output.c ───────────────────────────────────────────────────────── */

int  compute_layout(Ctx *ctx);
int  write_output(Ctx *ctx);

/* ── defs.c ─────────────────────────────────────────────────────────── */

int  parse_def_file(Ctx *ctx, const char *path);
void mark_exports(Ctx *ctx);

/* ── Utilities (defined in anyld.c) ─────────────────────────────────── */

uint8_t *read_file(const char *path, size_t *out_size);
void     fatal(const char *fmt, ...);

/* Add a global symbol, return its index */
int      add_global_sym(Ctx *ctx, const char *name, uint8_t bind,
                        uint8_t type, int defined, int obj_idx,
                        int sec_idx, uint64_t sec_off, uint64_t size);
/* Find a global symbol by name, return index or -1 */
int      find_global_sym(Ctx *ctx, const char *name);

#endif /* ANYLD_H */
