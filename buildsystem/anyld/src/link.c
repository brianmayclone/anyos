/*
 * link.c — Section merging, symbol resolution, relocation application.
 *
 * This is the core of the linker: takes parsed .o files and produces
 * merged, relocated output sections ready for ELF generation.
 */
#include "anyld.h"

/* ── Classify a section name into an output section ─────────────────── */

int classify_section(const char *name, uint64_t flags) {
    if (!(flags & SHF_ALLOC))
        return SEC_NONE;

    /* Discard debug and unwind info */
    if (strcmp(name, ".eh_frame") == 0) return SEC_NONE;
    if (strcmp(name, ".eh_frame_hdr") == 0) return SEC_NONE;
    if (strncmp(name, ".debug", 6) == 0) return SEC_NONE;
    if (strncmp(name, ".note", 5) == 0) return SEC_NONE;
    if (strcmp(name, ".comment") == 0) return SEC_NONE;
    if (strcmp(name, ".group") == 0) return SEC_NONE;

    /* Code sections */
    if (strncmp(name, ".text", 5) == 0)
        return SEC_TEXT;
    if (strncmp(name, ".init", 5) == 0 && !(flags & SHF_WRITE))
        return SEC_TEXT;

    /* Read-only data */
    if (strncmp(name, ".rodata", 7) == 0)
        return SEC_RODATA;
    if (strncmp(name, ".data.rel.ro", 12) == 0)
        return SEC_RODATA;

    /* Writable data */
    if (strncmp(name, ".data", 5) == 0)
        return SEC_DATA;
    if (strncmp(name, ".init_array", 11) == 0)
        return SEC_DATA;
    if (strncmp(name, ".fini_array", 11) == 0)
        return SEC_DATA;
    if (strncmp(name, ".got", 4) == 0)
        return SEC_DATA;
    if (strncmp(name, ".tdata", 6) == 0)
        return SEC_DATA;  /* TLS data — treat as regular data for now */

    /* BSS */
    if (strncmp(name, ".bss", 4) == 0)
        return SEC_BSS;
    if (strncmp(name, ".tbss", 5) == 0)
        return SEC_BSS;

    /* Unknown allocated section: classify by flags */
    if (flags & SHF_EXECINSTR)
        return SEC_TEXT;
    if (flags & SHF_WRITE)
        return SEC_DATA;
    return SEC_RODATA;
}

/* ── Merge all input sections into output buffers ───────────────────── */

int merge_sections(Ctx *ctx) {
    buf_init(&ctx->text);
    buf_init(&ctx->rodata);
    buf_init(&ctx->data);
    ctx->bss_size = 0;
    ctx->bss_align = 1;

    for (int i = 0; i < ctx->nobjs; i++) {
        InputObj *obj = &ctx->objs[i];

        for (uint16_t j = 0; j < obj->nshdr; j++) {
            Elf64_Shdr *sh = &obj->shdrs[j];

            /* Only merge PROGBITS and NOBITS sections */
            if (sh->sh_type != SHT_PROGBITS && sh->sh_type != SHT_NOBITS)
                continue;

            const char *name = obj->shstrtab
                               ? obj->shstrtab + sh->sh_name : "";
            int sec = classify_section(name, sh->sh_flags);

            if (sec == SEC_NONE) {
                obj->sec_map[j].out_sec = SEC_NONE;
                continue;
            }

            uint64_t align = sh->sh_addralign > 1 ? sh->sh_addralign : 1;

            if (sec == SEC_BSS || sh->sh_type == SHT_NOBITS) {
                /* BSS: no file data, just reserve space */
                uint64_t aligned_bss =
                    (ctx->bss_size + align - 1) & ~(align - 1);
                obj->sec_map[j].out_sec = SEC_BSS;
                obj->sec_map[j].out_off = aligned_bss;
                ctx->bss_size = aligned_bss + sh->sh_size;
                if (align > ctx->bss_align) ctx->bss_align = (uint32_t)align;
            } else {
                Buf *target;
                switch (sec) {
                    case SEC_TEXT:   target = &ctx->text; break;
                    case SEC_RODATA: target = &ctx->rodata; break;
                    case SEC_DATA:   target = &ctx->data; break;
                    default:         continue;
                }

                buf_align(target, (size_t)align);
                obj->sec_map[j].out_sec = sec;
                obj->sec_map[j].out_off = target->size;
                buf_append(target, obj->data + sh->sh_offset,
                           (size_t)sh->sh_size);
            }
        }
    }

    return 0;
}

