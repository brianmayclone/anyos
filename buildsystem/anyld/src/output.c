/*
 * output.c — Generate ELF64 ET_DYN shared object output.
 *
 * Layout:
 *   File Offset    Virtual Address     Content
 *   ──────────────────────────────────────────────────────
 *   0x0000         base+0x0000         ELF header + PHDRs
 *   0x00E8+        base+0x00E8+        .dynsym, .dynstr, .hash, .rela.dyn
 *   pad to 0x1000  base+0x1000         .text
 *   after text                         .rodata (16-byte aligned)
 *   pad to page    base+N*0x1000       .data
 *   after data                         .dynamic
 *                                      .bss (memsz only)
 *   after loaded   (not loaded)        Section Header Table
 *                                      .shstrtab
 */
#include "anyld.h"

/* ── Section indices in output ELF ──────────────────────────────────── */
#define SHIDX_NULL      0
#define SHIDX_TEXT      1
#define SHIDX_RODATA    2
#define SHIDX_DATA      3
#define SHIDX_BSS       4
#define SHIDX_DYNSYM    5
#define SHIDX_DYNSTR    6
#define SHIDX_HASH      7
#define SHIDX_RELADYN   8
#define SHIDX_DYNAMIC   9
#define SHIDX_SHSTRTAB  10
#define NUM_SECTIONS    11

#define NUM_PHDRS       3  /* PT_LOAD (RX), PT_LOAD (RW), PT_DYNAMIC */

/* ── Build .dynsym and .dynstr from exported symbols ────────────────── */

static void build_dynsym(Ctx *ctx, Buf *dynsym, Buf *dynstr,
                         int *out_nsyms) {
    buf_init(dynsym);
    buf_init(dynstr);

    /* .dynstr starts with empty string at offset 0 */
    uint8_t nul = 0;
    buf_append(dynstr, &nul, 1);

    /* .dynsym entry 0: NULL symbol */
    Elf64_Sym null_sym;
    memset(&null_sym, 0, sizeof(null_sym));
    buf_append(dynsym, &null_sym, sizeof(null_sym));
    int count = 1;

    /* Add SONAME string if library name is set */
    uint32_t soname_off = 0;
    if (ctx->lib_name && ctx->lib_name[0]) {
        soname_off = (uint32_t)dynstr->size;
        buf_append(dynstr, ctx->lib_name, strlen(ctx->lib_name) + 1);
    }

    /* Add all exported symbols */
    for (int i = 0; i < ctx->nsyms; i++) {
        Symbol *s = &ctx->syms[i];
        if (!s->is_export || !s->defined) continue;

        Elf64_Sym esym;
        esym.st_name  = (Elf64_Word)dynstr->size;
        esym.st_info  = ELF64_ST_INFO(STB_GLOBAL, s->type);
        esym.st_other = STV_DEFAULT;
        esym.st_value = s->value;
        esym.st_size  = s->size;

        /* Map output section → dynsym section index */
        switch (s->out_sec) {
            case SEC_TEXT:   esym.st_shndx = SHIDX_TEXT; break;
            case SEC_RODATA: esym.st_shndx = SHIDX_RODATA; break;
            case SEC_DATA:   esym.st_shndx = SHIDX_DATA; break;
            case SEC_BSS:    esym.st_shndx = SHIDX_BSS; break;
            default:         esym.st_shndx = SHN_ABS; break;
        }

        buf_append(dynsym, &esym, sizeof(esym));

        /* Add name to .dynstr */
        buf_append(dynstr, s->name, strlen(s->name) + 1);
        count++;
    }

    *out_nsyms = count;
    (void)soname_off;  /* Used later in build_dynamic */
}

/* ── Build .hash section (ELF hash table) ───────────────────────────── */

