/*
 * exfat.c — exFAT filesystem formatter
 *
 * Faithful C99 port of the ExFatFormatter class from mkimage.py.
 * Written in C99 for TCC compatibility.
 */

#include "mkimage.h"

#include <dirent.h>
#include <sys/stat.h>

/* ═══════════════════════════════════════════════════════════════════════════
 * Internal helpers
 * ═══════════════════════════════════════════════════════════════════════════ */

/* Return byte offset in image for a filesystem-relative sector. */
static uint32_t exfat_abs_offset(const ExFat *fs, uint32_t rel_sector)
{
    return (fs->fs_start + rel_sector) * SECTOR_SIZE;
}

/* Write 512 bytes to a filesystem-relative sector. */
static void exfat_write_sector(ExFat *fs, uint32_t rel, const uint8_t *data)
{
    uint32_t offset = exfat_abs_offset(fs, rel);
    memcpy(fs->image + offset, data, SECTOR_SIZE);
}

/* Read 512 bytes from a filesystem-relative sector into out[]. */
static void exfat_read_sector(const ExFat *fs, uint32_t rel, uint8_t *out)
{
    uint32_t offset = exfat_abs_offset(fs, rel);
    memcpy(out, fs->image + offset, SECTOR_SIZE);
}

/* Convert cluster number (>=2) to filesystem-relative sector. */
static uint32_t exfat_cluster_to_sector(const ExFat *fs, uint32_t cluster)
{
    return fs->cluster_heap_offset + (cluster - 2) * fs->spc;
}

/* Write data to a single cluster (spc sectors), zero-padding any remainder. */
static void exfat_write_cluster(ExFat *fs, uint32_t cluster,
                                const uint8_t *data, uint32_t len)
{
    uint32_t sector = exfat_cluster_to_sector(fs, cluster);
    uint32_t s;
    uint8_t  sector_data[SECTOR_SIZE];

    for (s = 0; s < fs->spc; ++s) {
        uint32_t s_offset = s * SECTOR_SIZE;
        if (s_offset >= len) {
            memset(sector_data, 0, SECTOR_SIZE);
        } else {
            uint32_t chunk = len - s_offset;
            if (chunk > SECTOR_SIZE) chunk = SECTOR_SIZE;
            memcpy(sector_data, data + s_offset, chunk);
            if (chunk < SECTOR_SIZE)
                memset(sector_data + chunk, 0, SECTOR_SIZE - chunk);
        }
        exfat_write_sector(fs, sector + s, sector_data);
    }
}

/* Allocate a single cluster: mark bitmap + write EOC to FAT cache.
 * Scans bitmap for the next free cluster (required after incremental frees). */
static uint32_t exfat_alloc_cluster(ExFat *fs)
{
    uint32_t c   = fs->next_cluster;
    uint32_t idx;

    /* Scan forward for a free cluster (bitmap bit = 0) */
    while (c - 2 < fs->cluster_count) {
        idx = c - 2;
        if (!(fs->bitmap[idx / 8] & (1u << (idx % 8))))
            break;
        c++;
    }

    if (c - 2 >= fs->cluster_count)
        fatal("exFAT: out of clusters");

    fs->next_cluster = c + 1;

    /* Mark bitmap */
    idx = c - 2;
    fs->bitmap[idx / 8] |= (uint8_t)(1u << (idx % 8));

    /* Write FAT EOC */
    write_le32(fs->fat_cache + c * 4, EXFAT_EOC);

    return c;
}

/* Allocate `count` contiguous clusters.  Does NOT write FAT chain
 * (for NoFatChain / contiguous files).  Returns first cluster.
 * Scans bitmap for a contiguous free run (required after incremental frees). */
static uint32_t exfat_alloc_contiguous(ExFat *fs, uint32_t count)
{
    uint32_t start;
    uint32_t i;

    if (count == 0)
        return 0;

    /* Find a contiguous run of `count` free clusters */
    start = fs->next_cluster;
    while (start - 2 + count <= fs->cluster_count) {
        int all_free = 1;
        for (i = 0; i < count; ++i) {
            uint32_t idx = start - 2 + i;
            if (fs->bitmap[idx / 8] & (1u << (idx % 8))) {
                start = start + i + 1;  /* Skip past the used cluster */
                all_free = 0;
                break;
            }
        }
        if (all_free)
            break;
    }

    if (start - 2 + count > fs->cluster_count)
        fatal("exFAT: out of clusters (contiguous, need %u)", count);

    /* Mark bitmap for all clusters in the run */
    for (i = 0; i < count; ++i) {
        uint32_t idx = start - 2 + i;
        fs->bitmap[idx / 8] |= (uint8_t)(1u << (idx % 8));
        /* No FAT chain — leave FAT entries as 0 */
    }

    fs->next_cluster = start + count;
    return start;
}

/* Allocate `count` clusters with a FAT chain.  Returns first cluster. */
static uint32_t exfat_alloc_chained(ExFat *fs, uint32_t count)
{
    uint32_t first;
    uint32_t prev;
    uint32_t i;

    if (count == 0)
        return 0;

    first = exfat_alloc_cluster(fs);
    prev  = first;
    for (i = 1; i < count; ++i) {
        uint32_t c = exfat_alloc_cluster(fs);
        write_le32(fs->fat_cache + prev * 4, c);
        prev = c;
    }
    return first;
}

/* Write data to contiguous clusters starting at first_cluster. */
static void exfat_write_contiguous(ExFat *fs, uint32_t first_cluster,
                                   const uint8_t *data, uint32_t len)
{
    uint32_t offset  = 0;
    uint32_t cluster = first_cluster;

    while (offset < len) {
        uint32_t chunk = len - offset;
        if (chunk > fs->cluster_size)
            chunk = fs->cluster_size;
        exfat_write_cluster(fs, cluster, data + offset, chunk);
        offset += fs->cluster_size;
        cluster++;
    }
}

/* ── Entry set helpers ────────────────────────────────────────────────────── */

/*
 * Compute exFAT entry set checksum.
 * Mirrors ExFatFormatter._entry_set_checksum().
 */
static uint16_t exfat_entry_set_checksum(const uint8_t *data, uint32_t len)
{
    uint32_t i;
    uint16_t checksum = 0;

    for (i = 0; i < len; ++i) {
        if (i == 2 || i == 3)   /* skip SetChecksum field */
            continue;
        checksum = (uint16_t)(((checksum << 15) | (checksum >> 1)) + data[i]);
    }
    return checksum;
}

/*
 * Compute exFAT name hash over UTF-16 characters (upper-cased ASCII range).
 * Mirrors ExFatFormatter._name_hash().
 */
static uint16_t exfat_name_hash(const uint16_t *utf16, uint32_t len)
{
    uint32_t i;
    uint16_t h = 0;

    for (i = 0; i < len; ++i) {
        uint16_t uc = utf16[i];
        if (uc >= 0x61 && uc <= 0x7A)
            uc = (uint16_t)(uc - 0x20);   /* a-z → A-Z */

        h = (uint16_t)(((h << 15) | (h >> 1)) + (uc & 0xFF));
        h = (uint16_t)(((h << 15) | (h >> 1)) + (uc >> 8));
    }
    return h;
}