/* ── Collect all symbols from all objects into global table ──────────── */

int collect_symbols(Ctx *ctx) {
    for (int i = 0; i < ctx->nobjs; i++) {
        InputObj *obj = &ctx->objs[i];

        for (uint32_t j = 0; j < obj->nsym; j++) {
            Elf64_Sym *sym = &obj->symtab[j];
            const char *name = obj->strtab
                               ? obj->strtab + sym->st_name : "";
            uint8_t bind = ELF64_ST_BIND(sym->st_info);
            uint8_t type = ELF64_ST_TYPE(sym->st_info);

            /* Skip NULL symbol (index 0) */
            if (j == 0) {
                obj->sym_map[j] = 0;  /* Will be handled specially */
                continue;
            }

            /* Skip FILE and SECTION symbols — not needed for linking */
            if (type == STT_FILE) {
                obj->sym_map[j] = 0;
                continue;
            }

            int defined = (sym->st_shndx != SHN_UNDEF &&
                           sym->st_shndx != SHN_COMMON);
            int is_abs  = (sym->st_shndx == SHN_ABS);

            /* Determine output section and offset */
            int out_sec = SEC_NONE;
            uint64_t sec_off = sym->st_value;

            if (defined && !is_abs && sym->st_shndx < obj->nshdr) {
                /* Symbol is in a specific section */
                out_sec = obj->sec_map[sym->st_shndx].out_sec;
                sec_off = sym->st_value;  /* Offset within input section */
            }

            /* Section symbols: represent the section itself */
            if (type == STT_SECTION) {
                if (sym->st_shndx < obj->nshdr) {
                    int gsym = add_global_sym(ctx, name, bind, type,
                                              defined, i, sym->st_shndx,
                                              0, 0);
                    if (gsym >= 0) {
                        ctx->syms[gsym].out_sec = out_sec;
                    }
                    obj->sym_map[j] = (uint32_t)gsym;
                } else {
                    obj->sym_map[j] = 0;
                }
                continue;
            }

            /* LOCAL symbols: always add (no conflict check) */
            if (bind == STB_LOCAL) {
                int gsym = add_global_sym(ctx, name, bind, type,
                                          defined, i, sym->st_shndx,
                                          sec_off, sym->st_size);
                if (gsym >= 0 && defined && !is_abs) {
                    ctx->syms[gsym].out_sec = out_sec;
                }
                obj->sym_map[j] = gsym >= 0 ? (uint32_t)gsym : 0;
                continue;
            }

            /* GLOBAL / WEAK: check for existing definition */
            int existing = find_global_sym(ctx, name);

            if (existing >= 0) {
                Symbol *es = &ctx->syms[existing];
                if (defined) {
                    if (es->defined && bind == STB_GLOBAL &&
                        es->bind == STB_GLOBAL) {
                        fprintf(stderr,
                                "anyld: duplicate symbol '%s'\n"
                                "  defined in: %s\n"
                                "  also in:    %s\n",
                                name,
                                ctx->objs[es->obj_idx].filename,
                                obj->filename);
                        return -1;
                    }
                    /* New definition wins if: old is weak, or old is undef */
                    if (!es->defined || es->bind == STB_WEAK) {
                        es->defined = 1;
                        es->bind = bind;
                        es->type = type;
                        es->obj_idx = i;
                        es->sec_idx = sym->st_shndx;
                        es->sec_off = sec_off;
                        es->size = sym->st_size;
                        es->out_sec = out_sec;
                    }
                }
                obj->sym_map[j] = (uint32_t)existing;
            } else {
                /* New symbol */
                int gsym = add_global_sym(ctx, name, bind, type,
                                          defined, i, sym->st_shndx,
                                          sec_off, sym->st_size);
                if (gsym >= 0 && defined && !is_abs) {
                    ctx->syms[gsym].out_sec = out_sec;
                }
                obj->sym_map[j] = gsym >= 0 ? (uint32_t)gsym : 0;
            }
        }
    }

    return 0;
}

/* ── Verify all undefined symbols are resolved ──────────────────────── */

