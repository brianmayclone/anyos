/*
 * anyelf — anyOS ELF conversion tool
 *
 * Replaces elf2bin.py and elf2kdrv.py with a single C tool.
 * Supports: bin, pflat, dlib, kdrv modes.
 *
 * Written in C99 for TCC compatibility (self-hosting on anyOS).
 *
 * Usage:
 *   anyelf bin   <input.elf> <output.bin>
 *   anyelf pflat <input.elf> <output.bin> [base_paddr]
 *   anyelf dlib  <input.elf> <output.dlib>
 *   anyelf kdrv  <input.elf> <output.kdrv> [--exports-symbol NAME]
 */
#include "anyelf.h"

#ifdef ONE_SOURCE
/* Single-source compilation mode (for TCC on anyOS) */
#include "convert.c"
#endif

/* ── Utility: fatal error ─────────────────────────────────────────────── */

void fatal(const char *fmt, ...) {
    va_list ap;
    fprintf(stderr, "anyelf: fatal: ");
    va_start(ap, fmt);
    vfprintf(stderr, fmt, ap);
    va_end(ap);
    fprintf(stderr, "\n");
    exit(1);
}

/* ── Utility: read entire file ────────────────────────────────────────── */

uint8_t *read_file(const char *path, size_t *out_size) {
    FILE *fp = fopen(path, "rb");
    if (!fp) {
        fprintf(stderr, "anyelf: cannot open '%s'\n", path);
        return NULL;
    }

    fseek(fp, 0, SEEK_END);
    long sz = ftell(fp);
    if (sz < 0) { fclose(fp); return NULL; }
    fseek(fp, 0, SEEK_SET);

    uint8_t *buf = malloc((size_t)sz);
    if (!buf) { fclose(fp); return NULL; }

    size_t n = fread(buf, 1, (size_t)sz, fp);
    fclose(fp);

    if (n != (size_t)sz) {
        fprintf(stderr, "anyelf: short read on '%s'\n", path);
        free(buf);
        return NULL;
    }

    *out_size = (size_t)sz;
    return buf;
}

/* ── ELF segment parser (handles ELF32 + ELF64) ──────────────────────── */

int parse_segments(const uint8_t *data, size_t size,
                   Segment **out_segs, int *out_nsegs,
                   int *out_class) {
    if (size < 16 ||
        data[0] != ELFMAG0 || data[1] != ELFMAG1 ||
        data[2] != ELFMAG2 || data[3] != ELFMAG3) {
        fprintf(stderr, "anyelf: not an ELF file\n");
        return -1;
    }

    int ei_class = data[4];
    *out_class = ei_class;

    uint64_t e_phoff;
    uint16_t e_phentsize, e_phnum;

    if (ei_class == ELFCLASS64) {
        Elf64_Ehdr *ehdr = (Elf64_Ehdr *)data;
        e_phoff     = ehdr->e_phoff;
        e_phentsize = ehdr->e_phentsize;
        e_phnum     = ehdr->e_phnum;
    } else if (ei_class == ELFCLASS32) {
        Elf32_Ehdr *ehdr = (Elf32_Ehdr *)data;
        e_phoff     = ehdr->e_phoff;
        e_phentsize = ehdr->e_phentsize;
        e_phnum     = ehdr->e_phnum;
    } else {
        fprintf(stderr, "anyelf: unknown ELF class %d\n", ei_class);
        return -1;
    }

    Segment *segs = malloc(e_phnum * sizeof(Segment));
    if (!segs) fatal("out of memory");
    int nsegs = 0;

    for (int i = 0; i < e_phnum; i++) {
        uint64_t off = e_phoff + (uint64_t)i * e_phentsize;
        uint32_t p_type;
        Segment seg;
        memset(&seg, 0, sizeof(seg));

        if (ei_class == ELFCLASS64) {
            Elf64_Phdr *ph = (Elf64_Phdr *)(data + off);
            p_type     = ph->p_type;
            seg.vaddr  = ph->p_vaddr;
            seg.paddr  = ph->p_paddr;
            seg.offset = ph->p_offset;
            seg.filesz = ph->p_filesz;
            seg.memsz  = ph->p_memsz;
            seg.flags  = ph->p_flags;
        } else {
            Elf32_Phdr *ph = (Elf32_Phdr *)(data + off);
            p_type     = ph->p_type;
            seg.vaddr  = ph->p_vaddr;
            seg.paddr  = ph->p_paddr;
            seg.offset = ph->p_offset;
            seg.filesz = ph->p_filesz;
            seg.memsz  = ph->p_memsz;
            seg.flags  = ph->p_flags;
        }

        if (p_type == PT_LOAD)
            segs[nsegs++] = seg;
    }

    *out_segs = segs;
    *out_nsegs = nsegs;
    return 0;
}

