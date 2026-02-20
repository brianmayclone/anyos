/*
 * convert.c — ELF conversion modes: bin, pflat, dlib, kdrv
 */
#include "anyelf.h"

/* ── Mode: flat binary (by vaddr) ─────────────────────────────────────── */

int do_bin(const char *input, const char *output) {
    size_t size;
    uint8_t *data = read_file(input, &size);
    if (!data) return 1;

    Segment *segs;
    int nsegs, ei_class;
    if (parse_segments(data, size, &segs, &nsegs, &ei_class) != 0) {
        free(data);
        return 1;
    }
    if (nsegs == 0) {
        fprintf(stderr, "anyelf: no PT_LOAD segments\n");
        free(segs); free(data);
        return 1;
    }

    uint64_t base = segs[0].vaddr;
    uint64_t end = 0;
    for (int i = 0; i < nsegs; i++) {
        if (segs[i].vaddr < base) base = segs[i].vaddr;
        uint64_t seg_end = segs[i].vaddr + segs[i].memsz;
        if (seg_end > end) end = seg_end;
    }

    size_t flat_size = (size_t)(end - base);
    uint8_t *flat = calloc(1, flat_size);
    if (!flat) fatal("out of memory (%zu bytes)", flat_size);

    for (int i = 0; i < nsegs; i++) {
        size_t dest = (size_t)(segs[i].vaddr - base);
        memcpy(flat + dest, data + segs[i].offset, (size_t)segs[i].filesz);
    }

    FILE *fp = fopen(output, "wb");
    if (!fp) fatal("cannot create '%s'", output);
    fwrite(flat, 1, flat_size, fp);
    fclose(fp);

    printf("  %s -> %s (%zu bytes, base=0x%08llx)\n",
           input, output, flat_size, (unsigned long long)base);

    free(flat); free(segs); free(data);
    return 0;
}

/* ── Mode: flat binary (by paddr, for kernel) ─────────────────────────── */

int do_pflat(const char *input, const char *output, uint64_t base_paddr) {
    size_t size;
    uint8_t *data = read_file(input, &size);
    if (!data) return 1;

    Segment *segs;
    int nsegs, ei_class;
    if (parse_segments(data, size, &segs, &nsegs, &ei_class) != 0) {
        free(data);
        return 1;
    }
    if (nsegs == 0) {
        fprintf(stderr, "anyelf: no PT_LOAD segments\n");
        free(segs); free(data);
        return 1;
    }

    /* Print ELF info */
    if (ei_class == ELFCLASS64) {
        Elf64_Ehdr *ehdr = (Elf64_Ehdr *)data;
        printf("  ELF64 entry point: 0x%016llX\n",
               (unsigned long long)ehdr->e_entry);
    } else {
        Elf32_Ehdr *ehdr = (Elf32_Ehdr *)data;
        printf("  ELF32 entry point: 0x%08X\n", ehdr->e_entry);
    }
    printf("  Program headers: %d entries\n", nsegs);

    uint64_t max_end = 0;
    for (int i = 0; i < nsegs; i++) {
        if (segs[i].filesz > 0) {
            printf("  PT_LOAD: paddr=0x%08llX vaddr=0x%016llX "
                   "filesz=0x%llX memsz=0x%llX\n",
                   (unsigned long long)segs[i].paddr,
                   (unsigned long long)segs[i].vaddr,
                   (unsigned long long)segs[i].filesz,
                   (unsigned long long)segs[i].memsz);
            uint64_t e = segs[i].paddr + segs[i].memsz;
            if (e > max_end) max_end = e;
        }
    }

    size_t flat_size = (size_t)(max_end - base_paddr);
    uint8_t *flat = calloc(1, flat_size);
    if (!flat) fatal("out of memory (%zu bytes)", flat_size);

    for (int i = 0; i < nsegs; i++) {
        if (segs[i].filesz > 0) {
            size_t dest = (size_t)(segs[i].paddr - base_paddr);
            memcpy(flat + dest, data + segs[i].offset,
                   (size_t)segs[i].filesz);
        }
    }

    printf("  Flat binary: %zu bytes (0x%08llX - 0x%08llX)\n",
           flat_size, (unsigned long long)base_paddr,
           (unsigned long long)max_end);

    FILE *fp = fopen(output, "wb");
    if (!fp) fatal("cannot create '%s'", output);
    fwrite(flat, 1, flat_size, fp);
    fclose(fp);

    free(flat); free(segs); free(data);
    return 0;
}

