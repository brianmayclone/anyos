/*
 * arm64_image.c - QEMU virt ARM64 data disk image writer.
 */

#include "mkimage.h"

void create_arm64_image(const Args *args)
{
    size_t image_size = (size_t)args->image_size * 1024 * 1024;
    uint64_t total_sectors = image_size / SECTOR_SIZE;

    printf("ARM64 image: %d MiB (%llu sectors)\n",
           args->image_size, (unsigned long long)total_sectors);

    uint64_t data_start = 2048;
    uint32_t entry_sectors = (GPT_ENTRY_COUNT * GPT_ENTRY_SIZE + 511) / 512;
    uint64_t data_end = total_sectors - 1 - entry_sectors - 1;
    uint64_t data_sectors = data_end - data_start + 1;

    printf("\nPartition layout:\n");
    printf("  Data: sectors %llu-%llu (%llu MiB)\n",
           (unsigned long long)data_start, (unsigned long long)data_end,
           (unsigned long long)(data_sectors * SECTOR_SIZE / (1024 * 1024)));

    int incremental = 0;
    uint8_t *image = NULL;

    if (!args->reset && args->sysroot) {
        FILE *f = fopen(args->output, "rb");
        if (f) {
            fseek(f, 0, SEEK_END);
            long existing_size = ftell(f);
            fclose(f);
            if (existing_size > 0 && (size_t)existing_size == image_size)
                incremental = 1;
        }
    }

    if (incremental) {
        size_t dummy;
        image = read_file(args->output, &dummy);
        if (!image)
            fatal("cannot read existing image '%s'", args->output);
        printf("\nIncremental update mode (use --reset for full rebuild)\n");
    } else {
        image = calloc(1, image_size);
        if (!image)
            fatal("out of memory for image (%zu bytes)", image_size);
        if (args->reset)
            printf("\nFull rebuild (--reset)\n");
    }

    write_protective_mbr(image, total_sectors);

    GptPartition parts[1];
    guid_basic_data(parts[0].type_guid);
    guid_random(parts[0].unique_guid);
    parts[0].first_lba = data_start;
    parts[0].last_lba = data_end;
    parts[0].name = "anyOS Data";

    create_gpt(image, total_sectors, parts, 1);

    printf("\nData filesystem (exFAT):\n");
    if (incremental) {
        ExFat data_exfat;
        exfat_open_existing(&data_exfat, image, (uint32_t)data_start);
        if (args->sysroot)
            exfat_sync_sysroot(&data_exfat, args->sysroot);
        exfat_flush(&data_exfat);
        exfat_free(&data_exfat);
    } else {
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

    FILE *fp = fopen(args->output, "wb");
    if (!fp)
        fatal("cannot create '%s'", args->output);
    fwrite(image, 1, image_size, fp);
    fclose(fp);
    free(image);

    printf("\nARM64 disk image %s: %s (%d MiB)\n",
           incremental ? "updated" : "created", args->output, args->image_size);
}
