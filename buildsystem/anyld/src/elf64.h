/*
 * elf64.h — ELF64 format definitions for anyld.
 *
 * Standard ELF structures and constants for x86_64.
 * Written in C99 for TCC compatibility (self-hosting on anyOS).
 */
#ifndef ELF64_H
#define ELF64_H

#include <stdint.h>

/* ── ELF base types ─────────────────────────────────────────────────── */

typedef uint64_t Elf64_Addr;
typedef uint64_t Elf64_Off;
typedef uint16_t Elf64_Half;
typedef uint32_t Elf64_Word;
typedef int32_t  Elf64_Sword;
typedef uint64_t Elf64_Xword;
typedef int64_t  Elf64_Sxword;

/* ── ELF identification ─────────────────────────────────────────────── */

#define EI_NIDENT      16
#define ELFMAG0        0x7F
#define ELFMAG1        'E'
#define ELFMAG2        'L'
#define ELFMAG3        'F'

#define ELFCLASS64     2       /* 64-bit objects */
#define ELFDATA2LSB    1       /* Little-endian */
#define EV_CURRENT     1       /* Current ELF version */
#define ELFOSABI_NONE  0       /* No OS-specific ABI */

/* ── ELF header ─────────────────────────────────────────────────────── */

typedef struct {
    unsigned char e_ident[EI_NIDENT];
    Elf64_Half    e_type;
    Elf64_Half    e_machine;
    Elf64_Word    e_version;
    Elf64_Addr    e_entry;
    Elf64_Off     e_phoff;
    Elf64_Off     e_shoff;
    Elf64_Word    e_flags;
    Elf64_Half    e_ehsize;
    Elf64_Half    e_phentsize;
    Elf64_Half    e_phnum;
    Elf64_Half    e_shentsize;
    Elf64_Half    e_shnum;
    Elf64_Half    e_shstrndx;
} Elf64_Ehdr;

/* e_type values */
#define ET_NONE   0
#define ET_REL    1   /* Relocatable */
#define ET_EXEC   2   /* Executable */
#define ET_DYN    3   /* Shared object */
#define ET_CORE   4

/* e_machine values */
#define EM_X86_64  62
#define EM_AARCH64 183

/* ── Section header ─────────────────────────────────────────────────── */

typedef struct {
    Elf64_Word    sh_name;
    Elf64_Word    sh_type;
    Elf64_Xword   sh_flags;
    Elf64_Addr    sh_addr;
    Elf64_Off     sh_offset;
    Elf64_Xword   sh_size;
    Elf64_Word    sh_link;
    Elf64_Word    sh_info;
    Elf64_Xword   sh_addralign;
    Elf64_Xword   sh_entsize;
} Elf64_Shdr;

/* sh_type values */
#define SHT_NULL          0
#define SHT_PROGBITS      1
#define SHT_SYMTAB        2
#define SHT_STRTAB        3
#define SHT_RELA          4
#define SHT_HASH          5
#define SHT_DYNAMIC       6
#define SHT_NOTE          7
#define SHT_NOBITS        8
#define SHT_REL           9
#define SHT_DYNSYM        11

/* sh_flags values */
#define SHF_WRITE         0x1
#define SHF_ALLOC         0x2
#define SHF_EXECINSTR     0x4
#define SHF_MERGE         0x10
#define SHF_STRINGS       0x20
#define SHF_INFO_LINK     0x40

/* Special section indices */
#define SHN_UNDEF         0
#define SHN_ABS           0xFFF1
#define SHN_COMMON        0xFFF2

/* ── Symbol table entry ─────────────────────────────────────────────── */

typedef struct {
    Elf64_Word    st_name;
    unsigned char st_info;
    unsigned char st_other;
    Elf64_Half    st_shndx;
    Elf64_Addr    st_value;
    Elf64_Xword   st_size;
} Elf64_Sym;

/* Symbol binding (high nibble of st_info) */
#define STB_LOCAL   0
#define STB_GLOBAL  1
#define STB_WEAK    2

/* Symbol type (low nibble of st_info) */
#define STT_NOTYPE  0
#define STT_OBJECT  1
#define STT_FUNC    2
#define STT_SECTION 3
#define STT_FILE    4

/* Macros for st_info */
#define ELF64_ST_BIND(i)    ((i) >> 4)
#define ELF64_ST_TYPE(i)    ((i) & 0xF)
#define ELF64_ST_INFO(b,t)  (((b) << 4) | ((t) & 0xF))

/* Symbol visibility (st_other) */
#define STV_DEFAULT   0
#define STV_HIDDEN    2

/* ── Relocation entries ─────────────────────────────────────────────── */

typedef struct {
    Elf64_Addr    r_offset;
    Elf64_Xword   r_info;
    Elf64_Sxword  r_addend;
} Elf64_Rela;

/* Macros for r_info */
#define ELF64_R_SYM(i)     ((i) >> 32)
#define ELF64_R_TYPE(i)    ((i) & 0xFFFFFFFF)
#define ELF64_R_INFO(s,t)  (((Elf64_Xword)(s) << 32) | ((t) & 0xFFFFFFFF))

/* x86_64 relocation types */
#define R_X86_64_NONE       0
#define R_X86_64_64         1   /* S + A        (absolute 64-bit) */
#define R_X86_64_PC32       2   /* S + A - P    (PC-relative 32-bit) */
#define R_X86_64_GOT32      3   /* G + A        (GOT offset 32-bit) */
#define R_X86_64_PLT32      4   /* L + A - P    (PLT-relative 32-bit) */
#define R_X86_64_COPY       5
#define R_X86_64_GLOB_DAT   6
#define R_X86_64_JUMP_SLOT  7
#define R_X86_64_RELATIVE   8   /* B + A        (base-relative) */
#define R_X86_64_GOTPCREL   9   /* G + GOT + A - P */
#define R_X86_64_32         10  /* S + A        (zero-extend 32-bit) */
#define R_X86_64_32S        11  /* S + A        (sign-extend 32-bit) */
#define R_X86_64_16         12
#define R_X86_64_PC16       13
#define R_X86_64_8          14
#define R_X86_64_PC8        15
#define R_X86_64_PC64       24  /* S + A - P    (PC-relative 64-bit) */
#define R_X86_64_GOTPCRELX  41  /* Relaxable GOTPCREL */
#define R_X86_64_REX_GOTPCRELX 42