static void build_hash(Buf *hash_buf, Buf *dynsym_buf, Buf *dynstr_buf,
                       int nsyms) {
    buf_init(hash_buf);

    /* Choose nbuckets: a small prime near nsyms */
    uint32_t nbuckets = nsyms < 4 ? 3 : (uint32_t)(nsyms | 1);
    uint32_t nchain = (uint32_t)nsyms;

    uint32_t *buckets = calloc(nbuckets, sizeof(uint32_t));
    uint32_t *chains  = calloc(nchain, sizeof(uint32_t));

    /* Build hash chains */
    Elf64_Sym *syms = (Elf64_Sym *)dynsym_buf->data;
    char *strs = (char *)dynstr_buf->data;

    for (uint32_t i = 1; i < nchain; i++) {
        const char *name = strs + syms[i].st_name;
        uint32_t h = elf_hash(name) % nbuckets;
        chains[i] = buckets[h];
        buckets[h] = i;
    }

    /* Write: nbuckets, nchain, buckets[], chains[] */
    buf_append(hash_buf, &nbuckets, 4);
    buf_append(hash_buf, &nchain, 4);
    buf_append(hash_buf, buckets, nbuckets * 4);
    buf_append(hash_buf, chains, nchain * 4);

    free(buckets);
    free(chains);
}

/* ── Build .dynamic section ─────────────────────────────────────────── */

static void build_dynamic(Ctx *ctx, Buf *dyn_buf,
                          uint64_t dynsym_vaddr, uint64_t dynstr_vaddr,
                          uint64_t dynstr_size, uint64_t hash_vaddr,
                          uint64_t rela_vaddr, uint64_t rela_size,
                          int rela_count) {
    buf_init(dyn_buf);
    Elf64_Dyn d;

    /* DT_HASH */
    d.d_tag = DT_HASH;
    d.d_un.d_ptr = hash_vaddr;
    buf_append(dyn_buf, &d, sizeof(d));

    /* DT_STRTAB */
    d.d_tag = DT_STRTAB;
    d.d_un.d_ptr = dynstr_vaddr;
    buf_append(dyn_buf, &d, sizeof(d));

    /* DT_SYMTAB */
    d.d_tag = DT_SYMTAB;
    d.d_un.d_ptr = dynsym_vaddr;
    buf_append(dyn_buf, &d, sizeof(d));

    /* DT_STRSZ */
    d.d_tag = DT_STRSZ;
    d.d_un.d_val = dynstr_size;
    buf_append(dyn_buf, &d, sizeof(d));

    /* DT_SYMENT */
    d.d_tag = DT_SYMENT;
    d.d_un.d_val = sizeof(Elf64_Sym);
    buf_append(dyn_buf, &d, sizeof(d));

    /* DT_RELA / DT_RELASZ / DT_RELAENT / DT_RELACOUNT */
    if (rela_count > 0) {
        d.d_tag = DT_RELA;
        d.d_un.d_ptr = rela_vaddr;
        buf_append(dyn_buf, &d, sizeof(d));

        d.d_tag = DT_RELASZ;
        d.d_un.d_val = rela_size;
        buf_append(dyn_buf, &d, sizeof(d));

        d.d_tag = DT_RELAENT;
        d.d_un.d_val = sizeof(Elf64_Rela);
        buf_append(dyn_buf, &d, sizeof(d));

        d.d_tag = DT_RELACOUNT;
        d.d_un.d_val = (uint64_t)rela_count;
        buf_append(dyn_buf, &d, sizeof(d));
    }

    /* DT_SONAME (if library name set) */
    if (ctx->lib_name && ctx->lib_name[0]) {
        /* SONAME offset in dynstr: right after the empty string byte.
         * We added it as the first string in build_dynsym. */
        d.d_tag = DT_SONAME;
        d.d_un.d_val = 1;  /* offset 1 in .dynstr */
        buf_append(dyn_buf, &d, sizeof(d));
    }

    /* DT_NULL (terminator) */
    d.d_tag = DT_NULL;
    d.d_un.d_val = 0;
    buf_append(dyn_buf, &d, sizeof(d));
}

/* ── Build .shstrtab (section name string table) ────────────────────── */

typedef struct {
    uint32_t null_off;
    uint32_t text_off;
    uint32_t rodata_off;
    uint32_t data_off;
    uint32_t bss_off;
    uint32_t dynsym_off;
    uint32_t dynstr_off;
    uint32_t hash_off;
    uint32_t reladyn_off;
    uint32_t dynamic_off;
    uint32_t shstrtab_off;
} ShstrOffsets;

