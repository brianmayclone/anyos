/*
 * input.c — Read ELF64 relocatable objects (.o) and AR archives (.a).
 */
#include "anyld.h"

/* ── Parse a single ELF64 relocatable object ────────────────────────── */

int parse_object(Ctx *ctx, const char *filename, uint8_t *data,
                 size_t size, int data_owned) {
    if (size < sizeof(Elf64_Ehdr)) {
        fprintf(stderr, "anyld: %s: too small for ELF header\n", filename);
        if (data_owned) free(data);
        return -1;
    }

    Elf64_Ehdr *ehdr = (Elf64_Ehdr *)data;

    /* Validate ELF magic */
    if (ehdr->e_ident[0] != ELFMAG0 || ehdr->e_ident[1] != ELFMAG1 ||
        ehdr->e_ident[2] != ELFMAG2 || ehdr->e_ident[3] != ELFMAG3) {
        fprintf(stderr, "anyld: %s: not an ELF file\n", filename);
        if (data_owned) free(data);
        return -1;
    }
    if (ehdr->e_ident[4] != ELFCLASS64) {
        fprintf(stderr, "anyld: %s: not ELF64 (class=%d)\n",
                filename, ehdr->e_ident[4]);
        if (data_owned) free(data);
        return -1;
    }
    if (ehdr->e_type != ET_REL) {
        fprintf(stderr, "anyld: %s: not relocatable (type=%d)\n",
                filename, ehdr->e_type);
        if (data_owned) free(data);
        return -1;
    }
    if (ehdr->e_machine != EM_X86_64 && ehdr->e_machine != EM_AARCH64) {
        fprintf(stderr, "anyld: %s: unsupported architecture (machine=%d)\n",
                filename, ehdr->e_machine);
        if (data_owned) free(data);
        return -1;
    }
    /* Verify all objects share the same architecture */
    if (ctx->nobjs == 0) {
        ctx->e_machine = ehdr->e_machine;
    } else if (ctx->e_machine != ehdr->e_machine) {
        fprintf(stderr, "anyld: %s: architecture mismatch (machine=%d, expected=%d)\n",
                filename, ehdr->e_machine, ctx->e_machine);
        if (data_owned) free(data);
        return -1;
    }

    /* Grow objects array */
    if (ctx->nobjs >= ctx->objs_cap) {
        ctx->objs_cap = ctx->objs_cap ? ctx->objs_cap * 2 : 64;
        ctx->objs = realloc(ctx->objs, ctx->objs_cap * sizeof(InputObj));
    }

    InputObj *obj = &ctx->objs[ctx->nobjs];
    memset(obj, 0, sizeof(*obj));
    obj->filename = strdup(filename);
    obj->data = data;
    obj->size = size;
    obj->data_owned = data_owned;
    obj->ehdr = *ehdr;

    /* Section headers */
    if (ehdr->e_shoff == 0 || ehdr->e_shnum == 0) {
        fprintf(stderr, "anyld: %s: no section headers\n", filename);
        ctx->nobjs++;
        return 0;
    }
    obj->nshdr = ehdr->e_shnum;
    obj->shdrs = (Elf64_Shdr *)(data + ehdr->e_shoff);

    /* Section name string table */
    if (ehdr->e_shstrndx < obj->nshdr) {
        Elf64_Shdr *shstr = &obj->shdrs[ehdr->e_shstrndx];
        obj->shstrtab = (char *)(data + shstr->sh_offset);
    }

    /* Find .symtab and its associated .strtab */
    for (uint16_t i = 0; i < obj->nshdr; i++) {
        Elf64_Shdr *sh = &obj->shdrs[i];
        if (sh->sh_type == SHT_SYMTAB) {
            obj->symtab = (Elf64_Sym *)(data + sh->sh_offset);
            obj->nsym = (uint32_t)(sh->sh_size / sizeof(Elf64_Sym));
            obj->symtab_shndx = i;
            if (sh->sh_link < obj->nshdr) {
                Elf64_Shdr *str_sh = &obj->shdrs[sh->sh_link];
                obj->strtab = (char *)(data + str_sh->sh_offset);
            }
            break;  /* Only one .symtab per object */
        }
    }

    /* Allocate section and symbol mappings */
    obj->sec_map = calloc(obj->nshdr, sizeof(SecMap));
    obj->sym_map = calloc(obj->nsym > 0 ? obj->nsym : 1, sizeof(uint32_t));

    ctx->nobjs++;
    return 0;
}

