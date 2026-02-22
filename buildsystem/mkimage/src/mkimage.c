/*
 * mkimage — anyOS disk image builder
 *
 * Replaces mkimage.py with a C tool for self-hosting on anyOS.
 * Supports BIOS (MBR + exFAT), UEFI (GPT + ESP + exFAT), and ISO modes.
 *
 * Written in C99 for TCC compatibility.
 *
 * Usage:
 *   mkimage --stage1 s1.bin --stage2 s2.bin --kernel k.elf
 *           --output disk.img [--sysroot dir] [--image-size 64] [--fs-start 8192]
 *   mkimage --uefi --bootloader boot.efi --kernel k.elf
 *           --output disk.img [--sysroot dir]
 *   mkimage --iso --stage1 s1.bin --stage2 s2.bin --kernel k.elf
 *           --output disk.img [--sysroot dir]
 */
#include "mkimage.h"
#include <time.h>

#ifdef ONE_SOURCE
/* Single-source compilation mode (for TCC on anyOS) */
#include "elf.c"
#include "fat16.c"
#include "exfat.c"
#include "gpt.c"
#include "iso9660.c"
#endif

/* ── Utility: fatal error ─────────────────────────────────────────────── */

void fatal(const char *fmt, ...) {
    va_list ap;
    fprintf(stderr, "mkimage: fatal: ");
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
        fprintf(stderr, "mkimage: cannot open '%s'\n", path);
        return NULL;
    }

    fseek(fp, 0, SEEK_END);
    long sz = ftell(fp);
    if (sz < 0) { fclose(fp); return NULL; }
    fseek(fp, 0, SEEK_SET);

    uint8_t *buf = malloc((size_t)sz);
    if (!buf) { fclose(fp); fatal("out of memory"); }

    size_t n = fread(buf, 1, (size_t)sz, fp);
    fclose(fp);

    if (n != (size_t)sz) {
        fprintf(stderr, "mkimage: short read on '%s'\n", path);
        free(buf);
        return NULL;
    }

    *out_size = (size_t)sz;
    return buf;
}

/* ── CRC32 (standard Ethernet/PKZIP polynomial) ──────────────────────── */

static uint32_t crc32_table[256];
static int crc32_table_init = 0;

static void crc32_init_table(void) {
    if (crc32_table_init) return;
    for (uint32_t i = 0; i < 256; i++) {
        uint32_t c = i;
        for (int j = 0; j < 8; j++) {
            if (c & 1)
                c = 0xEDB88320 ^ (c >> 1);
            else
                c >>= 1;
        }
        crc32_table[i] = c;
    }
    crc32_table_init = 1;
}

uint32_t crc32(const uint8_t *data, size_t len) {
    crc32_init_table();
    uint32_t c = 0xFFFFFFFF;
    for (size_t i = 0; i < len; i++)
        c = crc32_table[(c ^ data[i]) & 0xFF] ^ (c >> 8);
    return c ^ 0xFFFFFFFF;
}

/* ── Little/big-endian helpers ────────────────────────────────────────── */

void write_le16(uint8_t *p, uint16_t v) {
    p[0] = (uint8_t)(v);
    p[1] = (uint8_t)(v >> 8);
}

void write_le32(uint8_t *p, uint32_t v) {
    p[0] = (uint8_t)(v);
    p[1] = (uint8_t)(v >> 8);
    p[2] = (uint8_t)(v >> 16);
    p[3] = (uint8_t)(v >> 24);
}

void write_le64(uint8_t *p, uint64_t v) {
    write_le32(p, (uint32_t)(v));
    write_le32(p + 4, (uint32_t)(v >> 32));
}

void write_be16(uint8_t *p, uint16_t v) {
    p[0] = (uint8_t)(v >> 8);
    p[1] = (uint8_t)(v);
}

void write_be32(uint8_t *p, uint32_t v) {
    p[0] = (uint8_t)(v >> 24);
    p[1] = (uint8_t)(v >> 16);
    p[2] = (uint8_t)(v >> 8);
    p[3] = (uint8_t)(v);
}

uint16_t read_le16(const uint8_t *p) {
    return (uint16_t)p[0] | ((uint16_t)p[1] << 8);
}

uint32_t read_le32(const uint8_t *p) {
    return (uint32_t)p[0] | ((uint32_t)p[1] << 8) |
           ((uint32_t)p[2] << 16) | ((uint32_t)p[3] << 24);
}