static void build_shstrtab(Buf *shstrtab, ShstrOffsets *off) {
    buf_init(shstrtab);

    /* NUL byte at offset 0 */
    uint8_t nul = 0;
    buf_append(shstrtab, &nul, 1);
    off->null_off = 0;

    #define ADD_NAME(field, str) do { \
        off->field = (uint32_t)shstrtab->size; \
        buf_append(shstrtab, str, strlen(str) + 1); \
    } while (0)

    ADD_NAME(text_off,     ".text");
    ADD_NAME(rodata_off,   ".rodata");
    ADD_NAME(data_off,     ".data");
    ADD_NAME(bss_off,      ".bss");
    ADD_NAME(dynsym_off,   ".dynsym");
    ADD_NAME(dynstr_off,   ".dynstr");
    ADD_NAME(hash_off,     ".hash");
    ADD_NAME(reladyn_off,  ".rela.dyn");
    ADD_NAME(dynamic_off,  ".dynamic");
    ADD_NAME(shstrtab_off, ".shstrtab");

    #undef ADD_NAME
}

/* ── Compute section virtual addresses (must be called before relocs) ── */

int compute_layout(Ctx *ctx) {
    uint64_t base = ctx->base_addr;

    /*
     * Metadata region: offset 0x0000 → just before .text
     * [ELF header][PHDRs][.dynsym][.dynstr][.hash][.rela.dyn][pad to page]
     *
     * Build temporary dynsym/dynstr/hash just to determine sizes
     * (symbol values don't matter — only entry count affects size).
     */
    uint64_t meta_off = sizeof(Elf64_Ehdr) + NUM_PHDRS * sizeof(Elf64_Phdr);

    Buf tmp_dynsym, tmp_dynstr, tmp_hash;
    int tmp_count;
    build_dynsym(ctx, &tmp_dynsym, &tmp_dynstr, &tmp_count);
    build_hash(&tmp_hash, &tmp_dynsym, &tmp_dynstr, tmp_count);

    uint64_t dynsym_off = (meta_off + 7) & ~7ULL;
    uint64_t dynstr_off = dynsym_off + tmp_dynsym.size;
    uint64_t hash_off   = (dynstr_off + tmp_dynstr.size + 3) & ~3ULL;
    uint64_t rela_off   = (hash_off + tmp_hash.size + 7) & ~7ULL;
    uint64_t meta_end   = rela_off + ctx->rela_dyn.size;

    buf_free(&tmp_dynsym);
    buf_free(&tmp_dynstr);
    buf_free(&tmp_hash);

    /* .text starts at next page */
    uint64_t text_off = PAGE_ALIGN(meta_end);
    ctx->text_vaddr   = base + text_off;

    /* .rodata follows .text, 16-byte aligned */
    uint64_t rodata_off = text_off + ctx->text.size;
    rodata_off = (rodata_off + 15) & ~15ULL;
    ctx->rodata_vaddr = base + rodata_off;

    /* .data starts at next page after .rodata */
    uint64_t rx_end   = rodata_off + ctx->rodata.size;
    uint64_t data_off = PAGE_ALIGN(rx_end);
    ctx->data_vaddr   = base + data_off;

    /* .dynamic follows .data, 8-byte aligned */
    uint64_t dyn_off  = data_off + ctx->data.size;
    dyn_off = (dyn_off + 7) & ~7ULL;
    ctx->dynamic_vaddr = base + dyn_off;

    /* Estimate .dynamic size (11 entries * 16 bytes = 176 max) */
    uint64_t dyn_size_est = 11 * sizeof(Elf64_Dyn);
    uint64_t rw_file_end  = dyn_off + dyn_size_est;

    /* .bss follows at next page boundary */
    uint64_t bss_off_virt = PAGE_ALIGN(rw_file_end);
    ctx->bss_vaddr = base + bss_off_virt;

    return 0;
}

/* ── Write the complete ELF64 output file ───────────────────────────── */