/*
 * Build a complete exFAT directory entry set (File + Stream + FileName entries).
 *
 * name         — UTF-8 / ASCII filename string
 * attrs        — file attributes (EXFAT_ATTR_DIR or EXFAT_ATTR_ARCHIVE)
 * first_cluster — first data cluster (0 for empty file)
 * data_length  — file/dir data length in bytes
 * contiguous   — set EXFAT_FLAG_CONTIGUOUS if true
 * uid, gid, mode — VFS permissions stored in reserved fields
 * out_buf      — caller-supplied buffer (must be >= (2 + fn_entries) * 32 bytes)
 * out_len      — receives byte count written
 *
 * Mirrors ExFatFormatter._build_entry_set().
 */
static void exfat_build_entry_set(const char *name, uint16_t attrs,
                                  uint32_t first_cluster, uint64_t data_length,
                                  int contiguous,
                                  uint16_t uid, uint16_t gid, uint16_t mode,
                                  uint8_t *out_buf, uint32_t *out_len)
{
    /* Build UTF-16 array from name (ASCII only) */
    uint16_t utf16[256];
    uint32_t name_len = 0;
    uint32_t fn_entries;
    uint32_t secondary;
    uint32_t total;
    uint32_t fi;
    uint16_t nh;
    uint16_t checksum;
    uint8_t  flags;

    {
        uint32_t i;
        for (i = 0; name[i] && i < 255; ++i)
            utf16[i] = (uint16_t)(unsigned char)name[i];
        name_len = i;
    }

    fn_entries = (name_len + 14) / 15;
    secondary  = 1 + fn_entries;   /* Stream + FileName(s) */
    total      = 1 + secondary;

    memset(out_buf, 0, total * 32);

    /* ── File Directory Entry (0x85) ─────────────────────────────── */
    out_buf[0] = EXFAT_ENTRY_FILE;
    out_buf[1] = (uint8_t)secondary;
    /* [2..3] = SetChecksum — filled last */
    write_le16(out_buf + 4, attrs);
    /* Reserved / VFS fields: uid at [6], gid at [8], mode at [10] */
    write_le16(out_buf + 6,  uid);
    write_le16(out_buf + 8,  gid);
    write_le16(out_buf + 10, mode);

    /* ── Stream Extension (0xC0) ──────────────────────────────────── */
    {
        uint32_t s = 32;
        out_buf[s] = EXFAT_ENTRY_STREAM;
        flags = 0x01;   /* AllocationPossible */
        if (contiguous)
            flags |= EXFAT_FLAG_CONTIGUOUS;
        out_buf[s + 1] = flags;
        out_buf[s + 3] = (uint8_t)name_len;
        nh = exfat_name_hash(utf16, name_len);
        write_le16(out_buf + s + 4, nh);
        write_le64(out_buf + s + 8,  data_length);  /* ValidDataLength */
        write_le32(out_buf + s + 20, first_cluster);
        write_le64(out_buf + s + 24, data_length);  /* DataLength */
    }

    /* ── FileName entries (0xC1) ──────────────────────────────────── */
    for (fi = 0; fi < fn_entries; ++fi) {
        uint32_t f = (2 + fi) * 32;
        uint32_t j;
        out_buf[f] = EXFAT_ENTRY_FILENAME;
        for (j = 0; j < 15; ++j) {
            uint32_t ci = fi * 15 + j;
            uint16_t ch = (ci < name_len) ? utf16[ci] : 0x0000;
            write_le16(out_buf + f + 2 + j * 2, ch);
        }
    }

    /* Compute and store checksum */
    checksum = exfat_entry_set_checksum(out_buf, total * 32);
    write_le16(out_buf + 2, checksum);

    *out_len = total * 32;
}

/*
 * Find free space in a directory cluster chain and write the entry set.
 * Extends the directory with a new cluster when needed.
 *
 * Mirrors ExFatFormatter._add_entry_to_dir().
 */
static void exfat_add_entry_to_dir(ExFat *fs, uint32_t dir_cluster,
                                   const uint8_t *entry_set, uint32_t entry_set_len)
{
    uint32_t entry_count = entry_set_len / 32;
    uint32_t cluster     = dir_cluster;

    while (1) {
        uint32_t  sector   = exfat_cluster_to_sector(fs, cluster);
        uint8_t  *dir_data = (uint8_t *)malloc(fs->cluster_size);
        uint32_t  s;
        int       run_start;
        uint32_t  run_len;
        uint32_t  idx;
        uint32_t  fat_val;

        if (!dir_data)
            fatal("exfat_add_entry_to_dir: malloc(%u) failed", fs->cluster_size);

        /* Read current directory cluster */
        for (s = 0; s < fs->spc; ++s)
            exfat_read_sector(fs, sector + s, dir_data + s * SECTOR_SIZE);

        /* Search for a contiguous run of free / deleted entries */
        run_start = -1;
        run_len   = 0;

        for (idx = 0; idx < fs->cluster_size / 32; ++idx) {
            uint32_t off   = idx * 32;
            uint8_t  etype = dir_data[off];
            int      is_free = (etype == 0x00) ||
                               ((etype & 0x80) == 0 && etype != 0);

            if (is_free) {
                if (run_len == 0)
                    run_start = (int)idx;
                run_len++;

                if (run_len >= entry_count) {
                    /* Found sufficient space */
                    uint32_t write_off = (uint32_t)run_start * 32;
                    memcpy(dir_data + write_off, entry_set, entry_set_len);
                    for (s = 0; s < fs->spc; ++s)
                        exfat_write_sector(fs, sector + s,
                                           dir_data + s * SECTOR_SIZE);
                    free(dir_data);
                    return;
                }

                if (etype == 0x00) {
                    /* End-of-directory marker — check remaining space in cluster */
                    uint32_t remaining = fs->cluster_size / 32 - (uint32_t)run_start;
                    if (remaining >= entry_count) {
                        uint32_t write_off = (uint32_t)run_start * 32;
                        memcpy(dir_data + write_off, entry_set, entry_set_len);
                        for (s = 0; s < fs->spc; ++s)
                            exfat_write_sector(fs, sector + s,
                                               dir_data + s * SECTOR_SIZE);
                        free(dir_data);
                        return;
                    }
                    break;   /* Need a new cluster */
                }
            } else {
                run_len   = 0;
                run_start = -1;
            }
        }

        free(dir_data);

        /* Check FAT for next cluster */
        fat_val = read_le32(fs->fat_cache + cluster * 4);
        if (fat_val >= 0xFFFFFFF8u || fat_val == 0) {
            /* Extend directory with a new cluster */
            uint32_t  new_cluster = exfat_alloc_cluster(fs);
            uint8_t  *new_data    = (uint8_t *)calloc(1, fs->cluster_size);
            if (!new_data)
                fatal("exfat_add_entry_to_dir: calloc failed");
            write_le32(fs->fat_cache + cluster * 4, new_cluster);
            memcpy(new_data, entry_set, entry_set_len);
            exfat_write_cluster(fs, new_cluster, new_data, fs->cluster_size);
            free(new_data);
            return;
        }
        cluster = fat_val;
    }
}