uint64_t read_le64(const uint8_t *p) {
    return (uint64_t)read_le32(p) | ((uint64_t)read_le32(p + 4) << 32);
}

/* ── BIOS image creation ──────────────────────────────────────────────── */

void create_bios_image(const Args *args) {
    size_t s1_size, s2_size, k_size;
    uint8_t *s1 = read_file(args->stage1, &s1_size);
    if (!s1) fatal("cannot read stage1");
    uint8_t *s2 = read_file(args->stage2, &s2_size);
    if (!s2) fatal("cannot read stage2");
    uint8_t *kelf = read_file(args->kernel, &k_size);
    if (!kelf) fatal("cannot read kernel");

    if (s1_size != SECTOR_SIZE)
        fatal("stage1 must be exactly %d bytes, got %zu", SECTOR_SIZE, s1_size);
    if (s2_size > 63 * SECTOR_SIZE)
        fatal("stage2 too large: %zu bytes (max %d)", s2_size, 63 * SECTOR_SIZE);

    /* Convert kernel ELF to flat binary */
    uint64_t kernel_lma = 0x00100000;
    printf("Kernel ELF: %zu bytes\n", k_size);
    size_t flat_size;
    uint8_t *kernel = elf_to_flat(kelf, k_size, kernel_lma, &flat_size);
    if (!kernel) fatal("kernel ELF conversion failed");
    free(kelf);

    uint32_t kernel_sectors = (uint32_t)((flat_size + SECTOR_SIZE - 1) / SECTOR_SIZE);
    uint32_t kernel_start = 64;

    printf("Stage 1: %zu bytes (1 sector)\n", s1_size);
    printf("Stage 2: %zu bytes (%zu sectors)\n", s2_size,
           (s2_size + SECTOR_SIZE - 1) / SECTOR_SIZE);
    printf("Kernel:  %zu bytes (%u sectors, starting at sector %u)\n",
           flat_size, kernel_sectors, kernel_start);

    uint32_t kernel_end = kernel_start + kernel_sectors;
    if (kernel_end > (uint32_t)args->fs_start)
        fatal("kernel ends at sector %u, overlaps filesystem at sector %d",
              kernel_end, args->fs_start);

    /* Patch stage2 with kernel location */
    if (s2_size >= 8) {
        write_le16(s2 + 2, (uint16_t)kernel_sectors);
        write_le32(s2 + 4, kernel_start);
    }

    /* Create or load image */
    size_t image_size = (size_t)args->image_size * 1024 * 1024;
    int incremental = 0;
    uint8_t *image = NULL;

    /* Check for incremental update: existing image of same size, not --reset */
    if (!args->reset && args->sysroot) {
        FILE *f = fopen(args->output, "rb");
        if (f) {
            fseek(f, 0, SEEK_END);
            long existing_size = ftell(f);
            fclose(f);
            if (existing_size > 0 && (size_t)existing_size == image_size) {
                incremental = 1;
            }
        }
    }

    if (incremental) {
        /* Load existing image for incremental update */
        size_t dummy;
        image = read_file(args->output, &dummy);
        if (!image) fatal("cannot read existing image '%s'", args->output);
        printf("\nIncremental update mode (use --reset for full rebuild)\n");
    } else {
        image = calloc(1, image_size);
        if (!image) fatal("out of memory for image (%zu bytes)", image_size);
        if (args->reset)
            printf("\nFull rebuild (--reset)\n");
    }

    /* Always write boot sectors + kernel (even in incremental mode) */
    memcpy(image, s1, s1_size);
    memcpy(image + SECTOR_SIZE, s2, s2_size);
    memcpy(image + (size_t)kernel_start * SECTOR_SIZE, kernel, flat_size);

    /* Write MBR partition table (bytes 446-509 of sector 0).
     * Stage 1 bootloader code occupies bytes 0-~106, so this is safe.
     * Partition 1: the exFAT data partition at fs_start.
     * The boot signature 0x55AA at bytes 510-511 is already part of Stage 1. */
    {
        uint32_t part_sectors = (uint32_t)(image_size / SECTOR_SIZE)
                                - (uint32_t)args->fs_start;
        uint8_t *entry = image + 446;
        memset(entry, 0, 64);  /* zero all 4 partition entries */

        /* Entry 1: data partition (exFAT) */
        entry[0] = 0x80;      /* bootable / active */
        /* CHS start: head=0, sector=2, cylinder=0 (placeholder for LBA) */
        entry[1] = 0x00; entry[2] = 0x02; entry[3] = 0x00;
        entry[4] = 0x07;      /* type 0x07 = NTFS/exFAT/HPFS */
        /* CHS end: max for LBA mode */
        entry[5] = 0xFE; entry[6] = 0xFF; entry[7] = 0xFF;
        write_le32(entry + 8, (uint32_t)args->fs_start);
        write_le32(entry + 12, part_sectors);

        printf("\nMBR partition table:\n");
        printf("  Partition 1: type=0x07 (exFAT) start=%d sectors=%u\n",
               args->fs_start, part_sectors);
    }

    free(s1); free(s2); free(kernel);

    /* exFAT filesystem */
    uint32_t fs_sectors = (uint32_t)(image_size / SECTOR_SIZE) - (uint32_t)args->fs_start;
    printf("\nexFAT filesystem:\n");
    printf("  Start sector: %d (offset 0x%X)\n",
           args->fs_start, args->fs_start * SECTOR_SIZE);
    printf("  Size: %u sectors (%u MiB)\n",
           fs_sectors, fs_sectors * SECTOR_SIZE / (1024 * 1024));

    if (incremental) {
        /* Incremental: open existing FS, sync sysroot */
        ExFat exfat;
        exfat_open_existing(&exfat, image, (uint32_t)args->fs_start);
        if (args->sysroot) {
            exfat_sync_sysroot(&exfat, args->sysroot);
        }
        exfat_flush(&exfat);
        exfat_free(&exfat);
    } else {
        /* Full rebuild: format + populate */
        ExFat exfat;
        exfat_init(&exfat, image, (uint32_t)args->fs_start, fs_sectors, 8);
        exfat_write_boot(&exfat);
        exfat_init_fs(&exfat);

        if (args->sysroot) {
            printf("  Populating from sysroot: %s\n", args->sysroot);
            exfat_populate_sysroot(&exfat, args->sysroot);
        }

        exfat_flush(&exfat);
        exfat_free(&exfat);
    }

    /* Write image */
    FILE *fp = fopen(args->output, "wb");
    if (!fp) fatal("cannot create '%s'", args->output);
    fwrite(image, 1, image_size, fp);
    fclose(fp);
    free(image);

    printf("\nDisk image %s: %s (%d MiB)\n",
           incremental ? "updated" : "created", args->output, args->image_size);
}