int resolve_symbols(Ctx *ctx) {
    int errors = 0;
    for (int i = 0; i < ctx->nsyms; i++) {
        Symbol *s = &ctx->syms[i];
        if (!s->defined && s->bind == STB_GLOBAL &&
            s->name[0] != '\0') {
            fprintf(stderr, "anyld: undefined symbol '%s'\n", s->name);
            errors++;
        }
        /* Weak undefined symbols are OK — they resolve to 0 */
    }
    return errors > 0 ? -1 : 0;
}

/* ── Collect relocations from all objects ───────────────────────────── */

static int collect_relocs(Ctx *ctx) {
    for (int i = 0; i < ctx->nobjs; i++) {
        InputObj *obj = &ctx->objs[i];

        for (uint16_t j = 0; j < obj->nshdr; j++) {
            Elf64_Shdr *sh = &obj->shdrs[j];
            if (sh->sh_type != SHT_RELA) continue;

            /* sh_info = index of section being relocated */
            uint32_t target_shndx = sh->sh_info;
            if (target_shndx >= obj->nshdr) continue;

            /* Check if target section was merged */
            int out_sec = obj->sec_map[target_shndx].out_sec;
            uint64_t sec_base = obj->sec_map[target_shndx].out_off;
            if (out_sec == SEC_NONE) continue;

            /* Process each relocation entry */
            uint32_t nrela = (uint32_t)(sh->sh_size / sizeof(Elf64_Rela));
            Elf64_Rela *relas =
                (Elf64_Rela *)(obj->data + sh->sh_offset);

            for (uint32_t k = 0; k < nrela; k++) {
                Elf64_Rela *rela = &relas[k];
                uint32_t sym_idx = (uint32_t)ELF64_R_SYM(rela->r_info);
                uint32_t rtype   = (uint32_t)ELF64_R_TYPE(rela->r_info);

                if (rtype == R_X86_64_NONE) continue;

                /* Map local sym index → global sym index */
                uint32_t gsym = 0;
                if (sym_idx < obj->nsym) {
                    gsym = obj->sym_map[sym_idx];
                }

                /* Grow relocs array */
                if (ctx->nrelocs >= ctx->relocs_cap) {
                    ctx->relocs_cap = ctx->relocs_cap
                                      ? ctx->relocs_cap * 2 : 4096;
                    ctx->relocs = realloc(ctx->relocs,
                                          ctx->relocs_cap * sizeof(Reloc));
                }

                Reloc *r = &ctx->relocs[ctx->nrelocs++];
                r->out_sec  = out_sec;
                r->offset   = sec_base + rela->r_offset;
                r->type     = rtype;
                r->addend   = rela->r_addend;
                r->sym_idx  = gsym;
            }
        }
    }
    return 0;
}

/* ── Compute final symbol virtual addresses ─────────────────────────── */

static void finalize_symbol_values(Ctx *ctx) {
    for (int i = 0; i < ctx->nsyms; i++) {
        Symbol *s = &ctx->syms[i];
        if (!s->defined) {
            s->value = 0;  /* Weak undefined → 0 */
            continue;
        }

        /* Section symbols: value = section base vaddr */
        if (ELF64_ST_TYPE(ELF64_ST_INFO(s->bind, s->type)) == STT_SECTION ||
            s->type == STT_SECTION) {
            switch (s->out_sec) {
                case SEC_TEXT:   s->value = ctx->text_vaddr; break;
                case SEC_RODATA: s->value = ctx->rodata_vaddr; break;
                case SEC_DATA:   s->value = ctx->data_vaddr; break;
                case SEC_BSS:    s->value = ctx->bss_vaddr; break;
                default:         s->value = 0; break;
            }
            /* Add the section's output offset from the input object */
            if (s->obj_idx >= 0 && s->obj_idx < ctx->nobjs) {
                InputObj *obj = &ctx->objs[s->obj_idx];
                if (s->sec_idx < obj->nshdr) {
                    s->value += obj->sec_map[s->sec_idx].out_off;
                }
            }
            continue;
        }

        /* Regular symbols: section_vaddr + section_output_offset + sym_offset */
        uint64_t base_vaddr;
        uint64_t merged_off = 0;

        switch (s->out_sec) {
            case SEC_TEXT:   base_vaddr = ctx->text_vaddr; break;
            case SEC_RODATA: base_vaddr = ctx->rodata_vaddr; break;
            case SEC_DATA:   base_vaddr = ctx->data_vaddr; break;
            case SEC_BSS:    base_vaddr = ctx->bss_vaddr; break;
            default:
                /* ABS symbol or unknown section */
                s->value = s->sec_off;
                continue;
        }

        /* Find the merged offset of the symbol's input section */
        if (s->obj_idx >= 0 && s->obj_idx < ctx->nobjs) {
            InputObj *obj = &ctx->objs[s->obj_idx];
            if (s->sec_idx < obj->nshdr) {
                merged_off = obj->sec_map[s->sec_idx].out_off;
            }
        }

        s->value = base_vaddr + merged_off + s->sec_off;
    }
}