/* ═══════════════════════════════════════════════════════════════════════════
 * exFAT public API
 * ═══════════════════════════════════════════════════════════════════════════ */

/*
 * Initialise exFAT parameters and allocate in-memory caches.
 * Mirrors ExFatFormatter.__init__().
 */
void exfat_init(ExFat *fs, uint8_t *image, uint32_t fs_start,
                uint32_t fs_sectors, uint32_t spc)
{
    uint32_t est_clusters;
    uint32_t fat_bytes;

    fs->image      = image;
    fs->fs_start   = fs_start;
    fs->fs_sectors = fs_sectors;
    fs->spc        = spc;
    fs->cluster_size = spc * SECTOR_SIZE;

    /* Layout: Main Boot Region (12) + Backup (12) + alignment = FAT at sector 32 */
    fs->fat_offset = 32;

    /*
     * Iterative layout computation (mirrors Python exactly):
     *   cluster_count = (fs_sectors - cluster_heap_offset) / spc
     *   cluster_heap_offset = fat_offset + fat_length
     *   fat_length = ceil((cluster_count + 2) * 4 / 512)
     */

    /* First estimate */
    est_clusters         = (fs_sectors - fs->fat_offset) / spc;
    fat_bytes            = (est_clusters + 2) * 4;
    fs->fat_length       = (fat_bytes + SECTOR_SIZE - 1) / SECTOR_SIZE;
    fs->cluster_heap_offset = fs->fat_offset + fs->fat_length;
    fs->cluster_count    = (fs_sectors - fs->cluster_heap_offset) / spc;

    /* Recompute with final cluster_count */
    fat_bytes            = (fs->cluster_count + 2) * 4;
    fs->fat_length       = (fat_bytes + SECTOR_SIZE - 1) / SECTOR_SIZE;
    fs->cluster_heap_offset = fs->fat_offset + fs->fat_length;

    /* Next free cluster starts at 2 */
    fs->next_cluster    = 2;
    fs->bitmap_cluster  = 0;
    fs->root_cluster    = 0;

    /* In-memory FAT cache: (cluster_count + 2) * 4 bytes */
    fs->fat_cache = (uint8_t *)calloc(1, (fs->cluster_count + 2) * 4);
    if (!fs->fat_cache)
        fatal("exfat_init: calloc fat_cache failed");

    /* Entry 0: media type, Entry 1: end-marker */
    write_le32(fs->fat_cache + 0, 0xFFFFFFF8u);
    write_le32(fs->fat_cache + 4, 0xFFFFFFFFu);

    /* In-memory allocation bitmap */
    fs->bitmap_bytes = (fs->cluster_count + 7) / 8;
    fs->bitmap = (uint8_t *)calloc(1, fs->bitmap_bytes);
    if (!fs->bitmap)
        fatal("exfat_init: calloc bitmap failed");

    printf("  exFAT: %u clusters, %u bytes/cluster\n",
           fs->cluster_count, fs->cluster_size);
    printf("  exFAT: FAT at sector +%u (%u sectors), data at sector +%u\n",
           fs->fat_offset, fs->fat_length, fs->cluster_heap_offset);
}

/*
 * Write the exFAT VBR and backup boot region.
 * Mirrors ExFatFormatter.write_boot_sector().
 */
void exfat_write_boot(ExFat *fs)
{
    uint8_t vbr[SECTOR_SIZE];
    uint8_t ext[SECTOR_SIZE];   /* extended boot sector template */
    uint8_t oem[SECTOR_SIZE];
    uint8_t reserved_sec[SECTOR_SIZE];
    uint8_t boot_region[11 * SECTOR_SIZE];
    uint8_t cs_sector[SECTOR_SIZE];
    uint32_t checksum;
    int      i;

    memset(vbr,          0, SECTOR_SIZE);
    memset(oem,          0, SECTOR_SIZE);
    memset(reserved_sec, 0, SECTOR_SIZE);

    /* JumpBoot */
    vbr[0] = 0xEB; vbr[1] = 0x76; vbr[2] = 0x90;
    /* FileSystemName */
    memcpy(vbr + 3, "EXFAT   ", 8);
    /* MustBeZero [11..63] — already zero */

    /* PartitionOffset — set to fs_start (8 bytes / u64) */
    write_le64(vbr + 64, (uint64_t)fs->fs_start);
    /* VolumeLength */
    write_le64(vbr + 72, (uint64_t)fs->fs_sectors);
    /* FatOffset */
    write_le32(vbr + 80, fs->fat_offset);
    /* FatLength */
    write_le32(vbr + 84, fs->fat_length);
    /* ClusterHeapOffset */
    write_le32(vbr + 88, fs->cluster_heap_offset);
    /* ClusterCount */
    write_le32(vbr + 92, fs->cluster_count);
    /* FirstClusterOfRootDirectory — placeholder (updated by exfat_init_fs) */
    write_le32(vbr + 96, 4);
    /* VolumeSerialNumber */
    write_le32(vbr + 100, 0x414E594Fu);  /* "ANYO" */
    /* FileSystemRevision (1.00) */
    write_le16(vbr + 104, 0x0100);
    /* VolumeFlags */
    write_le16(vbr + 106, 0);
    /* BytesPerSectorShift: 2^9 = 512 */
    vbr[108] = 9;
    /* SectorsPerClusterShift: 2^spc_shift = spc */
    {
        uint8_t shift = 0;
        uint32_t s    = fs->spc;
        while (s > 1) { s >>= 1; shift++; }
        vbr[109] = shift;
    }
    /* NumberOfFats */
    vbr[110] = 1;
    /* DriveSelect */
    vbr[111] = 0x80;
    /* PercentInUse: unknown */
    vbr[112] = 0xFF;
    /* BootSignature */
    vbr[510] = 0x55;
    vbr[511] = 0xAA;

    /* Extended boot sectors 1-8: zeros + 0x55AA signature */
    memset(ext, 0, SECTOR_SIZE);
    ext[510] = 0x55;
    ext[511] = 0xAA;

    /*
     * Assemble sectors 0-10 into boot_region for checksum computation.
     * Mirrors Python: boot_region = vbr + 8*ext + oem + reserved
     */
    memcpy(boot_region, vbr, SECTOR_SIZE);
    for (i = 0; i < 8; ++i)
        memcpy(boot_region + (1 + i) * SECTOR_SIZE, ext, SECTOR_SIZE);
    memcpy(boot_region + 9  * SECTOR_SIZE, oem,          SECTOR_SIZE);
    memcpy(boot_region + 10 * SECTOR_SIZE, reserved_sec, SECTOR_SIZE);

    /* Compute boot region checksum (skip bytes 106, 107, 112) */
    checksum = 0;
    {
        uint32_t byte_idx;
        for (byte_idx = 0; byte_idx < 11 * SECTOR_SIZE; ++byte_idx) {
            if (byte_idx == 106 || byte_idx == 107 || byte_idx == 112)
                continue;
            checksum = (((checksum & 1u) << 31) | (checksum >> 1)) +
                       boot_region[byte_idx];
        }
    }

    /* Checksum sector (sector 11): repeated u32 */
    for (i = 0; i < SECTOR_SIZE / 4; ++i)
        write_le32(cs_sector + i * 4, checksum);

    /* ── Write Main Boot Region (sectors 0-11) ── */
    exfat_write_sector(fs, 0, vbr);
    for (i = 0; i < 8; ++i)
        exfat_write_sector(fs, 1 + i, ext);
    exfat_write_sector(fs, 9,  oem);
    exfat_write_sector(fs, 10, reserved_sec);
    exfat_write_sector(fs, 11, cs_sector);

    /* ── Write Backup Boot Region (sectors 12-23) ── */
    exfat_write_sector(fs, 12, vbr);
    for (i = 0; i < 8; ++i)
        exfat_write_sector(fs, 13 + i, ext);
    exfat_write_sector(fs, 21, oem);
    exfat_write_sector(fs, 22, reserved_sec);
    exfat_write_sector(fs, 23, cs_sector);

    printf("  exFAT: VBR written at sector %u\n", fs->fs_start);
}