/* ── UEFI image creation ──────────────────────────────────────────────── */

void create_uefi_image(const Args *args) {
    if (!args->bootloader)
        fatal("--bootloader required for UEFI mode");

    size_t efi_size;
    uint8_t *efi_data = read_file(args->bootloader, &efi_size);
    if (!efi_data) fatal("cannot read bootloader");

    uint8_t *kernel_flat = NULL;
    size_t kernel_flat_size = 0;
    if (args->kernel) {
        size_t k_size;
        uint8_t *kelf = read_file(args->kernel, &k_size);
        if (!kelf) fatal("cannot read kernel");
        uint64_t kernel_lma = 0x00100000;
        printf("Kernel ELF: %zu bytes\n", k_size);
        kernel_flat = elf_to_flat(kelf, k_size, kernel_lma, &kernel_flat_size);
        if (!kernel_flat) fatal("kernel ELF conversion failed");
        free(kelf);
    }

    size_t image_size = (size_t)args->image_size * 1024 * 1024;
    uint64_t total_sectors = image_size / SECTOR_SIZE;

    printf("\nUEFI image: %d MiB (%llu sectors)\n",
           args->image_size, (unsigned long long)total_sectors);
    printf("EFI bootloader: %zu bytes\n", efi_size);
    if (kernel_flat)
        printf("Kernel flat binary: %zu bytes\n", kernel_flat_size);

    /* Partition layout */
    uint64_t esp_start = 2048;
    uint64_t esp_sectors = 6144;  /* 3 MiB */
    uint64_t esp_end = esp_start + esp_sectors - 1;

    uint64_t data_start = esp_start + esp_sectors;  /* 8192 = kernel PARTITION_LBA */
    uint32_t entry_sectors = (GPT_ENTRY_COUNT * GPT_ENTRY_SIZE + 511) / 512;
    uint64_t data_end = total_sectors - 1 - entry_sectors - 1;
    uint64_t data_sectors = data_end - data_start + 1;

    printf("\nPartition layout:\n");
    printf("  ESP:  sectors %llu-%llu (%llu KiB)\n",
           (unsigned long long)esp_start, (unsigned long long)esp_end,
           (unsigned long long)(esp_sectors * 512 / 1024));
    printf("  Data: sectors %llu-%llu (%llu MiB)\n",
           (unsigned long long)data_start, (unsigned long long)data_end,
           (unsigned long long)(data_sectors * 512 / (1024 * 1024)));

    /* Create or load image */
    int incremental = 0;
    uint8_t *image = NULL;

    /* Check for incremental update */
    if (!args->reset && args->sysroot) {
        FILE *f = fopen(args->output, "rb");
        if (f) {
            fseek(f, 0, SEEK_END);
            long existing_size = ftell(f);
            fclose(f);
            if (existing_size > 0 && (size_t)existing_size == image_size) {
                incremental = 1;
            }
        }
    }

    if (incremental) {
        size_t dummy;
        image = read_file(args->output, &dummy);
        if (!image) fatal("cannot read existing image '%s'", args->output);
        printf("\nIncremental update mode (use --reset for full rebuild)\n");
    } else {
        image = calloc(1, image_size);
        if (!image) fatal("out of memory for image (%zu bytes)", image_size);
        if (args->reset)
            printf("\nFull rebuild (--reset)\n");
    }

    /* Always write GPT + ESP (boot sectors change with kernel updates) */
    write_protective_mbr(image, total_sectors);

    GptPartition parts[2];
    guid_esp(parts[0].type_guid);
    guid_random(parts[0].unique_guid);
    parts[0].first_lba = esp_start;
    parts[0].last_lba = esp_end;
    parts[0].name = "EFI System";

    guid_basic_data(parts[1].type_guid);
    guid_random(parts[1].unique_guid);
    parts[1].first_lba = data_start;
    parts[1].last_lba = data_end;
    parts[1].name = "anyOS Data";

    create_gpt(image, total_sectors, parts, 2);

    /* ESP as FAT16 — always rebuild (small, contains bootloader + kernel) */
    printf("\nESP filesystem:\n");
    Fat16 esp_fat;
    fat16_init(&esp_fat, image, (uint32_t)esp_start, (uint32_t)esp_sectors, 1);
    fat16_write_bpb(&esp_fat);
    fat16_init_fat(&esp_fat);

    /* Create /EFI/BOOT/BOOTX64.EFI */
    uint32_t efi_dir = fat16_create_dir(&esp_fat, 0, "EFI", 1);
    uint32_t boot_dir = fat16_create_dir(&esp_fat, efi_dir, "BOOT", 0);
    fat16_add_file(&esp_fat, boot_dir, "BOOTX64.EFI", efi_data, efi_size, 0);

    /* Place kernel on ESP */
    if (kernel_flat) {
        uint32_t sys_dir = fat16_create_dir(&esp_fat, 0, "System", 1);
        fat16_add_file(&esp_fat, sys_dir, "kernel.bin",
                       kernel_flat, kernel_flat_size, 0);
        printf("  Wrote kernel.bin to ESP (%zu bytes)\n", kernel_flat_size);
    }

    free(efi_data);

    /* Data partition as exFAT */
    printf("\nData filesystem (exFAT):\n");

    if (incremental) {
        /* Incremental: open existing FS, sync sysroot */
        ExFat data_exfat;
        exfat_open_existing(&data_exfat, image, (uint32_t)data_start);
        if (args->sysroot) {
            exfat_sync_sysroot(&data_exfat, args->sysroot);
        }
        exfat_flush(&data_exfat);
        exfat_free(&data_exfat);
    } else {
        /* Full rebuild: format + populate */
        ExFat data_exfat;
        exfat_init(&data_exfat, image, (uint32_t)data_start,
                   (uint32_t)data_sectors, 8);
        exfat_write_boot(&data_exfat);
        exfat_init_fs(&data_exfat);

        if (args->sysroot) {
            printf("  Populating from sysroot: %s\n", args->sysroot);
            exfat_populate_sysroot(&data_exfat, args->sysroot);
        }

        exfat_flush(&data_exfat);
        exfat_free(&data_exfat);
    }

    if (kernel_flat) free(kernel_flat);

    /* Write image */
    FILE *fp = fopen(args->output, "wb");
    if (!fp) fatal("cannot create '%s'", args->output);
    fwrite(image, 1, image_size, fp);
    fclose(fp);
    free(image);

    printf("\nUEFI disk image %s: %s (%d MiB)\n",
           incremental ? "updated" : "created", args->output, args->image_size);
}

