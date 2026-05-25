/*
 * uefi_image.c - GPT + ESP image writer.
 */

#include "mkimage.h"

#define UEFI_ESP_START_LBA 2048ULL
#define UEFI_ESP_MIN_MIB 32ULL
#define UEFI_ESP_SLACK_BYTES (8ULL * 1024ULL * 1024ULL)
#define FAT16_MAX_CLUSTERS 65524U

static uint64_t bytes_to_sectors(uint64_t bytes)
{
    return (bytes + SECTOR_SIZE - 1) / SECTOR_SIZE;
}

static uint64_t esp_size_sectors(size_t efi_size, size_t kernel_size)
{
    uint64_t min_bytes = UEFI_ESP_MIN_MIB * 1024ULL * 1024ULL;
    uint64_t payload = (uint64_t)efi_size + (uint64_t)kernel_size
                     + UEFI_ESP_SLACK_BYTES;
    uint64_t esp_bytes = payload > min_bytes ? payload : min_bytes;

    esp_bytes = ALIGN_UP(esp_bytes, 1024ULL * 1024ULL);
    return bytes_to_sectors(esp_bytes);
}

static uint32_t fat16_cluster_count_for(uint64_t fs_sectors, uint32_t spc)
{
    const uint32_t reserved = 1;
    const uint32_t num_fats = 2;
    const uint32_t root_dir_sectors =
        (FAT16_MAX_ROOT_ENTRIES * 32 + SECTOR_SIZE - 1) / SECTOR_SIZE;

    if (spc == 0 || fs_sectors <= reserved + root_dir_sectors)
        return 0;

    uint64_t data_sectors = fs_sectors - reserved - root_dir_sectors;
    uint64_t clusters = data_sectors / spc;
    uint64_t fat_size = (clusters * 2 + SECTOR_SIZE - 1) / SECTOR_SIZE;

    if (fs_sectors <= reserved + num_fats * fat_size + root_dir_sectors)
        return 0;

    data_sectors = fs_sectors - reserved - num_fats * fat_size
                 - root_dir_sectors;
    clusters = data_sectors / spc;
    if (clusters > FAT16_MAX_CLUSTERS)
        return FAT16_MAX_CLUSTERS + 1;
    return (uint32_t)clusters;
}

static uint32_t choose_fat16_spc(uint64_t fs_sectors)
{
    static const uint32_t candidates[] = {1, 2, 4, 8, 16, 32, 64};
    size_t i;

    for (i = 0; i < sizeof(candidates) / sizeof(candidates[0]); ++i) {
        uint32_t clusters = fat16_cluster_count_for(fs_sectors, candidates[i]);
        if (clusters >= 4085 && clusters <= FAT16_MAX_CLUSTERS)
            return candidates[i];
    }

    fatal("UEFI ESP size cannot be represented as FAT16 (%llu sectors)",
          (unsigned long long)fs_sectors);
    return 0;
}

void create_uefi_image(const Args *args)
{
    if (!args->bootloader)
        fatal("--bootloader required for UEFI mode");

    size_t efi_size;
    uint8_t *efi_data = read_file(args->bootloader, &efi_size);
    if (!efi_data)
        fatal("cannot read bootloader");

    uint8_t *kernel_flat = NULL;
    size_t kernel_flat_size = 0;
    if (args->kernel) {
        size_t k_size;
        uint8_t *kelf = read_file(args->kernel, &k_size);
        if (!kelf)
            fatal("cannot read kernel");
        printf("Kernel ELF: %zu bytes\n", k_size);
        kernel_flat = elf_to_flat(kelf, k_size, 0x00100000, &kernel_flat_size);
        if (!kernel_flat)
            fatal("kernel ELF conversion failed");
        free(kelf);
    }

    size_t image_size = (size_t)args->image_size * 1024 * 1024;
    uint64_t total_sectors = image_size / SECTOR_SIZE;

    printf("\nUEFI image: %d MiB (%llu sectors)\n",
           args->image_size, (unsigned long long)total_sectors);
    printf("EFI bootloader: %zu bytes\n", efi_size);
    if (kernel_flat)
        printf("Kernel flat binary: %zu bytes\n", kernel_flat_size);

    uint64_t esp_start = UEFI_ESP_START_LBA;
    uint64_t esp_sectors = esp_size_sectors(efi_size, kernel_flat_size);
    uint32_t esp_spc = choose_fat16_spc(esp_sectors);
    uint64_t esp_end = esp_start + esp_sectors - 1;
    uint64_t data_start = esp_start + esp_sectors;
    uint32_t entry_sectors = (GPT_ENTRY_COUNT * GPT_ENTRY_SIZE + 511) / 512;
    uint64_t data_end = total_sectors - 1 - entry_sectors - 1;

    if (data_start > data_end)
        fatal("UEFI image too small: ESP needs %llu MiB plus GPT/data space",
              (unsigned long long)(esp_sectors * SECTOR_SIZE / (1024 * 1024)));

    uint64_t data_sectors = data_end - data_start + 1;

    printf("\nPartition layout:\n");
    printf("  ESP:  sectors %llu-%llu (%llu MiB, FAT16 spc=%u)\n",
           (unsigned long long)esp_start, (unsigned long long)esp_end,
           (unsigned long long)(esp_sectors * SECTOR_SIZE / (1024 * 1024)),
           esp_spc);
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

    printf("\nESP filesystem:\n");
    Fat16 esp_fat;
    fat16_init(&esp_fat, image, (uint32_t)esp_start,
               (uint32_t)esp_sectors, esp_spc);
    fat16_write_bpb(&esp_fat);
    fat16_init_fat(&esp_fat);
    fat16_add_volume_label(&esp_fat, "ANYOS");

    uint32_t efi_dir = fat16_create_dir(&esp_fat, 0, "EFI", 1);
    uint32_t boot_dir = fat16_create_dir(&esp_fat, efi_dir, "BOOT", 0);
    fat16_add_file(&esp_fat, boot_dir, "BOOTX64.EFI", efi_data, efi_size, 0);

    if (kernel_flat) {
        uint32_t sys_dir = fat16_create_dir(&esp_fat, 0, "System", 1);
        fat16_add_file(&esp_fat, sys_dir, "kernel.bin",
                       kernel_flat, kernel_flat_size, 0);
        printf("  Wrote kernel.bin to ESP (%zu bytes)\n", kernel_flat_size);
    }

    free(efi_data);

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

    if (kernel_flat)
        free(kernel_flat);

    FILE *fp = fopen(args->output, "wb");
    if (!fp)
        fatal("cannot create '%s'", args->output);
    fwrite(image, 1, image_size, fp);
    fclose(fp);
    free(image);

    printf("\nUEFI disk image %s: %s (%d MiB)\n",
           incremental ? "updated" : "created", args->output, args->image_size);
}