/*
 * Initialise the exFAT filesystem structures: allocation bitmap, upcase table,
 * and root directory with their corresponding directory entries.
 * Mirrors ExFatFormatter.init_fs().
 */
void exfat_init_fs(ExFat *fs)
{
    uint32_t upcase_cluster;
    uint8_t *upcase_data;
    uint32_t upcase_len;
    uint8_t *root_data;
    uint32_t pos;
    uint32_t upcase_checksum;
    uint32_t i;

    /* Allocate cluster 2 for allocation bitmap */
    fs->bitmap_cluster = exfat_alloc_cluster(fs);   /* = 2 */
    /* Allocate cluster 3 for minimal upcase table */
    upcase_cluster     = exfat_alloc_cluster(fs);   /* = 3 */
    /* Allocate cluster 4 for root directory */
    fs->root_cluster   = exfat_alloc_cluster(fs);   /* = 4 */

    /* ── Write minimal upcase table (identity mapping for ASCII 0-127) ── */
    /* 128 UTF-16LE entries = 256 bytes, padded to cluster_size */
    upcase_len  = 128 * 2;
    upcase_data = (uint8_t *)calloc(1, fs->cluster_size);
    if (!upcase_data)
        fatal("exfat_init_fs: calloc upcase failed");

    for (i = 0; i < 128; ++i) {
        uint16_t ch = (uint16_t)i;
        if (ch >= 0x61 && ch <= 0x7A)
            ch = (uint16_t)(ch - 0x20);   /* a-z → A-Z */
        write_le16(upcase_data + i * 2, ch);
    }
    /* Rest of cluster stays zero (calloc) */
    exfat_write_cluster(fs, upcase_cluster, upcase_data, fs->cluster_size);

    /* Compute upcase table checksum (over the raw 256 bytes, not the padded cluster) */
    upcase_checksum = 0;
    for (i = 0; i < upcase_len; ++i) {
        upcase_checksum =
            (((upcase_checksum & 1u) << 31) | (upcase_checksum >> 1)) +
            upcase_data[i];
    }

    /* ── Build root directory ── */
    root_data = (uint8_t *)calloc(1, fs->cluster_size);
    if (!root_data)
        fatal("exfat_init_fs: calloc root_data failed");

    pos = 0;

    /* Allocation Bitmap entry (0x81) */
    {
        uint32_t bitmap_size = (fs->cluster_count + 7) / 8;
        root_data[pos]      = EXFAT_ENTRY_BITMAP;
        root_data[pos + 1]  = 0;   /* BitmapFlags: first bitmap */
        write_le32(root_data + pos + 20, fs->bitmap_cluster);
        write_le64(root_data + pos + 24, (uint64_t)bitmap_size);
        pos += 32;
    }

    /* Upcase Table entry (0x82) */
    root_data[pos] = EXFAT_ENTRY_UPCASE;
    write_le32(root_data + pos + 4,  upcase_checksum);
    write_le32(root_data + pos + 20, upcase_cluster);
    write_le64(root_data + pos + 24, (uint64_t)upcase_len);
    pos += 32;

    /* Volume Label entry (0x83): "anyOS" */
    {
        const char *label    = "anyOS";
        uint32_t    llen     = 5;
        uint32_t    li;
        root_data[pos]     = EXFAT_ENTRY_LABEL;
        root_data[pos + 1] = (uint8_t)llen;   /* CharacterCount */
        for (li = 0; li < llen; ++li)
            write_le16(root_data + pos + 2 + li * 2, (uint16_t)(unsigned char)label[li]);
        pos += 32;
    }
    (void)pos;   /* no more entries */

    exfat_write_cluster(fs, fs->root_cluster, root_data, fs->cluster_size);

    free(upcase_data);
    free(root_data);

    /* Update VBR root cluster field in both main and backup boot sectors */
    write_le32(fs->image + exfat_abs_offset(fs, 0)  + 96, fs->root_cluster);
    write_le32(fs->image + exfat_abs_offset(fs, 12) + 96, fs->root_cluster);

    printf("  exFAT: bitmap=cluster %u, upcase=cluster %u, root=cluster %u\n",
           fs->bitmap_cluster, upcase_cluster, fs->root_cluster);
}

/*
 * Create a subdirectory.  Returns the new directory's cluster.
 * Use parent==0 to add to the root directory.
 * Mirrors ExFatFormatter.create_directory().
 */
uint32_t exfat_create_dir(ExFat *fs, uint32_t parent, const char *name,
                          uint16_t uid, uint16_t gid, uint16_t mode)
{
    uint32_t dir_cluster;
    uint8_t  entry_buf[32 * (1 + 1 + ((255 + 14) / 15))];
    uint32_t entry_len;

    dir_cluster = exfat_alloc_cluster(fs);

    /* Initialise the new directory cluster with zeros */
    {
        uint8_t *zeros = (uint8_t *)calloc(1, fs->cluster_size);
        if (!zeros)
            fatal("exfat_create_dir: calloc failed");
        exfat_write_cluster(fs, dir_cluster, zeros, fs->cluster_size);
        free(zeros);
    }

    exfat_build_entry_set(name, EXFAT_ATTR_DIR, dir_cluster, 0,
                          0 /* not contiguous */, uid, gid, mode,
                          entry_buf, &entry_len);

    if (parent == 0)
        parent = fs->root_cluster;
    exfat_add_entry_to_dir(fs, parent, entry_buf, entry_len);

    return dir_cluster;
}