/* ── Mode: DLIB v3 ────────────────────────────────────────────────────── */

int do_dlib(const char *input, const char *output) {
    size_t size;
    uint8_t *data = read_file(input, &size);
    if (!data) return 1;

    Segment *segs;
    int nsegs, ei_class;
    (void)ei_class;
    if (parse_segments(data, size, &segs, &nsegs, &ei_class) != 0) {
        free(data);
        return 1;
    }
    if (nsegs == 0) {
        fprintf(stderr, "anyelf: no PT_LOAD segments\n");
        free(segs); free(data);
        return 1;
    }

    /* Separate RO and RW segments */
    Segment *ro_segs = malloc(nsegs * sizeof(Segment));
    Segment *rw_segs = malloc(nsegs * sizeof(Segment));
    int nro = 0, nrw = 0;

    for (int i = 0; i < nsegs; i++) {
        if (segs[i].flags & PF_W)
            rw_segs[nrw++] = segs[i];
        else
            ro_segs[nro++] = segs[i];
    }

    if (nro == 0) {
        fprintf(stderr, "anyelf: DLIB has no read-only segments (.rodata/.text)\n");
        free(ro_segs); free(rw_segs); free(segs); free(data);
        return 1;
    }

    uint64_t base = ro_segs[0].vaddr;
    for (int i = 1; i < nro; i++)
        if (ro_segs[i].vaddr < base) base = ro_segs[i].vaddr;

    uint64_t ro_size, data_sz, bss_size;

    if (nrw > 0) {
        uint64_t rw_start = rw_segs[0].vaddr;
        uint64_t rw_file_end = 0, rw_mem_end = 0;
        for (int i = 0; i < nrw; i++) {
            if (rw_segs[i].vaddr < rw_start) rw_start = rw_segs[i].vaddr;
            uint64_t fe = rw_segs[i].vaddr + rw_segs[i].filesz;
            uint64_t me = rw_segs[i].vaddr + rw_segs[i].memsz;
            if (fe > rw_file_end) rw_file_end = fe;
            if (me > rw_mem_end)  rw_mem_end  = me;
        }

        ro_size = ALIGN_UP(rw_start - base, PAGE_SIZE);
        uint64_t data_file_size = rw_file_end - rw_start;
        data_sz = ALIGN_UP(data_file_size, PAGE_SIZE);
        uint64_t total_rw = ALIGN_UP(rw_mem_end - rw_start, PAGE_SIZE);
        bss_size = total_rw - data_sz;
    } else {
        uint64_t ro_end = 0;
        for (int i = 0; i < nro; i++) {
            uint64_t e = ro_segs[i].vaddr + ro_segs[i].memsz;
            if (e > ro_end) ro_end = e;
        }
        ro_size  = ALIGN_UP(ro_end - base, PAGE_SIZE);
        data_sz  = 0;
        bss_size = 0;
    }

    uint32_t ro_pages    = (uint32_t)(ro_size / PAGE_SIZE);
    uint32_t data_pages  = (uint32_t)(data_sz / PAGE_SIZE);
    uint32_t bss_pages   = (uint32_t)(bss_size / PAGE_SIZE);
    uint32_t total_pages = ro_pages + data_pages + bss_pages;

    /* Build flat content: RO + .data template */
    size_t content_size = (size_t)(ro_size + data_sz);
    uint8_t *flat = calloc(1, content_size > 0 ? content_size : 1);
    if (!flat) fatal("out of memory");

    for (int i = 0; i < nsegs; i++) {
        size_t dest = (size_t)(segs[i].vaddr - base);
        size_t copy_end = dest + (size_t)segs[i].filesz;
        if (copy_end > content_size) copy_end = content_size;
        if (dest < content_size && copy_end > dest) {
            size_t len = copy_end - dest;
            memcpy(flat + dest, data + segs[i].offset, len);
        }
    }

    /* Build 4096-byte DLIB v3 header */
    uint8_t header[PAGE_SIZE];
    memset(header, 0, PAGE_SIZE);

    /* magic + version + header_size + flags */
    memcpy(header + 0x00, "DLIB", 4);
    uint32_t v32;
    v32 = 3;         memcpy(header + 0x04, &v32, 4);
    v32 = PAGE_SIZE; memcpy(header + 0x08, &v32, 4);
    v32 = 0;         memcpy(header + 0x0C, &v32, 4);

    /* base_vaddr (8 bytes at 0x10) */
    memcpy(header + 0x10, &base, 8);

    /* ro_pages, data_pages, bss_pages, total_pages (4 each at 0x18) */
    memcpy(header + 0x18, &ro_pages,    4);
    memcpy(header + 0x1C, &data_pages,  4);
    memcpy(header + 0x20, &bss_pages,   4);
    memcpy(header + 0x24, &total_pages, 4);

    FILE *fp = fopen(output, "wb");
    if (!fp) fatal("cannot create '%s'", output);
    fwrite(header, 1, PAGE_SIZE, fp);
    if (content_size > 0)
        fwrite(flat, 1, content_size, fp);
    fclose(fp);

    size_t file_size = PAGE_SIZE + content_size;
    printf("  %s -> %s (DLIB v3: %u RO + %u data + %u BSS pages, "
           "%zu bytes, base=0x%08llx)\n",
           input, output, ro_pages, data_pages, bss_pages,
           file_size, (unsigned long long)base);

    free(flat); free(ro_segs); free(rw_segs); free(segs); free(data);
    return 0;
}

