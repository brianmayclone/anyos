/*
 * mkimage.h — anyOS disk image builder
 *
 * Replaces mkimage.py with a C tool for self-hosting on anyOS.
 * Supports BIOS, UEFI, and ISO image creation with FAT16/exFAT filesystems.
 *
 * Written in C99 for TCC compatibility.
 */
#ifndef MKIMAGE_H
#define MKIMAGE_H

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <stdarg.h>

/* ── Constants ────────────────────────────────────────────────────────── */

#define SECTOR_SIZE   512
#define PAGE_SIZE     4096
#define ISO_BLOCK_SIZE 2048

/* ELF */
#define ELFMAG0   0x7f
#define ELFMAG1   'E'
#define ELFMAG2   'L'
#define ELFMAG3   'F'
#define ELFCLASS32 1
#define ELFCLASS64 2
#define PT_LOAD    1

/* FAT */
#define FAT16_MAX_ROOT_ENTRIES  512
#define FAT16_END_OF_CHAIN      0xFFFF
#define FAT16_MEDIA_TYPE        0xF8

/* exFAT */
#define EXFAT_EOC       0xFFFFFFFF
#define EXFAT_FREE      0x00000000
#define EXFAT_ENTRY_BITMAP   0x81
#define EXFAT_ENTRY_UPCASE   0x82
#define EXFAT_ENTRY_LABEL    0x83
#define EXFAT_ENTRY_FILE     0x85
#define EXFAT_ENTRY_STREAM   0xC0
#define EXFAT_ENTRY_FILENAME 0xC1
#define EXFAT_ATTR_DIR       0x0010
#define EXFAT_ATTR_ARCHIVE   0x0020
#define EXFAT_FLAG_CONTIGUOUS 0x02

/* GPT */
#define GPT_HEADER_SIZE  92
#define GPT_ENTRY_SIZE   128
#define GPT_ENTRY_COUNT  128

/* ── Macros ───────────────────────────────────────────────────────────── */

#define ALIGN_UP(v, a) (((v) + (a) - 1) & ~((uint64_t)(a) - 1))

/* ── ELF structures (minimal, for segment parsing) ────────────────────── */

typedef struct {
    uint8_t  e_ident[16];
    uint16_t e_type, e_machine;
    uint32_t e_version;
    uint32_t e_entry, e_phoff, e_shoff, e_flags;
    uint16_t e_ehsize, e_phentsize, e_phnum;
    uint16_t e_shentsize, e_shnum, e_shstrndx;
} Elf32_Ehdr;

typedef struct {
    uint32_t p_type, p_offset, p_vaddr, p_paddr;
    uint32_t p_filesz, p_memsz, p_flags, p_align;
} Elf32_Phdr;

typedef struct {
    uint8_t  e_ident[16];
    uint16_t e_type, e_machine;
    uint32_t e_version;
    uint64_t e_entry, e_phoff, e_shoff;
    uint32_t e_flags;
    uint16_t e_ehsize, e_phentsize, e_phnum;
    uint16_t e_shentsize, e_shnum, e_shstrndx;
} Elf64_Ehdr;

typedef struct {
    uint32_t p_type, p_flags;
    uint64_t p_offset, p_vaddr, p_paddr;
    uint64_t p_filesz, p_memsz, p_align;
} Elf64_Phdr;

/* ── Command-line arguments ───────────────────────────────────────────── */

typedef struct {
    int         mode;       /* 0=bios, 1=uefi, 2=iso, 3=arm64 */
    const char *stage1;
    const char *stage2;
    const char *kernel;
    const char *bootloader; /* UEFI .efi */
    const char *output;
    const char *sysroot;
    int         image_size; /* MiB */
    int         fs_start;   /* sector */
    int         reset;      /* 1 = force full rebuild, 0 = incremental if possible */
} Args;

/* ── FAT16 formatter state ────────────────────────────────────────────── */

typedef struct {
    uint8_t *image;
    uint32_t fs_start;      /* absolute sector */
    uint32_t fs_sectors;
    uint32_t sectors_per_cluster;
    uint32_t reserved_sectors;
    uint32_t num_fats;
    uint32_t root_entry_count;
    uint32_t root_dir_sectors;
    uint32_t fat_size;
    uint32_t total_clusters;
    uint32_t first_fat_sector;
    uint32_t first_root_dir_sector;
    uint32_t first_data_sector;
    uint32_t next_cluster;
    uint32_t next_root_entry;
} Fat16;

/* ── exFAT formatter state ────────────────────────────────────────────── */

typedef struct {
    uint8_t *image;
    uint32_t fs_start;
    uint32_t fs_sectors;
    uint32_t spc;           /* sectors per cluster */
    uint32_t cluster_size;
    uint32_t fat_offset;
    uint32_t fat_length;
    uint32_t cluster_heap_offset;
    uint32_t cluster_count;
    uint32_t next_cluster;
    uint32_t bitmap_cluster;
    uint32_t root_cluster;
    uint8_t *fat_cache;     /* in-memory FAT */
    uint8_t *bitmap;        /* in-memory allocation bitmap */
    uint32_t bitmap_bytes;
} ExFat;

/* ── Short name collision tracker ─────────────────────────────────────── */

/* Simple counter array for LFN short name generation */
#define SHORT_NAME_SLOTS 4096

/* ── Utility functions (mkimage.c) ────────────────────────────────────── */

