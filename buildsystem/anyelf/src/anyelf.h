/*
 * anyelf.h — ELF format definitions for anyelf conversion tool
 *
 * Supports both ELF32 and ELF64 for cross-format conversion.
 * Written in C99 for TCC compatibility (self-hosting on anyOS).
 */
#ifndef ANYELF_H
#define ANYELF_H

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <stdarg.h>

/* ── ELF identification ───────────────────────────────────────────────── */

#define ELFMAG0  0x7f
#define ELFMAG1  'E'
#define ELFMAG2  'L'
#define ELFMAG3  'F'

#define ELFCLASS32 1
#define ELFCLASS64 2

/* ── ELF types ────────────────────────────────────────────────────────── */

#define ET_REL   1
#define ET_EXEC  2
#define ET_DYN   3

/* ── Program header types ─────────────────────────────────────────────── */

#define PT_LOAD  1

/* ── Segment flags ────────────────────────────────────────────────────── */

#define PF_X  0x1
#define PF_W  0x2
#define PF_R  0x4

/* ── Section types ────────────────────────────────────────────────────── */

#define SHT_SYMTAB 2
#define SHT_STRTAB 3

/* ── Symbol binding ───────────────────────────────────────────────────── */

#define STB_LOCAL  0
#define STB_GLOBAL 1

/* ── Utilities ────────────────────────────────────────────────────────── */

#define PAGE_SIZE  4096
#define ALIGN_UP(v, a) (((v) + (a) - 1) & ~((uint64_t)(a) - 1))

/* ── ELF32 structures ─────────────────────────────────────────────────── */

typedef struct {
    uint8_t  e_ident[16];
    uint16_t e_type;
    uint16_t e_machine;
    uint32_t e_version;
    uint32_t e_entry;
    uint32_t e_phoff;
    uint32_t e_shoff;
    uint32_t e_flags;
    uint16_t e_ehsize;
    uint16_t e_phentsize;
    uint16_t e_phnum;
    uint16_t e_shentsize;
    uint16_t e_shnum;
    uint16_t e_shstrndx;
} Elf32_Ehdr;

typedef struct {
    uint32_t p_type;
    uint32_t p_offset;
    uint32_t p_vaddr;
    uint32_t p_paddr;
    uint32_t p_filesz;
    uint32_t p_memsz;
    uint32_t p_flags;
    uint32_t p_align;
} Elf32_Phdr;

/* ── ELF64 structures ─────────────────────────────────────────────────── */

typedef struct {
    uint8_t  e_ident[16];
    uint16_t e_type;
    uint16_t e_machine;
    uint32_t e_version;
    uint64_t e_entry;
    uint64_t e_phoff;
    uint64_t e_shoff;
    uint32_t e_flags;
    uint16_t e_ehsize;
    uint16_t e_phentsize;
    uint16_t e_phnum;
    uint16_t e_shentsize;
    uint16_t e_shnum;
    uint16_t e_shstrndx;
} Elf64_Ehdr;

typedef struct {
    uint32_t p_type;
    uint32_t p_flags;
    uint64_t p_offset;
    uint64_t p_vaddr;
    uint64_t p_paddr;
    uint64_t p_filesz;
    uint64_t p_memsz;
    uint64_t p_align;
} Elf64_Phdr;

typedef struct {
    uint32_t sh_name;
    uint32_t sh_type;
    uint64_t sh_flags;
    uint64_t sh_addr;
    uint64_t sh_offset;
    uint64_t sh_size;
    uint32_t sh_link;
    uint32_t sh_info;
    uint64_t sh_addralign;
    uint64_t sh_entsize;
} Elf64_Shdr;

typedef struct {
    uint32_t st_name;
    uint8_t  st_info;
    uint8_t  st_other;
    uint16_t st_shndx;
    uint64_t st_value;
    uint64_t st_size;
} Elf64_Sym;

/* ── Parsed segment (unified for ELF32/ELF64) ─────────────────────────── */

typedef struct {
    uint64_t vaddr;
    uint64_t paddr;
    uint64_t offset;
    uint64_t filesz;
    uint64_t memsz;
    uint32_t flags;
} Segment;

/* ── Function prototypes ──────────────────────────────────────────────── */

/* Utility */
void     fatal(const char *fmt, ...);
uint8_t *read_file(const char *path, size_t *out_size);

/* ELF parsing */
int      parse_segments(const uint8_t *data, size_t size,
                        Segment **out_segs, int *out_nsegs,
                        int *out_class);
uint64_t find_symbol_64(const uint8_t *data, size_t size, const char *name);

/* Conversion modes */
int do_bin(const char *input, const char *output);
int do_pflat(const char *input, const char *output, uint64_t base_paddr);
int do_dlib(const char *input, const char *output);
int do_kdrv(const char *input, const char *output, const char *exports_symbol);

#endif /* ANYELF_H */