/* ── Mode: KDRV (kernel driver) ───────────────────────────────────────── */

int do_kdrv(const char *input, const char *output,
            const char *exports_symbol) {
    size_t size;
    uint8_t *data = read_file(input, &size);
    if (!data) return 1;

    Segment *segs;
    int nsegs, ei_class;
    if (parse_segments(data, size, &segs, &nsegs, &ei_class) != 0) {
        free(data);
        return 1;
    }

    if (ei_class != ELFCLASS64) {
        fprintf(stderr, "anyelf: KDRV requires ELF64\n");
        free(segs); free(data);
        return 1;
    }
    if (nsegs == 0) {
        fprintf(stderr, "anyelf: no PT_LOAD segments\n");
        free(segs); free(data);
        return 1;
    }

    /* Sort segments by vaddr (insertion sort) */
    for (int i = 1; i < nsegs; i++) {
        Segment tmp = segs[i];
        int j = i - 1;
        while (j >= 0 && segs[j].vaddr > tmp.vaddr) {
            segs[j + 1] = segs[j];
            j--;
        }
        segs[j + 1] = tmp;
    }

    uint64_t base_vaddr = segs[0].vaddr & ~(uint64_t)(PAGE_SIZE - 1);

    /* First pass: determine code and data sizes */
    size_t code_size = 0;
    size_t data_file_size = 0;
    size_t bss_size_raw = 0;

    for (int i = 0; i < nsegs; i++) {
        size_t seg_off = (size_t)(segs[i].vaddr - base_vaddr);
        if (segs[i].flags & PF_W) {
            size_t data_off = seg_off - (size_t)ALIGN_UP(code_size, PAGE_SIZE);
            size_t needed = data_off + (size_t)segs[i].filesz;
            if (needed > data_file_size) data_file_size = needed;
            bss_size_raw = (size_t)(segs[i].memsz - segs[i].filesz);
        } else {
            size_t needed = seg_off + (size_t)segs[i].filesz;
            if (needed > code_size) code_size = needed;
        }
    }

    /* Allocate and fill */
    uint8_t *code_data = calloc(1, code_size > 0 ? code_size : 1);
    uint8_t *data_data = calloc(1, data_file_size > 0 ? data_file_size : 1);
    if (!code_data || !data_data) fatal("out of memory");

    for (int i = 0; i < nsegs; i++) {
        size_t seg_off = (size_t)(segs[i].vaddr - base_vaddr);
        if (segs[i].flags & PF_W) {
            size_t data_off = seg_off - (size_t)ALIGN_UP(code_size, PAGE_SIZE);
            memcpy(data_data + data_off, data + segs[i].offset,
                   (size_t)segs[i].filesz);
        } else {
            memcpy(code_data + seg_off, data + segs[i].offset,
                   (size_t)segs[i].filesz);
        }
    }

    uint32_t code_pages = code_size > 0
        ? (uint32_t)(ALIGN_UP(code_size, PAGE_SIZE) / PAGE_SIZE) : 0;
    uint32_t data_pages = data_file_size > 0
        ? (uint32_t)(ALIGN_UP(data_file_size, PAGE_SIZE) / PAGE_SIZE) : 0;
    uint32_t bss_pages = bss_size_raw > 0
        ? (uint32_t)(ALIGN_UP(bss_size_raw, PAGE_SIZE) / PAGE_SIZE) : 0;

    /* Find exports symbol */
    uint64_t exports_addr = find_symbol_64(data, size, exports_symbol);
    uint64_t exports_offset = 0;
    if (exports_addr == (uint64_t)-1) {
        fprintf(stderr, "WARNING: Symbol '%s' not found — "
                "exports_offset set to 0\n", exports_symbol);
    } else {
        exports_offset = PAGE_SIZE + (exports_addr - base_vaddr);
    }

    /* Build KDRV header */
    uint8_t header[PAGE_SIZE];
    memset(header, 0, PAGE_SIZE);

    memcpy(header + 0,  "KDRV", 4);
    uint32_t v32;
    v32 = 1; memcpy(header + 4,  &v32, 4);  /* version */
    v32 = 1; memcpy(header + 8,  &v32, 4);  /* abi_version */
    v32 = 0; memcpy(header + 12, &v32, 4);  /* flags */
    memcpy(header + 16, &exports_offset, 8);
    memcpy(header + 24, &code_pages, 4);
    memcpy(header + 28, &data_pages, 4);
    memcpy(header + 32, &bss_pages,  4);

    /* Write output */
    FILE *fp = fopen(output, "wb");
    if (!fp) fatal("cannot create '%s'", output);

    fwrite(header, 1, PAGE_SIZE, fp);

    if (code_pages > 0) {
        size_t padded = code_pages * PAGE_SIZE;
        uint8_t *pad = calloc(1, padded);
        memcpy(pad, code_data, code_size);
        fwrite(pad, 1, padded, fp);
        free(pad);
    }
    if (data_pages > 0) {
        size_t padded = data_pages * PAGE_SIZE;
        uint8_t *pad = calloc(1, padded);
        memcpy(pad, data_data, data_file_size);
        fwrite(pad, 1, padded, fp);
        free(pad);
    }
    fclose(fp);

    size_t total_size = PAGE_SIZE + code_pages * PAGE_SIZE
                                  + data_pages * PAGE_SIZE;
    printf("anyelf kdrv: %s -> %s\n", input, output);
    printf("  base_vaddr: 0x%llx\n",        (unsigned long long)base_vaddr);
    printf("  code: %u pages (%zu bytes)\n", code_pages, code_size);
    printf("  data: %u pages (%zu bytes)\n", data_pages, data_file_size);
    printf("  bss:  %u pages (%zu bytes)\n", bss_pages,  bss_size_raw);
    printf("  exports_offset: 0x%llx\n",     (unsigned long long)exports_offset);
    printf("  total: %zu bytes\n",           total_size);

    free(code_data); free(data_data); free(segs); free(data);
    return 0;
}