/* AArch64 relocation types */
#define R_AARCH64_NONE              0
#define R_AARCH64_ABS64             257  /* S + A        (absolute 64-bit) */
#define R_AARCH64_ABS32             258  /* S + A        (absolute 32-bit) */
#define R_AARCH64_ABS16             259  /* S + A        (absolute 16-bit) */
#define R_AARCH64_PREL64            260  /* S + A - P    (PC-relative 64-bit) */
#define R_AARCH64_PREL32            261  /* S + A - P    (PC-relative 32-bit) */
#define R_AARCH64_PREL16            262  /* S + A - P    (PC-relative 16-bit) */
#define R_AARCH64_ADR_PREL_PG_HI21 275  /* Page(S+A)-Page(P) (ADRP imm) */
#define R_AARCH64_ADD_ABS_LO12_NC  277  /* (S+A) & 0xFFF   (ADD imm12) */
#define R_AARCH64_LDST8_ABS_LO12_NC  278  /* (S+A) & 0xFFF (LDR/STR 8-bit) */
#define R_AARCH64_JUMP26            282  /* S+A-P >> 2   (B imm26) */
#define R_AARCH64_CALL26            283  /* S+A-P >> 2   (BL imm26) */
#define R_AARCH64_LDST16_ABS_LO12_NC 284  /* ((S+A)&0xFFF)>>1 (16-bit) */
#define R_AARCH64_LDST32_ABS_LO12_NC 285  /* ((S+A)&0xFFF)>>2 (32-bit) */
#define R_AARCH64_LDST64_ABS_LO12_NC 286  /* ((S+A)&0xFFF)>>3 (64-bit) */
#define R_AARCH64_LDST128_ABS_LO12_NC 299 /* ((S+A)&0xFFF)>>4 (128-bit) */
#define R_AARCH64_ADR_GOT_PAGE      311  /* Page(G(S))-Page(P) (ADRP GOT) */
#define R_AARCH64_LD64_GOT_LO12_NC  312  /* G(S) & 0xFFF (LDR GOT) */
#define R_AARCH64_RELATIVE          1024 /* B + A        (base-relative) */

/* ── Program header ─────────────────────────────────────────────────── */

typedef struct {
    Elf64_Word    p_type;
    Elf64_Word    p_flags;
    Elf64_Off     p_offset;
    Elf64_Addr    p_vaddr;
    Elf64_Addr    p_paddr;
    Elf64_Xword   p_filesz;
    Elf64_Xword   p_memsz;
    Elf64_Xword   p_align;
} Elf64_Phdr;

/* p_type values */
#define PT_NULL     0
#define PT_LOAD     1
#define PT_DYNAMIC  2
#define PT_INTERP   3
#define PT_NOTE     4
#define PT_PHDR     6

/* p_flags values */
#define PF_X  0x1  /* Execute */
#define PF_W  0x2  /* Write */
#define PF_R  0x4  /* Read */

/* ── Dynamic section entry ──────────────────────────────────────────── */

typedef struct {
    Elf64_Sxword  d_tag;
    union {
        Elf64_Xword d_val;
        Elf64_Addr  d_ptr;
    } d_un;
} Elf64_Dyn;

/* d_tag values */
#define DT_NULL     0   /* End of .dynamic */
#define DT_NEEDED   1   /* Name of needed library */
#define DT_HASH     4   /* Address of symbol hash table */
#define DT_STRTAB   5   /* Address of .dynstr */
#define DT_SYMTAB   6   /* Address of .dynsym */
#define DT_STRSZ    10  /* Size of .dynstr */
#define DT_SYMENT   11  /* Size of one Elf64_Sym */
#define DT_RELA     7   /* Address of Rela relocs */
#define DT_RELASZ   8   /* Total size of Rela relocs */
#define DT_RELAENT  9   /* Size of one Rela reloc entry */
#define DT_SONAME   14  /* Shared object name */
#define DT_RELACOUNT 0x6FFFFFF9  /* Count of RELATIVE relocs */

/* ── ELF hash function ──────────────────────────────────────────────── */

static inline uint32_t elf_hash(const char *name) {
    uint32_t h = 0, g;
    const unsigned char *p = (const unsigned char *)name;
    while (*p) {
        h = (h << 4) + *p++;
        g = h & 0xF0000000;
        if (g) h ^= g >> 24;
        h &= ~g;
    }
    return h;
}

/* ── AR archive format ──────────────────────────────────────────────── */

#define AR_MAGIC  "!<arch>\n"
#define AR_MAGIC_LEN 8
#define AR_HDR_SIZE  60

/* AR member header (60 bytes, ASCII) */
typedef struct {
    char ar_name[16];
    char ar_date[12];
    char ar_uid[6];
    char ar_gid[6];
    char ar_mode[8];
    char ar_size[10];
    char ar_fmag[2];   /* "`\n" */
} ArHdr;

/* ── Page size ──────────────────────────────────────────────────────── */

#define PAGE_SIZE  4096
#define PAGE_ALIGN(x) (((x) + PAGE_SIZE - 1) & ~(uint64_t)(PAGE_SIZE - 1))

#endif /* ELF64_H */