void     fatal(const char *fmt, ...);
uint8_t *read_file(const char *path, size_t *out_size);
uint32_t crc32(const uint8_t *data, size_t len);
void     write_le16(uint8_t *p, uint16_t v);
void     write_le32(uint8_t *p, uint32_t v);
void     write_le64(uint8_t *p, uint64_t v);
void     write_be16(uint8_t *p, uint16_t v);
void     write_be32(uint8_t *p, uint32_t v);
uint16_t read_le16(const uint8_t *p);
uint32_t read_le32(const uint8_t *p);
uint64_t read_le64(const uint8_t *p);

/* ── ELF functions (elf.c) ────────────────────────────────────────────── */

uint8_t *elf_to_flat(const uint8_t *elf_data, size_t elf_size,
                     uint64_t base_paddr, size_t *out_size);

/* ── FAT16 functions (fat16.c) ────────────────────────────────────────── */

void     fat16_init(Fat16 *fs, uint8_t *image, uint32_t fs_start,
                    uint32_t fs_sectors, uint32_t spc);
void     fat16_write_bpb(Fat16 *fs);
void     fat16_init_fat(Fat16 *fs);
uint32_t fat16_create_dir(Fat16 *fs, uint32_t parent, const char *name,
                          int is_root_parent);
void     fat16_add_file(Fat16 *fs, uint32_t parent, const char *name,
                        const uint8_t *data, size_t size,
                        int is_root_parent);
void     fat16_populate_sysroot(Fat16 *fs, const char *sysroot_path);

/* ── exFAT directory tree node (for incremental updates) ──────────────── */

typedef struct ExFatNode {
    char          name[256];       /* UTF-8 filename */
    uint16_t      attrs;           /* EXFAT_ATTR_DIR or EXFAT_ATTR_ARCHIVE */
    uint32_t      first_cluster;
    uint64_t      data_length;
    uint16_t      uid, gid, mode;  /* VFS permissions */
    int           contiguous;      /* EXFAT_FLAG_CONTIGUOUS set */
    uint32_t      dir_cluster;     /* cluster of parent dir containing this entry */
    uint32_t      entry_offset;    /* byte offset of entry set in dir cluster chain */
    uint32_t      entry_set_len;   /* total bytes of entry set (File+Stream+FileNames) */
    /* Children (for directories) */
    struct ExFatNode *children;
    int           child_count;
    int           child_cap;
} ExFatNode;

/* ── exFAT functions (exfat.c) ────────────────────────────────────────── */

/* Format + populate (full rebuild) */
void     exfat_init(ExFat *fs, uint8_t *image, uint32_t fs_start,
                    uint32_t fs_sectors, uint32_t spc);
void     exfat_write_boot(ExFat *fs);
void     exfat_init_fs(ExFat *fs);
uint32_t exfat_create_dir(ExFat *fs, uint32_t parent, const char *name,
                          uint16_t uid, uint16_t gid, uint16_t mode);
void     exfat_add_file(ExFat *fs, uint32_t parent, const char *name,
                        const uint8_t *data, size_t size,
                        uint16_t uid, uint16_t gid, uint16_t mode);
void     exfat_populate_sysroot(ExFat *fs, const char *sysroot_path);
void     exfat_flush(ExFat *fs);
void     exfat_free(ExFat *fs);

/* Incremental update */
void       exfat_open_existing(ExFat *fs, uint8_t *image, uint32_t fs_start);
ExFatNode *exfat_read_dir_tree(ExFat *fs, uint32_t dir_cluster);
ExFatNode *exfat_find_child(ExFatNode *parent, const char *name);
int        exfat_file_matches(ExFat *fs, ExFatNode *node,
                              const uint8_t *new_data, size_t new_size);
void       exfat_free_clusters(ExFat *fs, ExFatNode *node);
void       exfat_delete_entry(ExFat *fs, ExFatNode *node);
void       exfat_sync_sysroot(ExFat *fs, const char *sysroot_path);
void       exfat_free_tree(ExFatNode *node);

/* ── GPT functions (gpt.c) ────────────────────────────────────────────── */

typedef struct {
    uint8_t type_guid[16];
    uint8_t unique_guid[16];
    uint64_t first_lba;
    uint64_t last_lba;
    const char *name;
} GptPartition;

void write_protective_mbr(uint8_t *image, uint64_t total_sectors);
void create_gpt(uint8_t *image, uint64_t total_sectors,
                const GptPartition *parts, int nparts);

/* Well-known GUIDs */
void guid_esp(uint8_t *out);        /* EFI System Partition */
void guid_basic_data(uint8_t *out);  /* Basic Data */
void guid_random(uint8_t *out);      /* Random unique GUID */

/* ── ISO 9660 functions (iso9660.c) ───────────────────────────────────── */

void create_iso_image(const Args *args);

/* ── BIOS/UEFI image creation (mkimage.c) ─────────────────────────────── */

void create_bios_image(const Args *args);
void create_uefi_image(const Args *args);
void create_arm64_image(const Args *args);

/* ── LFN helpers (fat16.c, shared with exfat for FAT16 ESP) ───────────── */

int  needs_lfn(const char *filename);
void generate_short_name(const char *filename, char *out11);
int  make_lfn_entries(const char *filename, const char *name83,
                      uint8_t *entries, int max_entries);
uint8_t lfn_checksum(const uint8_t *name83);

#endif /* MKIMAGE_H */