/*
 * Add a file to a directory.
 * Use parent==0 to add to the root directory.
 * Mirrors ExFatFormatter.add_file().
 */
void exfat_add_file(ExFat *fs, uint32_t parent, const char *name,
                    const uint8_t *data, size_t size,
                    uint16_t uid, uint16_t gid, uint16_t mode)
{
    uint8_t  entry_buf[32 * (1 + 1 + ((255 + 14) / 15))];
    uint32_t entry_len;

    if (parent == 0)
        parent = fs->root_cluster;

    if (size == 0) {
        exfat_build_entry_set(name, EXFAT_ATTR_ARCHIVE, 0, 0,
                              1 /* contiguous */, uid, gid, mode,
                              entry_buf, &entry_len);
        exfat_add_entry_to_dir(fs, parent, entry_buf, entry_len);
        return;
    }

    {
        uint32_t num_clusters =
            (uint32_t)((size + fs->cluster_size - 1) / fs->cluster_size);
        uint32_t first_cluster = exfat_alloc_contiguous(fs, num_clusters);

        exfat_write_contiguous(fs, first_cluster, data, (uint32_t)size);

        exfat_build_entry_set(name, EXFAT_ATTR_ARCHIVE, first_cluster,
                              (uint64_t)size,
                              1 /* contiguous */, uid, gid, mode,
                              entry_buf, &entry_len);
        exfat_add_entry_to_dir(fs, parent, entry_buf, entry_len);

        printf("    File: %s (%zu bytes, %u cluster(s), start=%u, contiguous)\n",
               name, size, num_clusters, first_cluster);
    }
}

/* ── Recursive sysroot population ─────────────────────────────────────────── */

/*
 * Root-only directories: contents get uid=0, gid=0, mode=0xF00.
 * Paths are relative to sysroot, using forward slashes.
 * Mirrors ExFatFormatter.ROOT_ONLY_DIRS.
 */
static const char * const ROOT_ONLY_DIRS[] = {
    "System/sbin",
    "System/users/perm",
    NULL
};

/*
 * Return 1 if virt_path matches or is under any ROOT_ONLY_DIRS entry.
 */
static int is_root_only(const char *virt_path)
{
    int i;
    for (i = 0; ROOT_ONLY_DIRS[i] != NULL; ++i) {
        const char *d   = ROOT_ONLY_DIRS[i];
        size_t      dlen = strlen(d);
        if (strcmp(virt_path, d) == 0)
            return 1;
        if (strncmp(virt_path, d, dlen) == 0 && virt_path[dlen] == '/')
            return 1;
    }
    return 0;
}

/*
 * Internal recursive worker.
 * Mirrors ExFatFormatter._populate_dir().
 */
static void exfat_populate_dir(ExFat *fs, const char *host_path,
                               uint32_t parent_cluster,
                               const char *virt_path)
{
    DIR           *d;
    struct dirent *ent;
    char         **names      = NULL;
    int            name_count = 0;
    int            name_cap   = 0;
    int            i;

    d = opendir(host_path);
    if (!d) {
        fprintf(stderr, "  WARNING: Cannot open directory %s\n", host_path);
        return;
    }

    /* Collect non-hidden entry names */
    while ((ent = readdir(d)) != NULL) {
        const char *n = ent->d_name;

        /* Skip "." and ".." (and all dot-files per Python: entry_name.startswith('.')) */
        if (n[0] == '.')
            continue;

        /* Grow array */
        if (name_count >= name_cap) {
            int    new_cap = (name_cap == 0) ? 64 : name_cap * 2;
            char **tmp     = (char **)realloc(names,
                                              (size_t)new_cap * sizeof(char *));
            if (!tmp)
                fatal("exfat_populate_dir: realloc failed");
            names    = tmp;
            name_cap = new_cap;
        }
        names[name_count++] = strdup(n);
    }
    closedir(d);

    /* Sort (ASCII order, matches Python sorted()) */
    for (i = 0; i < name_count - 1; ++i) {
        int j;
        for (j = i + 1; j < name_count; ++j) {
            if (strcmp(names[i], names[j]) > 0) {
                char *tmp  = names[i];
                names[i]   = names[j];
                names[j]   = tmp;
            }
        }
    }

    /* Process each entry */
    for (i = 0; i < name_count; ++i) {
        const char *entry_name = names[i];
        char        full_path[4096];
        char        child_virt[4096];
        struct stat st;
        uint16_t    uid, gid, mode;

        snprintf(full_path,  sizeof(full_path),  "%s/%s", host_path, entry_name);

        /* Build virtual path: "parentvirt/entryname" or just "entryname" at root */
        if (virt_path[0] == '\0')
            snprintf(child_virt, sizeof(child_virt), "%s", entry_name);
        else
            snprintf(child_virt, sizeof(child_virt), "%s/%s", virt_path, entry_name);

        if (stat(full_path, &st) != 0) {
            free(names[i]);
            continue;
        }

        /* Determine permissions */
        uid  = 0;
        gid  = 0;
        mode = is_root_only(child_virt) ? 0xF00 : 0xFFF;

        if (S_ISDIR(st.st_mode)) {
            uint32_t dir_cluster = exfat_create_dir(fs, parent_cluster,
                                                    entry_name, uid, gid, mode);
            printf("    Dir:  %s/ (cluster=%u)%s\n",
                   entry_name, dir_cluster,
                   (mode == 0xF00) ? " [root-only]" : "");
            exfat_populate_dir(fs, full_path, dir_cluster, child_virt);
        } else if (S_ISREG(st.st_mode)) {
            size_t   file_size;
            uint8_t *file_data = read_file(full_path, &file_size);
            exfat_add_file(fs, parent_cluster, entry_name,
                           file_data, file_size, uid, gid, mode);
            free(file_data);
        }

        free(names[i]);
    }

    free(names);
}

/*
 * Recursively populate the exFAT filesystem from a sysroot directory.
 * Mirrors ExFatFormatter.populate_from_sysroot().
 */
void exfat_populate_sysroot(ExFat *fs, const char *sysroot_path)
{
    struct stat st;

    if (stat(sysroot_path, &st) != 0 || !S_ISDIR(st.st_mode)) {
        printf("  Warning: sysroot path '%s' does not exist, skipping\n",
               sysroot_path);
        return;
    }

    exfat_populate_dir(fs, sysroot_path, fs->root_cluster, "");
}

/*
 * Write the in-memory FAT cache and allocation bitmap to disk.
 * Mirrors ExFatFormatter.flush_fat_and_bitmap().
 */