/* ── Read a single .o file from disk ────────────────────────────────── */

int read_object_file(Ctx *ctx, const char *path) {
    size_t size;
    uint8_t *data = read_file(path, &size);
    if (!data) return -1;
    return parse_object(ctx, path, data, size, 1);
}

/* ── Parse a decimal ASCII field (space-padded, like ar headers) ────── */

static long parse_ar_decimal(const char *field, int width) {
    char buf[32];
    int len = width < 31 ? width : 31;
    memcpy(buf, field, len);
    buf[len] = '\0';
    /* Trim trailing spaces */
    while (len > 0 && buf[len - 1] == ' ')
        buf[--len] = '\0';
    return atol(buf);
}

/* ── Read an AR archive (.a) file ───────────────────────────────────── */

int read_archive(Ctx *ctx, const char *path) {
    size_t ar_size;
    uint8_t *ar_data = read_file(path, &ar_size);
    if (!ar_data) return -1;

    if (ar_size < AR_MAGIC_LEN ||
        memcmp(ar_data, AR_MAGIC, AR_MAGIC_LEN) != 0) {
        fprintf(stderr, "anyld: %s: not an AR archive\n", path);
        free(ar_data);
        return -1;
    }

    char *long_names = NULL;
    size_t long_names_size = 0;
    size_t pos = AR_MAGIC_LEN;
    (void)0;  /* member_count removed — count not needed */

    while (pos + AR_HDR_SIZE <= ar_size) {
        ArHdr *hdr = (ArHdr *)(ar_data + pos);

        /* Validate fmag */
        if (hdr->ar_fmag[0] != '`' || hdr->ar_fmag[1] != '\n') {
            fprintf(stderr, "anyld: %s: corrupt ar header at offset %zu\n",
                    path, pos);
            break;
        }

        size_t member_size = (size_t)parse_ar_decimal(hdr->ar_size, 10);
        pos += AR_HDR_SIZE;

        if (pos + member_size > ar_size) {
            fprintf(stderr, "anyld: %s: truncated member at offset %zu\n",
                    path, pos);
            break;
        }

        uint8_t *member_data = ar_data + pos;

        /* Check special members */
        if (hdr->ar_name[0] == '/' && hdr->ar_name[1] == '/' &&
            hdr->ar_name[2] == ' ') {
            /* GNU long filename table */
            long_names = (char *)member_data;
            long_names_size = member_size;
        } else if (hdr->ar_name[0] == '/' && hdr->ar_name[1] == ' ') {
            /* Archive symbol table — skip */
        } else {
            /* Regular member — determine name */
            char name[256];
            name[0] = '\0';

            if (hdr->ar_name[0] == '/' &&
                hdr->ar_name[1] >= '0' && hdr->ar_name[1] <= '9') {
                /* Long name: /offset into long_names table */
                long off = atol(hdr->ar_name + 1);
                if (long_names && off >= 0 && (size_t)off < long_names_size) {
                    int j = 0;
                    while ((size_t)(off + j) < long_names_size &&
                           long_names[off + j] != '/' &&
                           long_names[off + j] != '\n' &&
                           j < 255) {
                        name[j] = long_names[off + j];
                        j++;
                    }
                    name[j] = '\0';
                }
            } else {
                /* Short name: terminated by '/' or space-padded */
                int j = 0;
                while (j < 16 && hdr->ar_name[j] != '/' &&
                       hdr->ar_name[j] != ' ' && hdr->ar_name[j] != '\0') {
                    name[j] = hdr->ar_name[j];
                    j++;
                }
                name[j] = '\0';
            }

            /* Only process ELF objects */
            if (member_size >= 4 &&
                member_data[0] == ELFMAG0 && member_data[1] == ELFMAG1 &&
                member_data[2] == ELFMAG2 && member_data[3] == ELFMAG3) {
                /* Make a private copy so the object can be used independently */
                uint8_t *copy = malloc(member_size);
                if (!copy) {
                    fprintf(stderr, "anyld: out of memory\n");
                    free(ar_data);
                    return -1;
                }
                memcpy(copy, member_data, member_size);

                char member_name[512];
                snprintf(member_name, sizeof(member_name), "%s(%s)",
                         path, name);
                parse_object(ctx, member_name, copy, member_size, 1);
                (void)0;  /* count not needed */
            }
        }

        pos += member_size;
        if (pos & 1) pos++;  /* AR members are 2-byte aligned */
    }

    free(ar_data);  /* Safe: each member was copied */
    return 0;
}
