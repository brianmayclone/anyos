/*
 * elf.c — ELF to flat binary conversion for mkimage
 *
 * Supports both ELF32 and ELF64 executables.  Only PT_LOAD segments with
 * filesz > 0 are copied into the output buffer; the buffer is zero-initialised
 * (via calloc) so BSS regions are implicitly zeroed.
 *
 * Written in C99 for TCC compatibility.
 */

#include "mkimage.h"

uint8_t *elf_to_flat(const uint8_t *elf_data, size_t elf_size,
                     uint64_t base_paddr, size_t *out_size)
{
    /* ── Validate ELF magic ────────────────────────────────────────────── */
    if (elf_size < 16) {
        fprintf(stderr, "elf_to_flat: file too small to be an ELF (%zu bytes)\n",
                elf_size);
        return NULL;
    }

    if (elf_data[0] != ELFMAG0 || elf_data[1] != ELFMAG1 ||
        elf_data[2] != ELFMAG2 || elf_data[3] != ELFMAG3) {
        fprintf(stderr, "elf_to_flat: not an ELF file (bad magic)\n");
        return NULL;
    }

    uint8_t elf_class = elf_data[4];
    if (elf_class != ELFCLASS32 && elf_class != ELFCLASS64) {
        fprintf(stderr, "elf_to_flat: unknown ELF class %d\n", (int)elf_class);
        return NULL;
    }

    /* ── Dispatch to 32-bit or 64-bit parser ───────────────────────────── */

    if (elf_class == ELFCLASS32) {
        /* ── ELF32 ────────────────────────────────────────────────────── */

        if (elf_size < sizeof(Elf32_Ehdr)) {
            fprintf(stderr, "elf_to_flat: file too small for ELF32 header\n");
            return NULL;
        }

        const Elf32_Ehdr *ehdr = (const Elf32_Ehdr *)elf_data;

        printf("  ELF32 entry point: 0x%08X\n", (unsigned)ehdr->e_entry);
        printf("  Program headers: %d entries at offset %llu\n",
               (int)ehdr->e_phnum, (unsigned long long)ehdr->e_phoff);

        if (ehdr->e_phnum == 0 || ehdr->e_phoff == 0) {
            fprintf(stderr, "elf_to_flat: ELF32 has no program headers\n");
            return NULL;
        }

        if ((uint64_t)ehdr->e_phoff + (uint64_t)ehdr->e_phnum * sizeof(Elf32_Phdr) > elf_size) {
            fprintf(stderr, "elf_to_flat: ELF32 program header table out of bounds\n");
            return NULL;
        }

        const Elf32_Phdr *phdrs =
            (const Elf32_Phdr *)(elf_data + ehdr->e_phoff);

        /* First pass: determine extent */
        uint64_t max_paddr_end = 0;
        int i;
        for (i = 0; i < (int)ehdr->e_phnum; i++) {
            if (phdrs[i].p_type != PT_LOAD || phdrs[i].p_filesz == 0)
                continue;
            uint64_t seg_end = (uint64_t)phdrs[i].p_paddr +
                               (uint64_t)phdrs[i].p_memsz;
            if (seg_end > max_paddr_end)
                max_paddr_end = seg_end;
        }

        if (max_paddr_end <= base_paddr) {
            fprintf(stderr,
                    "elf_to_flat: ELF32 no PT_LOAD segments above base 0x%08llX\n",
                    (unsigned long long)base_paddr);
            return NULL;
        }

        size_t flat_size = (size_t)(max_paddr_end - base_paddr);
        uint8_t *flat = (uint8_t *)calloc(1, flat_size);
        if (!flat)
            fatal("elf_to_flat: calloc(%zu) failed", flat_size);

        /* Second pass: copy segments */
        for (i = 0; i < (int)ehdr->e_phnum; i++) {
            if (phdrs[i].p_type != PT_LOAD || phdrs[i].p_filesz == 0)
                continue;

            uint64_t paddr  = (uint64_t)phdrs[i].p_paddr;
            uint64_t vaddr  = (uint64_t)phdrs[i].p_vaddr;
            uint64_t filesz = (uint64_t)phdrs[i].p_filesz;
            uint64_t memsz  = (uint64_t)phdrs[i].p_memsz;

            printf("  PT_LOAD: paddr=0x%08llX vaddr=0x%016llX"
                   " filesz=0x%llX memsz=0x%llX\n",
                   (unsigned long long)paddr,
                   (unsigned long long)vaddr,
                   (unsigned long long)filesz,
                   (unsigned long long)memsz);

            if (paddr < base_paddr) {
                fprintf(stderr,
                        "elf_to_flat: ELF32 segment paddr 0x%08llX"
                        " is below base 0x%08llX, skipping\n",
                        (unsigned long long)paddr,
                        (unsigned long long)base_paddr);
                continue;
            }

            uint64_t file_offset = (uint64_t)phdrs[i].p_offset;
            if (file_offset + filesz > elf_size) {
                fprintf(stderr,
                        "elf_to_flat: ELF32 segment data out of bounds\n");
                free(flat);
                return NULL;
            }

            uint64_t dest_offset = paddr - base_paddr;
            if (dest_offset + filesz > flat_size) {
                fprintf(stderr,
                        "elf_to_flat: ELF32 segment exceeds flat buffer\n");
                free(flat);
                return NULL;
            }

            memcpy(flat + dest_offset, elf_data + file_offset, (size_t)filesz);
        }

        printf("  Flat binary: %zu bytes (0x%08llX - 0x%08llX)\n",
               flat_size,
               (unsigned long long)base_paddr,
               (unsigned long long)max_paddr_end);

        *out_size = flat_size;
        return flat;

    } else {
        /* ── ELF64 ────────────────────────────────────────────────────── */

        if (elf_size < sizeof(Elf64_Ehdr)) {
            fprintf(stderr, "elf_to_flat: file too small for ELF64 header\n");
            return NULL;
        }

        const Elf64_Ehdr *ehdr = (const Elf64_Ehdr *)elf_data;

        printf("  ELF64 entry point: 0x%016llX\n",
               (unsigned long long)ehdr->e_entry);
        printf("  Program headers: %d entries at offset %llu\n",
               (int)ehdr->e_phnum, (unsigned long long)ehdr->e_phoff);

        if (ehdr->e_phnum == 0 || ehdr->e_phoff == 0) {
            fprintf(stderr, "elf_to_flat: ELF64 has no program headers\n");
            return NULL;
        }

        if (ehdr->e_phoff + (uint64_t)ehdr->e_phnum * sizeof(Elf64_Phdr) > elf_size) {
            fprintf(stderr, "elf_to_flat: ELF64 program header table out of bounds\n");
            return NULL;
        }

        const Elf64_Phdr *phdrs =
            (const Elf64_Phdr *)(elf_data + ehdr->e_phoff);

        /* First pass: determine extent */
        uint64_t max_paddr_end = 0;
        int i;
        for (i = 0; i < (int)ehdr->e_phnum; i++) {
            if (phdrs[i].p_type != PT_LOAD || phdrs[i].p_filesz == 0)
                continue;
            uint64_t seg_end = phdrs[i].p_paddr + phdrs[i].p_memsz;
            if (seg_end > max_paddr_end)
                max_paddr_end = seg_end;
        }

        if (max_paddr_end <= base_paddr) {
            fprintf(stderr,
                    "elf_to_flat: ELF64 no PT_LOAD segments above base 0x%08llX\n",
                    (unsigned long long)base_paddr);
            return NULL;
        }

        size_t flat_size = (size_t)(max_paddr_end - base_paddr);
        uint8_t *flat = (uint8_t *)calloc(1, flat_size);
        if (!flat)
            fatal("elf_to_flat: calloc(%zu) failed", flat_size);

        /* Second pass: copy segments */
        for (i = 0; i < (int)ehdr->e_phnum; i++) {
            if (phdrs[i].p_type != PT_LOAD || phdrs[i].p_filesz == 0)
                continue;

            uint64_t paddr  = phdrs[i].p_paddr;
            uint64_t vaddr  = phdrs[i].p_vaddr;
            uint64_t filesz = phdrs[i].p_filesz;
            uint64_t memsz  = phdrs[i].p_memsz;

            printf("  PT_LOAD: paddr=0x%08llX vaddr=0x%016llX"
                   " filesz=0x%llX memsz=0x%llX\n",
                   (unsigned long long)paddr,
                   (unsigned long long)vaddr,
                   (unsigned long long)filesz,
                   (unsigned long long)memsz);

            if (paddr < base_paddr) {
                fprintf(stderr,
                        "elf_to_flat: ELF64 segment paddr 0x%016llX"
                        " is below base 0x%016llX, skipping\n",
                        (unsigned long long)paddr,
                        (unsigned long long)base_paddr);
                continue;
            }

            uint64_t file_offset = phdrs[i].p_offset;
            if (file_offset + filesz > elf_size) {
                fprintf(stderr,
                        "elf_to_flat: ELF64 segment data out of bounds\n");
                free(flat);
                return NULL;
            }

            uint64_t dest_offset = paddr - base_paddr;
            if (dest_offset + filesz > flat_size) {
                fprintf(stderr,
                        "elf_to_flat: ELF64 segment exceeds flat buffer\n");
                free(flat);
                return NULL;
            }

            memcpy(flat + dest_offset, elf_data + file_offset, (size_t)filesz);
        }

        printf("  Flat binary: %zu bytes (0x%08llX - 0x%08llX)\n",
               flat_size,
               (unsigned long long)base_paddr,
               (unsigned long long)max_paddr_end);

        *out_size = flat_size;
        return flat;
    }
}