void exfat_flush(ExFat *fs)
{
    uint32_t s;
    uint32_t cluster;
    uint32_t offset;
    uint32_t num_clusters;

    /* Write FAT sectors */
    for (s = 0; s < fs->fat_length; ++s) {
        uint32_t byte_off = s * SECTOR_SIZE;
        uint8_t  sector_data[SECTOR_SIZE];
        uint32_t fat_bytes = (fs->cluster_count + 2) * 4;
        uint32_t chunk     = fat_bytes - byte_off;

        if (byte_off >= fat_bytes) {
            memset(sector_data, 0, SECTOR_SIZE);
        } else {
            if (chunk > SECTOR_SIZE) chunk = SECTOR_SIZE;
            memcpy(sector_data, fs->fat_cache + byte_off, chunk);
            if (chunk < SECTOR_SIZE)
                memset(sector_data + chunk, 0, SECTOR_SIZE - chunk);
        }
        exfat_write_sector(fs, fs->fat_offset + s, sector_data);
    }

    /* Write allocation bitmap to its cluster(s) */
    num_clusters = (fs->bitmap_bytes + fs->cluster_size - 1) / fs->cluster_size;
    offset       = 0;
    cluster      = fs->bitmap_cluster;

    for (s = 0; s < num_clusters; ++s) {
        uint32_t chunk = fs->bitmap_bytes - offset;
        if (chunk > fs->cluster_size)
            chunk = fs->cluster_size;
        exfat_write_cluster(fs, cluster, fs->bitmap + offset, chunk);
        offset += fs->cluster_size;
        cluster++;
    }

    /* Count actual used clusters from bitmap (accurate after incremental frees) */
    {
        uint32_t used = 0;
        for (uint32_t ci = 0; ci < fs->cluster_count; ci++) {
            if (fs->bitmap[ci / 8] & (1u << (ci % 8)))
                used++;
        }
        printf("  exFAT: FAT and bitmap flushed (%u clusters used of %u)\n",
               used, fs->cluster_count);
    }
}

/*
 * Free in-memory resources.
 */
void exfat_free(ExFat *fs)
{
    free(fs->fat_cache);
    fs->fat_cache = NULL;
    free(fs->bitmap);
    fs->bitmap = NULL;
}

/* ═══════════════════════════════════════════════════════════════════════════
 * Incremental update support — exFAT reader + sync
 * ═══════════════════════════════════════════════════════════════════════════ */

/*
 * Open an existing exFAT filesystem by parsing its VBR.
 * Loads FAT cache and allocation bitmap from the image data.
 * After this call, the ExFat struct is ready for sync operations.
 */
void exfat_open_existing(ExFat *fs, uint8_t *image, uint32_t fs_start)
{
    uint8_t *vbr = image + (size_t)fs_start * SECTOR_SIZE;

    /* Verify exFAT signature */
    if (memcmp(vbr + 3, "EXFAT   ", 8) != 0)
        fatal("exfat_open_existing: not an exFAT filesystem at sector %u", fs_start);

    fs->image    = image;
    fs->fs_start = fs_start;

    /* Parse VBR fields */
    fs->fs_sectors         = (uint32_t)read_le64(vbr + 72);
    fs->fat_offset         = read_le32(vbr + 80);
    fs->fat_length         = read_le32(vbr + 84);
    fs->cluster_heap_offset = read_le32(vbr + 88);
    fs->cluster_count      = read_le32(vbr + 92);
    fs->root_cluster       = read_le32(vbr + 96);
    fs->spc                = 1u << vbr[109];
    fs->cluster_size       = fs->spc * SECTOR_SIZE;

    /* Load FAT cache from image */
    uint32_t fat_bytes = (fs->cluster_count + 2) * 4;
    fs->fat_cache = (uint8_t *)malloc(fat_bytes);
    if (!fs->fat_cache) fatal("exfat_open_existing: malloc fat_cache failed");
    memcpy(fs->fat_cache,
           image + (size_t)(fs_start + fs->fat_offset) * SECTOR_SIZE,
           fat_bytes);

    /* Load allocation bitmap from cluster 2 */
    fs->bitmap_cluster = 2;
    fs->bitmap_bytes   = (fs->cluster_count + 7) / 8;
    fs->bitmap         = (uint8_t *)malloc(fs->bitmap_bytes);
    if (!fs->bitmap) fatal("exfat_open_existing: malloc bitmap failed");

    uint32_t bm_sector = exfat_cluster_to_sector(fs, 2);
    uint32_t bm_clusters = (fs->bitmap_bytes + fs->cluster_size - 1) / fs->cluster_size;
    uint32_t bm_offset = 0;
    for (uint32_t i = 0; i < bm_clusters; i++) {
        uint32_t chunk = fs->bitmap_bytes - bm_offset;
        if (chunk > fs->cluster_size) chunk = fs->cluster_size;
        memcpy(fs->bitmap + bm_offset,
               image + (size_t)(fs_start + bm_sector + i * fs->spc) * SECTOR_SIZE,
               chunk);
        bm_offset += fs->cluster_size;
    }

    /* Find next_cluster: scan bitmap for first free cluster */
    fs->next_cluster = fs->cluster_count + 2; /* default: full */
    for (uint32_t c = 2; c < fs->cluster_count + 2; c++) {
        uint32_t idx = c - 2;
        if (!(fs->bitmap[idx / 8] & (1u << (idx % 8)))) {
            fs->next_cluster = c;
            break;
        }
    }

    printf("  exFAT: opened existing filesystem (%u clusters, %u bytes/cluster)\n",
           fs->cluster_count, fs->cluster_size);
    printf("  exFAT: next free cluster: %u (%u used)\n",
           fs->next_cluster, fs->next_cluster - 2);
}

/*
 * Read data from a cluster chain (contiguous or FAT-chained).
 * Returns malloc'd buffer of `length` bytes. Caller frees.
 */
static uint8_t *exfat_read_cluster_data(ExFat *fs, uint32_t first_cluster,
                                         uint64_t length, int contiguous)
{
    uint8_t *data = (uint8_t *)malloc((size_t)length);
    if (!data) fatal("exfat_read_cluster_data: malloc failed");

    uint32_t cluster = first_cluster;
    uint64_t offset = 0;

    while (offset < length) {
        uint32_t sector = exfat_cluster_to_sector(fs, cluster);
        uint32_t abs_off = (fs->fs_start + sector) * SECTOR_SIZE;
        uint64_t chunk = length - offset;
        if (chunk > fs->cluster_size) chunk = fs->cluster_size;
        memcpy(data + offset, fs->image + abs_off, (size_t)chunk);
        offset += fs->cluster_size;

        if (contiguous) {
            cluster++;
        } else {
            uint32_t next = read_le32(fs->fat_cache + cluster * 4);
            if (next >= 0xFFFFFFF8u) break;
            cluster = next;
        }
    }
    return data;
}

/*
 * Parse directory entries from a directory cluster chain and build
 * an ExFatNode tree. Returns a root node whose children are the entries.
 */