/* ── Apply all collected relocations to output section buffers ──────── */

static int apply_relocs(Ctx *ctx) {
    int errors = 0;

    for (int i = 0; i < ctx->nrelocs; i++) {
        Reloc *r = &ctx->relocs[i];

        /* Symbol value (S) */
        uint64_t S = 0;
        if (r->sym_idx < (uint32_t)ctx->nsyms) {
            S = ctx->syms[r->sym_idx].value;
        }
        int64_t A = r->addend;

        /* Patch location */
        uint8_t *patch;
        uint64_t P;  /* Virtual address of patch location */

        switch (r->out_sec) {
            case SEC_TEXT:
                if (r->offset >= ctx->text.size) goto bounds_err;
                patch = ctx->text.data + r->offset;
                P = ctx->text_vaddr + r->offset;
                break;
            case SEC_RODATA:
                if (r->offset >= ctx->rodata.size) goto bounds_err;
                patch = ctx->rodata.data + r->offset;
                P = ctx->rodata_vaddr + r->offset;
                break;
            case SEC_DATA:
                if (r->offset >= ctx->data.size) goto bounds_err;
                patch = ctx->data.data + r->offset;
                P = ctx->data_vaddr + r->offset;
                break;
            default:
                continue;
        }

        switch (r->type) {
            case R_X86_64_64:
                /* S + A (absolute 64-bit) */
                *(uint64_t *)patch = (uint64_t)((int64_t)S + A);
                /* Record runtime relocation for dynamic loading */
                {
                    Elf64_Rela rr;
                    rr.r_offset = P;
                    rr.r_info   = ELF64_R_INFO(0, R_X86_64_RELATIVE);
                    rr.r_addend = *(int64_t *)patch;
                    buf_append(&ctx->rela_dyn, &rr, sizeof(rr));
                    ctx->nrela_dyn++;
                }
                break;

            case R_X86_64_PC32:
            case R_X86_64_PLT32:
                /* S + A - P (PC-relative 32-bit) */
                {
                    int64_t val = (int64_t)S + A - (int64_t)P;
                    if (val < -2147483648LL || val > 2147483647LL) {
                        const char *sname = r->sym_idx < (uint32_t)ctx->nsyms
                            ? ctx->syms[r->sym_idx].name : "?";
                        fprintf(stderr,
                                "anyld: PC32 relocation overflow for '%s' "
                                "(value=0x%llx)\n",
                                sname, (unsigned long long)val);
                        errors++;
                    }
                    *(int32_t *)patch = (int32_t)val;
                }
                break;

            case R_X86_64_32:
                /* S + A (zero-extend to 32-bit) */
                {
                    uint64_t val = (uint64_t)((int64_t)S + A);
                    if (val > 0xFFFFFFFF) {
                        const char *sname = r->sym_idx < (uint32_t)ctx->nsyms
                            ? ctx->syms[r->sym_idx].name : "?";
                        fprintf(stderr,
                                "anyld: R_X86_64_32 overflow for '%s' "
                                "(value=0x%llx)\n",
                                sname, (unsigned long long)val);
                        errors++;
                    }
                    *(uint32_t *)patch = (uint32_t)val;
                    /* Record 32-bit runtime relocation */
                    {
                        Elf64_Rela rr;
                        rr.r_offset = P;
                        rr.r_info   = ELF64_R_INFO(0, R_X86_64_32);
                        rr.r_addend = (int64_t)val;
                        buf_append(&ctx->rela_dyn, &rr, sizeof(rr));
                        ctx->nrela_dyn++;
                    }
                }
                break;

            case R_X86_64_32S:
                /* S + A (sign-extend to 32-bit) */
                {
                    int64_t val = (int64_t)S + A;
                    if (val < -2147483648LL || val > 2147483647LL) {
                        const char *sname = r->sym_idx < (uint32_t)ctx->nsyms
                            ? ctx->syms[r->sym_idx].name : "?";
                        fprintf(stderr,
                                "anyld: R_X86_64_32S overflow for '%s' "
                                "(value=0x%llx)\n",
                                sname, (unsigned long long)val);
                        errors++;
                    }
                    *(int32_t *)patch = (int32_t)val;
                    /* Record 32-bit runtime relocation */
                    {
                        Elf64_Rela rr;
                        rr.r_offset = P;
                        rr.r_info   = ELF64_R_INFO(0, R_X86_64_32S);
                        rr.r_addend = val;
                        buf_append(&ctx->rela_dyn, &rr, sizeof(rr));
                        ctx->nrela_dyn++;
                    }
                }
                break;

            case R_X86_64_PC64:
                /* S + A - P (PC-relative 64-bit) */
                *(int64_t *)patch = (int64_t)S + A - (int64_t)P;
                break;

            case R_X86_64_GOTPCREL:
            case R_X86_64_GOTPCRELX:
            case R_X86_64_REX_GOTPCRELX:
                /*
                 * GOT-relative → direct PC-relative relaxation.
                 *
                 * The instruction loads a pointer FROM a GOT entry:
                 *   mov reg, [rip + GOT(sym)]    (opcode 0x8b)
                 *
                 * Since we have no GOT, relax to direct address:
                 *   lea reg, [rip + sym]          (opcode 0x8d)
                 *
                 * The opcode byte is at patch[-2] (before ModRM + disp32).
                 * Without this rewrite, the instruction would DEREFERENCE
                 * the symbol address instead of loading it.
                 */
                {
                    /* Rewrite mov → lea for GOT relaxation */
                    if (r->offset >= 2 && patch[-2] == 0x8b) {
                        patch[-2] = 0x8d;  /* mov → lea */
                    } else if (r->offset >= 2 && patch[-2] != 0x8d) {
                        /* Non-mov GOT access (e.g. call/jmp indirect) */
                        const char *sname = r->sym_idx < (uint32_t)ctx->nsyms
                            ? ctx->syms[r->sym_idx].name : "?";
                        fprintf(stderr,
                                "anyld: warning: GOTPCREL with opcode 0x%02x"
                                " for '%s' (cannot relax)\n",
                                (unsigned)patch[-2], sname);
                    }
                    int64_t val = (int64_t)S + A - (int64_t)P;
                    if (val < -2147483648LL || val > 2147483647LL) {
                        const char *sname = r->sym_idx < (uint32_t)ctx->nsyms
                            ? ctx->syms[r->sym_idx].name : "?";
                        fprintf(stderr,
                                "anyld: GOTPCREL relocation overflow for '%s'"
                                " (value=0x%llx)\n",
                                sname, (unsigned long long)val);
                        errors++;
                    }
                    *(int32_t *)patch = (int32_t)val;
                }
                break;

            default:
                fprintf(stderr,
                        "anyld: unsupported relocation type %u at "
                        "section %d offset 0x%llx\n",
                        r->type, r->out_sec,
                        (unsigned long long)r->offset);
                errors++;
                break;
        }
        continue;

    bounds_err:
        fprintf(stderr,
                "anyld: relocation offset 0x%llx out of bounds "
                "(section %d)\n",
                (unsigned long long)r->offset, r->out_sec);
        errors++;
    }

    return errors > 0 ? -1 : 0;
}

/* ── Public entry: full relocation pipeline ─────────────────────────── */

int apply_relocations(Ctx *ctx) {
    if (collect_relocs(ctx) != 0) return -1;

    /* Pre-size .rela.dyn so compute_layout() accounts for it.
     * Absolute relocations (R_X86_64_64, _32, _32S) each produce a runtime
     * relocation entry. Without pre-sizing, layout is computed with
     * rela_dyn.size=0, causing section offset mismatch. */
    {
        int nabs = 0;
        for (int i = 0; i < ctx->nrelocs; i++) {
            int t = ctx->relocs[i].type;
            if (t == R_X86_64_64 || t == R_X86_64_32 || t == R_X86_64_32S)
                nabs++;
        }
        if (nabs > 0) {
            buf_append_zero(&ctx->rela_dyn,
                            (size_t)nabs * sizeof(Elf64_Rela));
            compute_layout(ctx);
            /* Reset — apply_relocs() will fill it for real */
            ctx->rela_dyn.size = 0;
            ctx->nrela_dyn     = 0;
        }
    }

    finalize_symbol_values(ctx);
    return apply_relocs(ctx);
}