int write_output(Ctx *ctx) {
    /* ─── Phase 1: Layout the file ──────────────────────────────────── */

    uint64_t base = ctx->base_addr;

    /*
     * Metadata region: offset 0x0000 → just before .text
     * [ELF header][PHDRs][.dynsym][.dynstr][.hash][.rela.dyn][pad to page]
     */
    uint64_t meta_off = sizeof(Elf64_Ehdr) + NUM_PHDRS * sizeof(Elf64_Phdr);

    /* Build export tables (need sizes for layout) */
    Buf dynsym_buf, dynstr_buf, hash_buf, dyn_buf, shstrtab_buf;
    int dynsym_count;

    /* Build dynsym/dynstr/hash with final symbol values */
    build_dynsym(ctx, &dynsym_buf, &dynstr_buf, &dynsym_count);
    build_hash(&hash_buf, &dynsym_buf, &dynstr_buf, dynsym_count);

    /* Metadata layout (all within page 0) */
    uint64_t dynsym_off  = (meta_off + 7) & ~7ULL;  /* 8-byte align */
    uint64_t dynstr_off  = dynsym_off + dynsym_buf.size;
    uint64_t hash_off    = (dynstr_off + dynstr_buf.size + 3) & ~3ULL;
    uint64_t reladyn_off = (hash_off + hash_buf.size + 7) & ~7ULL;
    uint64_t meta_end    = reladyn_off + ctx->rela_dyn.size;

    /* .text starts at next page */
    uint64_t text_off = PAGE_ALIGN(meta_end);

    /* Recompute layout (must match compute_layout) */
    ctx->text_vaddr   = base + text_off;
    uint64_t rodata_off = text_off + ctx->text.size;
    rodata_off = (rodata_off + 15) & ~15ULL;  /* 16-byte align */
    ctx->rodata_vaddr = base + rodata_off;

    /* RX segment ends after .rodata, pad to page */
    uint64_t rx_end    = rodata_off + ctx->rodata.size;
    uint64_t data_off  = PAGE_ALIGN(rx_end);

    /* Virtual addresses for data */
    ctx->data_vaddr    = base + data_off;
    uint64_t dyn_off   = data_off + ctx->data.size;
    dyn_off = (dyn_off + 7) & ~7ULL;  /* 8-byte align */

    /* Now build .dynamic with correct vaddrs */
    build_dynamic(ctx, &dyn_buf,
                  base + dynsym_off,
                  base + dynstr_off,
                  dynstr_buf.size,
                  base + hash_off,
                  base + reladyn_off,
                  ctx->rela_dyn.size,
                  ctx->nrela_dyn);

    ctx->dynamic_vaddr = base + dyn_off;
    uint64_t rw_file_end = dyn_off + dyn_buf.size;

    /* BSS follows .dynamic in virtual space */
    uint64_t bss_off_virt = PAGE_ALIGN(rw_file_end);
    ctx->bss_vaddr = base + bss_off_virt;

    build_dynsym(ctx, &dynsym_buf, &dynstr_buf, &dynsym_count);
    build_hash(&hash_buf, &dynsym_buf, &dynstr_buf, dynsym_count);
    build_dynamic(ctx, &dyn_buf,
                  base + dynsym_off,
                  base + dynstr_off,
                  dynstr_buf.size,
                  base + hash_off,
                  base + reladyn_off,
                  ctx->rela_dyn.size,
                  ctx->nrela_dyn);

    /* Section headers and .shstrtab go after all loaded data */
    ShstrOffsets shstr_off;
    build_shstrtab(&shstrtab_buf, &shstr_off);

    uint64_t sht_off = (rw_file_end + 7) & ~7ULL;
    uint64_t shstrtab_file_off = sht_off + NUM_SECTIONS * sizeof(Elf64_Shdr);

    /* ─── Phase 2: Build and write the ELF file ─────────────────────── */

    FILE *fp = fopen(ctx->output_path, "wb");
    if (!fp) {
        fprintf(stderr, "anyld: cannot create '%s'\n", ctx->output_path);
        goto err;
    }

    /* Helper: write zeros up to a target file offset */
    #define PAD_TO(target) do { \
        long cur = ftell(fp); \
        if (cur < 0) goto err; \
        if ((uint64_t)cur < (target)) { \
            size_t pad = (size_t)((target) - (uint64_t)cur); \
            uint8_t *z = calloc(1, pad); \
            fwrite(z, 1, pad, fp); \
            free(z); \
        } \
    } while (0)

    /* ── ELF Header ─────────────────────────────────────────────────── */
    Elf64_Ehdr ehdr;
    memset(&ehdr, 0, sizeof(ehdr));
    ehdr.e_ident[0] = ELFMAG0;
    ehdr.e_ident[1] = ELFMAG1;
    ehdr.e_ident[2] = ELFMAG2;
    ehdr.e_ident[3] = ELFMAG3;
    ehdr.e_ident[4] = ELFCLASS64;
    ehdr.e_ident[5] = ELFDATA2LSB;
    ehdr.e_ident[6] = EV_CURRENT;
    ehdr.e_ident[7] = ELFOSABI_NONE;
    ehdr.e_type      = ET_DYN;
    ehdr.e_machine   = EM_X86_64;
    ehdr.e_version   = EV_CURRENT;
    ehdr.e_entry     = 0;  /* No entry point for a shared library */
    ehdr.e_phoff     = sizeof(Elf64_Ehdr);
    ehdr.e_shoff     = sht_off;
    ehdr.e_flags     = 0;
    ehdr.e_ehsize    = sizeof(Elf64_Ehdr);
    ehdr.e_phentsize = sizeof(Elf64_Phdr);
    ehdr.e_phnum     = NUM_PHDRS;
    ehdr.e_shentsize = sizeof(Elf64_Shdr);
    ehdr.e_shnum     = NUM_SECTIONS;
    ehdr.e_shstrndx  = SHIDX_SHSTRTAB;
    fwrite(&ehdr, sizeof(ehdr), 1, fp);

    /* ── Program Headers ────────────────────────────────────────────── */

    /* PT_LOAD #1: RX (metadata page + .text + .rodata) */
    Elf64_Phdr ph;
    memset(&ph, 0, sizeof(ph));
    ph.p_type   = PT_LOAD;
    ph.p_flags  = PF_R | PF_X;
    ph.p_offset = 0;
    ph.p_vaddr  = base;
    ph.p_paddr  = base;
    ph.p_filesz = rx_end;
    ph.p_memsz  = rx_end;
    ph.p_align  = PAGE_SIZE;
    fwrite(&ph, sizeof(ph), 1, fp);

    /* PT_LOAD #2: RW (.data + .dynamic + .bss) */
    memset(&ph, 0, sizeof(ph));
    ph.p_type   = PT_LOAD;
    ph.p_flags  = PF_R | PF_W;
    ph.p_offset = data_off;
    ph.p_vaddr  = base + data_off;
    ph.p_paddr  = base + data_off;
    ph.p_filesz = rw_file_end - data_off;
    ph.p_memsz  = (bss_off_virt + ctx->bss_size) - data_off;
    ph.p_align  = PAGE_SIZE;
    fwrite(&ph, sizeof(ph), 1, fp);

    /* PT_DYNAMIC */
    memset(&ph, 0, sizeof(ph));
    ph.p_type   = PT_DYNAMIC;
    ph.p_flags  = PF_R | PF_W;
    ph.p_offset = dyn_off;
    ph.p_vaddr  = base + dyn_off;
    ph.p_paddr  = base + dyn_off;
    ph.p_filesz = dyn_buf.size;
    ph.p_memsz  = dyn_buf.size;
    ph.p_align  = 8;
    fwrite(&ph, sizeof(ph), 1, fp);

    /* ── Metadata sections (in page 0) ──────────────────────────────── */

    PAD_TO(dynsym_off);
    fwrite(dynsym_buf.data, 1, dynsym_buf.size, fp);

    PAD_TO(dynstr_off);
    fwrite(dynstr_buf.data, 1, dynstr_buf.size, fp);

    PAD_TO(hash_off);
    fwrite(hash_buf.data, 1, hash_buf.size, fp);

    /* .rela.dyn */
    PAD_TO(reladyn_off);
    if (ctx->rela_dyn.size > 0)
        fwrite(ctx->rela_dyn.data, 1, ctx->rela_dyn.size, fp);

    /* ── .text ──────────────────────────────────────────────────────── */

    PAD_TO(text_off);
    if (ctx->text.size > 0)
        fwrite(ctx->text.data, 1, ctx->text.size, fp);

    /* ── .rodata ────────────────────────────────────────────────────── */

    PAD_TO(rodata_off);
    if (ctx->rodata.size > 0)
        fwrite(ctx->rodata.data, 1, ctx->rodata.size, fp);

    /* ── .data ──────────────────────────────────────────────────────── */

    PAD_TO(data_off);
    if (ctx->data.size > 0)
        fwrite(ctx->data.data, 1, ctx->data.size, fp);

    /* ── .dynamic ───────────────────────────────────────────────────── */

    PAD_TO(dyn_off);
    fwrite(dyn_buf.data, 1, dyn_buf.size, fp);

    /* ── Section Header Table (not loaded) ──────────────────────────── */

    PAD_TO(sht_off);

    Elf64_Shdr shdr;

    /* Section 0: NULL */
    memset(&shdr, 0, sizeof(shdr));
    fwrite(&shdr, sizeof(shdr), 1, fp);

    /* Section 1: .text */
    memset(&shdr, 0, sizeof(shdr));
    shdr.sh_name      = shstr_off.text_off;
    shdr.sh_type      = SHT_PROGBITS;
    shdr.sh_flags     = SHF_ALLOC | SHF_EXECINSTR;
    shdr.sh_addr      = ctx->text_vaddr;
    shdr.sh_offset    = text_off;
    shdr.sh_size      = ctx->text.size;
    shdr.sh_addralign = 16;
    fwrite(&shdr, sizeof(shdr), 1, fp);

    /* Section 2: .rodata */
    memset(&shdr, 0, sizeof(shdr));
    shdr.sh_name      = shstr_off.rodata_off;
    shdr.sh_type      = SHT_PROGBITS;
    shdr.sh_flags     = SHF_ALLOC;
    shdr.sh_addr      = ctx->rodata_vaddr;
    shdr.sh_offset    = rodata_off;
    shdr.sh_size      = ctx->rodata.size;
    shdr.sh_addralign = 16;
    fwrite(&shdr, sizeof(shdr), 1, fp);

    /* Section 3: .data */
    memset(&shdr, 0, sizeof(shdr));
    shdr.sh_name      = shstr_off.data_off;
    shdr.sh_type      = SHT_PROGBITS;
    shdr.sh_flags     = SHF_ALLOC | SHF_WRITE;
    shdr.sh_addr      = ctx->data_vaddr;
    shdr.sh_offset    = data_off;
    shdr.sh_size      = ctx->data.size;
    shdr.sh_addralign = 8;
    fwrite(&shdr, sizeof(shdr), 1, fp);

    /* Section 4: .bss */
    memset(&shdr, 0, sizeof(shdr));
    shdr.sh_name      = shstr_off.bss_off;
    shdr.sh_type      = SHT_NOBITS;
    shdr.sh_flags     = SHF_ALLOC | SHF_WRITE;
    shdr.sh_addr      = ctx->bss_vaddr;
    shdr.sh_offset    = rw_file_end;  /* No file data */
    shdr.sh_size      = ctx->bss_size;
    shdr.sh_addralign = ctx->bss_align > 0 ? ctx->bss_align : 8;
    fwrite(&shdr, sizeof(shdr), 1, fp);

    /* Section 5: .dynsym */
    memset(&shdr, 0, sizeof(shdr));
    shdr.sh_name      = shstr_off.dynsym_off;
    shdr.sh_type      = SHT_DYNSYM;
    shdr.sh_flags     = SHF_ALLOC;
    shdr.sh_addr      = base + dynsym_off;
    shdr.sh_offset    = dynsym_off;
    shdr.sh_size      = dynsym_buf.size;
    shdr.sh_link      = SHIDX_DYNSTR;  /* Associated string table */
    shdr.sh_info      = 1;             /* First non-local symbol */
    shdr.sh_addralign = 8;
    shdr.sh_entsize   = sizeof(Elf64_Sym);
    fwrite(&shdr, sizeof(shdr), 1, fp);

    /* Section 6: .dynstr */
    memset(&shdr, 0, sizeof(shdr));
    shdr.sh_name      = shstr_off.dynstr_off;
    shdr.sh_type      = SHT_STRTAB;
    shdr.sh_flags     = SHF_ALLOC;
    shdr.sh_addr      = base + dynstr_off;
    shdr.sh_offset    = dynstr_off;
    shdr.sh_size      = dynstr_buf.size;
    shdr.sh_addralign = 1;
    fwrite(&shdr, sizeof(shdr), 1, fp);

    /* Section 7: .hash */
    memset(&shdr, 0, sizeof(shdr));
    shdr.sh_name      = shstr_off.hash_off;
    shdr.sh_type      = SHT_HASH;
    shdr.sh_flags     = SHF_ALLOC;
    shdr.sh_addr      = base + hash_off;
    shdr.sh_offset    = hash_off;
    shdr.sh_size      = hash_buf.size;
    shdr.sh_link      = SHIDX_DYNSYM;
    shdr.sh_addralign = 4;
    shdr.sh_entsize   = 4;
    fwrite(&shdr, sizeof(shdr), 1, fp);

    /* Section 8: .rela.dyn */
    memset(&shdr, 0, sizeof(shdr));
    shdr.sh_name      = shstr_off.reladyn_off;
    shdr.sh_type      = SHT_RELA;
    shdr.sh_flags     = SHF_ALLOC;
    shdr.sh_addr      = base + reladyn_off;
    shdr.sh_offset    = reladyn_off;
    shdr.sh_size      = ctx->rela_dyn.size;
    shdr.sh_link      = SHIDX_DYNSYM;
    shdr.sh_addralign = 8;
    shdr.sh_entsize   = sizeof(Elf64_Rela);
    fwrite(&shdr, sizeof(shdr), 1, fp);

    /* Section 9: .dynamic */
    memset(&shdr, 0, sizeof(shdr));
    shdr.sh_name      = shstr_off.dynamic_off;
    shdr.sh_type      = SHT_DYNAMIC;
    shdr.sh_flags     = SHF_ALLOC | SHF_WRITE;
    shdr.sh_addr      = base + dyn_off;
    shdr.sh_offset    = dyn_off;
    shdr.sh_size      = dyn_buf.size;
    shdr.sh_link      = SHIDX_DYNSTR;
    shdr.sh_addralign = 8;
    shdr.sh_entsize   = sizeof(Elf64_Dyn);
    fwrite(&shdr, sizeof(shdr), 1, fp);

    /* Section 10: .shstrtab */
    memset(&shdr, 0, sizeof(shdr));
    shdr.sh_name      = shstr_off.shstrtab_off;
    shdr.sh_type      = SHT_STRTAB;
    shdr.sh_flags     = 0;  /* Not loaded */
    shdr.sh_addr      = 0;
    shdr.sh_offset    = shstrtab_file_off;
    shdr.sh_size      = shstrtab_buf.size;
    shdr.sh_addralign = 1;
    fwrite(&shdr, sizeof(shdr), 1, fp);

    /* ── .shstrtab content ──────────────────────────────────────────── */

    fwrite(shstrtab_buf.data, 1, shstrtab_buf.size, fp);

    /* ── Done ───────────────────────────────────────────────────────── */

    fclose(fp);

    /* Statistics */
    if (!ctx->quiet) {
        printf("anyld: '%s' created\n", ctx->output_path);
        printf("  base:     0x%llx\n", (unsigned long long)base);
        printf("  .text:    %zu bytes at 0x%llx\n",
               ctx->text.size, (unsigned long long)ctx->text_vaddr);
        printf("  .rodata:  %zu bytes at 0x%llx\n",
               ctx->rodata.size, (unsigned long long)ctx->rodata_vaddr);
        printf("  .data:    %zu bytes at 0x%llx\n",
               ctx->data.size, (unsigned long long)ctx->data_vaddr);
        printf("  .bss:     %llu bytes at 0x%llx\n",
               (unsigned long long)ctx->bss_size,
               (unsigned long long)ctx->bss_vaddr);
        printf("  exports:  %d symbols\n", dynsym_count - 1);
        if (ctx->nrela_dyn > 0)
            printf("  relocs:   %d R_X86_64_RELATIVE entries\n", ctx->nrela_dyn);
    }

    #undef PAD_TO

    buf_free(&dynsym_buf);
    buf_free(&dynstr_buf);
    buf_free(&hash_buf);
    buf_free(&dyn_buf);
    buf_free(&shstrtab_buf);
    return 0;

err:
    buf_free(&dynsym_buf);
    buf_free(&dynstr_buf);
    buf_free(&hash_buf);
    buf_free(&dyn_buf);
    buf_free(&shstrtab_buf);
    return -1;
}