ExFatNode *exfat_read_dir_tree(ExFat *fs, uint32_t dir_cluster)
{
    ExFatNode *parent = (ExFatNode *)calloc(1, sizeof(ExFatNode));
    if (!parent) fatal("exfat_read_dir_tree: calloc failed");
    parent->attrs = EXFAT_ATTR_DIR;
    parent->first_cluster = dir_cluster;

    uint32_t cluster = dir_cluster;

    while (1) {
        uint32_t sector = exfat_cluster_to_sector(fs, cluster);
        uint8_t *dir_data = (uint8_t *)malloc(fs->cluster_size);
        if (!dir_data) fatal("exfat_read_dir_tree: malloc failed");

        /* Read cluster */
        for (uint32_t s = 0; s < fs->spc; s++)
            exfat_read_sector(fs, sector + s, dir_data + s * SECTOR_SIZE);

        /* Walk entries */
        for (uint32_t idx = 0; idx < fs->cluster_size / 32; idx++) {
            uint32_t off = idx * 32;
            uint8_t etype = dir_data[off];

            if (etype == 0x00) {
                /* End of entries in this cluster.  Don't stop here —
                 * a multi-cluster directory may have entries in subsequent
                 * clusters (preceding cluster was full, remaining space was
                 * zero-filled).  We follow the FAT chain after this loop. */
                break;
            }

            if (etype == EXFAT_ENTRY_FILE) {
                /* File directory entry — start of entry set */
                uint8_t secondary_count = dir_data[off + 1];
                uint32_t entry_set_len = (1 + secondary_count) * 32;

                /* Ensure we have enough data in this cluster */
                if (off + entry_set_len > fs->cluster_size) {
                    /* Entry set spans cluster boundary — skip this entry,
                     * continue to next cluster via FAT chain */
                    break;
                }

                /* Parse attributes */
                uint16_t attrs = read_le16(dir_data + off + 4);
                uint16_t uid   = read_le16(dir_data + off + 6);
                uint16_t gid   = read_le16(dir_data + off + 8);
                uint16_t mode  = read_le16(dir_data + off + 10);

                /* Parse stream extension (second entry) */
                uint32_t stream_off = off + 32;
                uint8_t flags       = dir_data[stream_off + 1];
                uint8_t name_len    = dir_data[stream_off + 3];
                uint32_t first_cl   = read_le32(dir_data + stream_off + 20);
                uint64_t data_len   = read_le64(dir_data + stream_off + 24);

                /* Parse filename entries */
                char name[256];
                uint32_t name_pos = 0;
                for (uint8_t fi = 0; fi < secondary_count - 1 && fi < 17; fi++) {
                    uint32_t fn_off = off + (2 + fi) * 32;
                    if (dir_data[fn_off] != EXFAT_ENTRY_FILENAME) break;
                    for (int j = 0; j < 15 && name_pos < name_len; j++) {
                        uint16_t ch = read_le16(dir_data + fn_off + 2 + j * 2);
                        if (ch == 0) break;
                        name[name_pos++] = (char)(ch & 0xFF); /* ASCII only */
                    }
                }
                name[name_pos] = '\0';

                /* Create node */
                ExFatNode *node = (ExFatNode *)calloc(1, sizeof(ExFatNode));
                if (!node) fatal("exfat_read_dir_tree: calloc node failed");
                strncpy(node->name, name, sizeof(node->name) - 1);
                node->attrs         = attrs;
                node->first_cluster = first_cl;
                node->data_length   = data_len;
                node->uid           = uid;
                node->gid           = gid;
                node->mode          = mode;
                node->contiguous    = (flags & EXFAT_FLAG_CONTIGUOUS) ? 1 : 0;
                node->dir_cluster   = cluster;  /* actual cluster containing this entry */
                node->entry_offset  = off;
                node->entry_set_len = entry_set_len;

                /* Add to parent's children */
                if (parent->child_count >= parent->child_cap) {
                    int new_cap = (parent->child_cap == 0) ? 32 : parent->child_cap * 2;
                    parent->children = (ExFatNode *)realloc(parent->children,
                        (size_t)new_cap * sizeof(ExFatNode));
                    if (!parent->children) fatal("realloc children failed");
                    parent->child_cap = new_cap;
                }
                parent->children[parent->child_count++] = *node;
                free(node);

                /* If directory, recurse */
                if (attrs & EXFAT_ATTR_DIR) {
                    ExFatNode *child = &parent->children[parent->child_count - 1];
                    if (first_cl >= 2 && first_cl < fs->cluster_count + 2) {
                        ExFatNode *subtree = exfat_read_dir_tree(fs, first_cl);
                        child->children    = subtree->children;
                        child->child_count = subtree->child_count;
                        child->child_cap   = subtree->child_cap;
                        /* Free the wrapper node but NOT children */
                        subtree->children = NULL;
                        free(subtree);
                    }
                }

                /* Skip past the secondary entries */
                idx += secondary_count;
            }
            /* Skip bitmap (0x81), upcase (0x82), label (0x83), and deleted entries */
        }

        free(dir_data);

        /* Follow FAT chain */
        uint32_t next = read_le32(fs->fat_cache + cluster * 4);
        if (next >= 0xFFFFFFF8u || next == 0) {
            /* No more clusters — truly end of directory */
            break;
        }
        cluster = next;
        /* Continue to next cluster — a multi-cluster directory may have
         * trailing zeros in one cluster and valid entries in the next */
    }

    return parent;
}

/*
 * Find a child node by name in a directory node.
 * Returns pointer into parent->children array, or NULL.
 */
ExFatNode *exfat_find_child(ExFatNode *parent, const char *name)
{
    if (!parent) return NULL;
    for (int i = 0; i < parent->child_count; i++) {
        if (strcmp(parent->children[i].name, name) == 0)
            return &parent->children[i];
    }
    return NULL;
}

/*
 * Compare file content in the existing image with new data.
 * Returns 1 if content matches (no update needed), 0 otherwise.
 */
int exfat_file_matches(ExFat *fs, ExFatNode *node,
                       const uint8_t *new_data, size_t new_size)
{
    if (node->data_length != (uint64_t)new_size)
        return 0;
    if (new_size == 0)
        return 1;

    uint32_t cluster = node->first_cluster;
    size_t offset = 0;

    while (offset < new_size) {
        uint32_t sector = exfat_cluster_to_sector(fs, cluster);
        uint32_t abs_off = (fs->fs_start + sector) * SECTOR_SIZE;
        size_t chunk = new_size - offset;
        if (chunk > fs->cluster_size) chunk = fs->cluster_size;

        if (memcmp(fs->image + abs_off, new_data + offset, chunk) != 0)
            return 0;

        offset += fs->cluster_size;
        if (node->contiguous) {
            cluster++;
        } else {
            uint32_t next = read_le32(fs->fat_cache + cluster * 4);
            if (next >= 0xFFFFFFF8u) break;
            cluster = next;
        }
    }
    return 1;
}

/*
 * Free clusters used by a node. Clears bitmap bits and FAT entries.
 */