/* ── Find symbol by name in ELF64 ─────────────────────────────────────── */

uint64_t find_symbol_64(const uint8_t *data, size_t size,
                        const char *name) {
    (void)size;
    Elf64_Ehdr *ehdr = (Elf64_Ehdr *)data;

    for (uint16_t i = 0; i < ehdr->e_shnum; i++) {
        Elf64_Shdr *sh = (Elf64_Shdr *)(data + ehdr->e_shoff +
                                         (uint64_t)i * ehdr->e_shentsize);
        if (sh->sh_type != SHT_SYMTAB) continue;

        Elf64_Shdr *strtab_sh = (Elf64_Shdr *)(data + ehdr->e_shoff +
                                    (uint64_t)sh->sh_link * ehdr->e_shentsize);
        const char *strtab = (const char *)(data + strtab_sh->sh_offset);

        uint64_t nsyms = sh->sh_entsize ? sh->sh_size / sh->sh_entsize : 0;
        for (uint64_t j = 0; j < nsyms; j++) {
            Elf64_Sym *sym = (Elf64_Sym *)(data + sh->sh_offset +
                                           j * sh->sh_entsize);
            if (sym->st_name &&
                strcmp(strtab + sym->st_name, name) == 0) {
                return sym->st_value;
            }
        }
    }

    return (uint64_t)-1;
}

/* ── Parse hex address ────────────────────────────────────────────────── */

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

/* ── Usage ────────────────────────────────────────────────────────────── */

static void usage(void) {
    fprintf(stderr,
        "anyelf — anyOS ELF conversion tool\n"
        "\n"
        "Usage:\n"
        "  anyelf bin   <input.elf> <output.bin>        "
            "Flat binary (by vaddr)\n"
        "  anyelf pflat <input.elf> <output.bin> [base]  "
            "Flat binary (by paddr)\n"
        "  anyelf dlib  <input.elf> <output.dlib>        "
            "DLIB v3 dynamic library\n"
        "  anyelf kdrv  <input.elf> <output.kdrv>        "
            "KDRV kernel driver\n"
        "               [--exports-symbol NAME]          "
            "(default: DRIVER_EXPORTS)\n"
    );
    exit(1);
}

/* ── Main ─────────────────────────────────────────────────────────────── */

int main(int argc, char **argv) {
    if (argc < 2) usage();

    if (strcmp(argv[1], "bin") == 0) {
        if (argc != 4) {
            fprintf(stderr, "anyelf bin: expected 2 arguments\n");
            usage();
        }
        return do_bin(argv[2], argv[3]);

    } else if (strcmp(argv[1], "pflat") == 0) {
        if (argc < 4 || argc > 5) {
            fprintf(stderr, "anyelf pflat: expected 2-3 arguments\n");
            usage();
        }
        uint64_t base = 0x00100000;  /* default kernel LMA */
        if (argc == 5)
            base = parse_address(argv[4]);
        return do_pflat(argv[2], argv[3], base);

    } else if (strcmp(argv[1], "dlib") == 0) {
        if (argc != 4) {
            fprintf(stderr, "anyelf dlib: expected 2 arguments\n");
            usage();
        }
        return do_dlib(argv[2], argv[3]);

    } else if (strcmp(argv[1], "kdrv") == 0) {
        if (argc < 4) {
            fprintf(stderr, "anyelf kdrv: expected at least 2 arguments\n");
            usage();
        }
        const char *exports_sym = "DRIVER_EXPORTS";
        int i = 4;
        while (i < argc) {
            if (strcmp(argv[i], "--exports-symbol") == 0 && i + 1 < argc) {
                exports_sym = argv[i + 1];
                i += 2;
            } else {
                i++;
            }
        }
        return do_kdrv(argv[2], argv[3], exports_sym);

    } else {
        fprintf(stderr, "anyelf: unknown command '%s'\n", argv[1]);
        usage();
    }

    return 0;
}