/* ── Usage ────────────────────────────────────────────────────────────── */

static void usage(void) {
    fprintf(stderr,
        "mkimage — anyOS disk image builder\n"
        "\n"
        "BIOS mode (default):\n"
        "  mkimage --stage1 FILE --stage2 FILE --kernel FILE\n"
        "          --output FILE [--sysroot DIR] [--image-size N]\n"
        "          [--fs-start SECTOR] [--reset]\n"
        "\n"
        "UEFI mode:\n"
        "  mkimage --uefi --bootloader FILE --kernel FILE\n"
        "          --output FILE [--sysroot DIR] [--image-size N] [--reset]\n"
        "\n"
        "ISO mode:\n"
        "  mkimage --iso --stage1 FILE --stage2 FILE --kernel FILE\n"
        "          --output FILE [--sysroot DIR]\n"
        "\n"
        "Options:\n"
        "  --reset   Force full image rebuild (default: incremental update)\n"
    );
    exit(1);
}

/* ── Argument parsing ─────────────────────────────────────────────────── */

static int parse_args(int argc, char **argv, Args *args) {
    memset(args, 0, sizeof(*args));
    args->image_size = 64;
    args->fs_start = 8192;

    int i = 1;
    while (i < argc) {
        if (strcmp(argv[i], "--uefi") == 0) {
            args->mode = 1;
        } else if (strcmp(argv[i], "--iso") == 0) {
            args->mode = 2;
        } else if (strcmp(argv[i], "--stage1") == 0 && i + 1 < argc) {
            args->stage1 = argv[++i];
        } else if (strcmp(argv[i], "--stage2") == 0 && i + 1 < argc) {
            args->stage2 = argv[++i];
        } else if (strcmp(argv[i], "--kernel") == 0 && i + 1 < argc) {
            args->kernel = argv[++i];
        } else if (strcmp(argv[i], "--bootloader") == 0 && i + 1 < argc) {
            args->bootloader = argv[++i];
        } else if (strcmp(argv[i], "--output") == 0 && i + 1 < argc) {
            args->output = argv[++i];
        } else if (strcmp(argv[i], "--sysroot") == 0 && i + 1 < argc) {
            args->sysroot = argv[++i];
        } else if (strcmp(argv[i], "--image-size") == 0 && i + 1 < argc) {
            args->image_size = atoi(argv[++i]);
        } else if (strcmp(argv[i], "--fs-start") == 0 && i + 1 < argc) {
            args->fs_start = atoi(argv[++i]);
        } else if (strcmp(argv[i], "--reset") == 0) {
            args->reset = 1;
        } else if (strcmp(argv[i], "-h") == 0 ||
                   strcmp(argv[i], "--help") == 0) {
            usage();
        } else {
            fprintf(stderr, "mkimage: unknown option '%s'\n", argv[i]);
            usage();
        }
        i++;
    }

    if (!args->output) {
        fprintf(stderr, "mkimage: --output is required\n");
        usage();
    }

    return 0;
}

/* ── Main ─────────────────────────────────────────────────────────────── */

int main(int argc, char **argv) {
    if (argc < 2) usage();

    srand((unsigned)time(NULL));

    Args args;
    parse_args(argc, argv, &args);

    if (args.mode == 2) {
        /* ISO mode */
        if (!args.stage1 || !args.stage2 || !args.kernel)
            fatal("--stage1, --stage2, and --kernel required for ISO mode");
        create_iso_image(&args);
    } else if (args.mode == 1) {
        /* UEFI mode */
        create_uefi_image(&args);
    } else {
        /* BIOS mode */
        if (!args.stage1 || !args.stage2 || !args.kernel)
            fatal("--stage1, --stage2, and --kernel required for BIOS mode");
        create_bios_image(&args);
    }

    return 0;
}