void exfat_free_clusters(ExFat *fs, ExFatNode *node)
{
    if (node->first_cluster < 2 || node->data_length == 0)
        return;

    uint32_t num_clusters = (uint32_t)((node->data_length + fs->cluster_size - 1)
                                        / fs->cluster_size);
    uint32_t cluster = node->first_cluster;

    for (uint32_t i = 0; i < num_clusters; i++) {
        uint32_t idx = cluster - 2;
        if (idx < fs->cluster_count) {
            /* Clear bitmap bit */
            fs->bitmap[idx / 8] &= (uint8_t)~(1u << (idx % 8));
        }

        uint32_t next;
        if (node->contiguous) {
            next = cluster + 1;
        } else {
            next = read_le32(fs->fat_cache + cluster * 4);
        }

        /* Clear FAT entry */
        write_le32(fs->fat_cache + cluster * 4, EXFAT_FREE);

        if (!node->contiguous && next >= 0xFFFFFFF8u)
            break;
        cluster = next;
    }

    /* Update next_cluster if we freed earlier clusters */
    if (node->first_cluster < fs->next_cluster)
        fs->next_cluster = node->first_cluster;
}

/*
 * Mark a directory entry set as deleted.
 * Sets the type byte's bit 7 to 0 for each entry in the set.
 */
void exfat_delete_entry(ExFat *fs, ExFatNode *node)
{
    uint32_t cluster = node->dir_cluster;
    uint32_t offset  = node->entry_offset;
    uint32_t sector  = exfat_cluster_to_sector(fs, cluster);
    uint32_t abs_off = (fs->fs_start + sector) * SECTOR_SIZE + offset;

    /* Mark each entry in the set as deleted (clear bit 7 of type byte) */
    uint32_t num_entries = node->entry_set_len / 32;
    for (uint32_t i = 0; i < num_entries; i++) {
        fs->image[abs_off + i * 32] &= 0x7F;
    }
}

/*
 * Internal: sync a single directory, comparing sysroot entries with
 * existing filesystem entries.
 */
static void exfat_sync_dir(ExFat *fs, const char *host_path,
                            uint32_t parent_cluster, ExFatNode *existing,
                            const char *virt_path,
                            int *n_unchanged, int *n_updated, int *n_added)
{
    DIR *d = opendir(host_path);
    if (!d) {
        fprintf(stderr, "  WARNING: Cannot open directory %s\n", host_path);
        return;
    }

    /* Collect and sort names (same as exfat_populate_dir) */
    char **names = NULL;
    int name_count = 0, name_cap = 0;
    struct dirent *ent;

    while ((ent = readdir(d)) != NULL) {
        if (ent->d_name[0] == '.') continue;
        if (name_count >= name_cap) {
            int new_cap = (name_cap == 0) ? 64 : name_cap * 2;
            names = (char **)realloc(names, (size_t)new_cap * sizeof(char *));
            if (!names) fatal("exfat_sync_dir: realloc failed");
            name_cap = new_cap;
        }
        names[name_count++] = strdup(ent->d_name);
    }
    closedir(d);

    /* Sort ASCII */
    for (int i = 0; i < name_count - 1; i++) {
        for (int j = i + 1; j < name_count; j++) {
            if (strcmp(names[i], names[j]) > 0) {
                char *tmp = names[i]; names[i] = names[j]; names[j] = tmp;
            }
        }
    }

    for (int i = 0; i < name_count; i++) {
        char full_path[4096], child_virt[4096];
        snprintf(full_path, sizeof(full_path), "%s/%s", host_path, names[i]);

        if (virt_path[0] == '\0')
            snprintf(child_virt, sizeof(child_virt), "%s", names[i]);
        else
            snprintf(child_virt, sizeof(child_virt), "%s/%s", virt_path, names[i]);

        struct stat st;
        if (stat(full_path, &st) != 0) { free(names[i]); continue; }

        uint16_t uid = 0, gid = 0;
        uint16_t mode = is_root_only(child_virt) ? 0xF00 : 0xFFF;

        ExFatNode *child = exfat_find_child(existing, names[i]);

        if (S_ISDIR(st.st_mode)) {
            if (child && (child->attrs & EXFAT_ATTR_DIR)) {
                /* Directory exists — recurse */
                exfat_sync_dir(fs, full_path, child->first_cluster, child,
                               child_virt, n_unchanged, n_updated, n_added);
            } else {
                /* New directory */
                uint32_t dir_cl = exfat_create_dir(fs, parent_cluster,
                                                    names[i], uid, gid, mode);
                printf("    Dir+: %s/ (cluster=%u)\n", names[i], dir_cl);
                /* Populate new directory fully */
                exfat_populate_dir(fs, full_path, dir_cl, child_virt);
                (*n_added)++;
            }
        } else if (S_ISREG(st.st_mode)) {
            size_t file_size;
            uint8_t *file_data = read_file(full_path, &file_size);

            if (child && !(child->attrs & EXFAT_ATTR_DIR)) {
                /* File exists — check if content changed */
                if (exfat_file_matches(fs, child, file_data, file_size)) {
                    (*n_unchanged)++;
                } else {
                    /* Changed — delete old, add new */
                    exfat_free_clusters(fs, child);
                    exfat_delete_entry(fs, child);
                    exfat_add_file(fs, parent_cluster, names[i],
                                   file_data, file_size, uid, gid, mode);
                    (*n_updated)++;
                }
            } else {
                /* New file */
                exfat_add_file(fs, parent_cluster, names[i],
                               file_data, file_size, uid, gid, mode);
                (*n_added)++;
            }
            free(file_data);
        }
        free(names[i]);
    }
    free(names);
}

/*
 * Sync sysroot with existing exFAT filesystem.
 * Only updates changed files, preserves non-sysroot data.
 */
void exfat_sync_sysroot(ExFat *fs, const char *sysroot_path)
{
    struct stat st;
    if (stat(sysroot_path, &st) != 0 || !S_ISDIR(st.st_mode)) {
        printf("  Warning: sysroot path '%s' does not exist, skipping\n", sysroot_path);
        return;
    }

    printf("  Incremental sync from: %s\n", sysroot_path);

    /* Read existing directory tree */
    ExFatNode *root = exfat_read_dir_tree(fs, fs->root_cluster);

    int n_unchanged = 0, n_updated = 0, n_added = 0;

    exfat_sync_dir(fs, sysroot_path, fs->root_cluster, root, "",
                   &n_unchanged, &n_updated, &n_added);

    printf("  exFAT sync: %d unchanged, %d updated, %d added\n",
           n_unchanged, n_updated, n_added);

    exfat_free_tree(root);
}

/*
 * Recursively free children arrays of a node.
 * Does NOT free the node itself (it is embedded in its parent's children array).
 */
static void exfat_free_children(ExFatNode *node)
{
    int i;
    if (!node || !node->children) return;
    for (i = 0; i < node->child_count; i++)
        exfat_free_children(&node->children[i]);
    free(node->children);
    node->children = NULL;
    node->child_count = 0;
}

/*
 * Free an ExFatNode tree.  Only the root node was individually calloc'd;
 * all other nodes live inside their parent's realloc'd children array.
 */
void exfat_free_tree(ExFatNode *node)
{
    if (!node) return;
    exfat_free_children(node);
    free(node);
}
