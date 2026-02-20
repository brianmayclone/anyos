/*
 * fat16.c — FAT16 filesystem formatter with VFAT Long Filename support
 *
 * Faithful C port of the Fat16Formatter class and LFN helpers from mkimage.py.
 * Written in C99 for TCC compatibility.
 */

#include "mkimage.h"

#include <ctype.h>
#include <dirent.h>
#include <sys/stat.h>

/* ── Short-name collision counter table ───────────────────────────────────── */

/*
 * We track how many times each (base6, ext3) pair has been used so we can
 * append a unique ~N numeric tail.  The table is indexed by a simple hash
 * of the canonicalised (base, ext) pair and stores the current counter value.
 * 4096 slots gives negligible collision probability for any real sysroot.
 */
static uint16_t short_name_counters[SHORT_NAME_SLOTS];

/* Reset the collision table at the start of each Fat16 init (matches Python's
 * `global _short_name_counters; _short_name_counters = {}`). */
static void reset_short_name_counters(void)
{
    memset(short_name_counters, 0, sizeof(short_name_counters));
}

/* Simple djb2-style hash over a null-terminated string. */
static unsigned short_name_hash(const char *s)
{
    unsigned h = 5381;
    while (*s) {
        h = ((h << 5) + h) ^ (unsigned char)*s;
        ++s;
    }
    return (unsigned short)(h & (SHORT_NAME_SLOTS - 1));
}

/* ── LFN helper: needs_lfn ────────────────────────────────────────────────── */

/*
 * Returns 1 if filename requires LFN entries (does not fit 8.3).
 * Mirrors the Python needs_lfn() function exactly.
 */
int needs_lfn(const char *filename)
{
    size_t len;
    const char *dot;
    const char *base;
    const char *ext;
    size_t base_len, ext_len;
    const char *p;

    if (!filename)
        return 1;

    len = strlen(filename);

    /* Empty or longer than 12 chars always needs LFN */
    if (len == 0 || len > 12)
        return 1;

    /* Dot-files (other than "." and "..") need LFN */
    if (filename[0] == '.') {
        if (!(filename[1] == '\0' ||
              (filename[1] == '.' && filename[2] == '\0')))
            return 1;
    }

    /* More than one dot */
    {
        int dot_count = 0;
        for (p = filename; *p; ++p)
            if (*p == '.')
                ++dot_count;
        if (dot_count > 1)
            return 1;
    }

    /* Split at last dot */
    dot = strrchr(filename, '.');
    if (dot) {
        base     = filename;
        base_len = (size_t)(dot - filename);
        ext      = dot + 1;
        ext_len  = strlen(ext);
    } else {
        base     = filename;
        base_len = len;
        ext      = "";
        ext_len  = 0;
    }

    if (base_len > 8 || ext_len > 3)
        return 1;

    /* Special chars that require LFN */
    for (p = filename; *p; ++p) {
        if (strchr(" +,;=[]", *p))
            return 1;
    }

    /* Lowercase chars require LFN (Windows behaviour) */
    for (p = filename; *p; ++p) {
        if (islower((unsigned char)*p))
            return 1;
    }

    (void)base; (void)ext; /* suppress unused-variable warnings */
    return 0;
}

/* ── LFN helper: generate_short_name ─────────────────────────────────────── */

/*
 * Generate a unique 8.3 short name from a long filename.
 * out11 receives exactly 11 bytes (8 base + 3 ext), space-padded, uppercase,
 * NO null terminator — matches FAT directory entry format.
 *
 * Mirrors the Python generate_short_name() function exactly.
 */
void generate_short_name(const char *filename, char *out11)
{
    char name_up[256];
    char base[256];
    char ext[256];
    char base_filtered[256];
    char ext_filtered[256];
    char key[16];           /* "BBBBBB.EEE" for hash */
    unsigned slot;
    uint16_t counter;
    char tail[12];
    int tail_len;
    int max_base;
    char short_base[16];
    int i;
    const char *p;

    /* Uppercase the filename */
    {
        size_t fi = 0;
        for (fi = 0; filename[fi] && fi < sizeof(name_up) - 1; ++fi)
            name_up[fi] = (char)toupper((unsigned char)filename[fi]);
        name_up[fi] = '\0';
    }

    /* Split at last dot */
    {
        const char *dot = strrchr(name_up, '.');
        if (dot) {
            size_t blen = (size_t)(dot - name_up);
            if (blen >= sizeof(base)) blen = sizeof(base) - 1;
            memcpy(base, name_up, blen);
            base[blen] = '\0';
            strncpy(ext, dot + 1, sizeof(ext) - 1);
            ext[sizeof(ext) - 1] = '\0';
        } else {
            strncpy(base, name_up, sizeof(base) - 1);
            base[sizeof(base) - 1] = '\0';
            ext[0] = '\0';
        }
    }

    /* Filter invalid chars from base: remove ' ', '.', '+', ',', ';', '=', '[', ']' */
    {
        size_t out_i = 0;
        for (p = base; *p && out_i < sizeof(base_filtered) - 1; ++p) {
            if (!strchr(" .+,;=[]", *p))
                base_filtered[out_i++] = *p;
        }
        base_filtered[out_i] = '\0';
    }

    /* Filter invalid chars from ext: remove ' ', '.' */
    {
        size_t out_i = 0;
        for (p = ext; *p && out_i < sizeof(ext_filtered) - 1; ++p) {
            if (!strchr(" .", *p))
                ext_filtered[out_i++] = *p;
        }
        ext_filtered[out_i] = '\0';
    }

    /* Truncate base to 6, ext to 3 */
    base_filtered[6] = '\0';
    ext_filtered[3]  = '\0';

    /* Build hash key from (base6, ext3) — matches Python key = (base, ext) */
    snprintf(key, sizeof(key), "%.6s.%.3s", base_filtered, ext_filtered);
    slot    = short_name_hash(key);
    counter = ++short_name_counters[slot];

    /* Build ~N tail */
    snprintf(tail, sizeof(tail), "~%u", (unsigned)counter);
    tail_len = (int)strlen(tail);
    max_base = 8 - tail_len;
    if (max_base < 0) max_base = 0;

    /* short_base = base[:max_base] + tail */
    snprintf(short_base, sizeof(short_base), "%.*s%s",
             max_base, base_filtered, tail);
    /* short_base is at most 8 chars; ensure it */
    short_base[8] = '\0';

    /* Fill out11: 8 bytes base (space-padded) + 3 bytes ext (space-padded) */
    for (i = 0; i < 8; ++i)
        out11[i] = short_base[i] ? short_base[i] : ' ';
    for (i = 0; i < 3; ++i)
        out11[8 + i] = ext_filtered[i] ? ext_filtered[i] : ' ';
}

/* ── LFN helper: lfn_checksum ─────────────────────────────────────────────── */

/*
 * Compute the VFAT LFN checksum from an 11-byte 8.3 name.
 * s = (((s & 1) << 7) + (s >> 1) + byte) & 0xFF  for each byte.
 */
uint8_t lfn_checksum(const uint8_t *name83)
{
    uint8_t s = 0;
    int i;
    for (i = 0; i < 11; ++i)
        s = (uint8_t)((((s & 1u) << 7) + (s >> 1) + name83[i]) & 0xFFu);
    return s;
}

/* ── LFN helper: make_lfn_entries ─────────────────────────────────────────── */

/*
 * Create LFN directory entries for filename, given the 11-byte 8.3 short name.
 * Entries are written to `entries` in disk order (last logical entry first).
 * Returns the number of 32-byte entries written (0..max_entries).
 *
 * Each entry is 32 bytes.  The caller must ensure entries[] has room for
 * ceil(strlen(filename)/13) * 32 bytes.
 *
 * Mirrors the Python make_lfn_entries() function exactly.
 */
int make_lfn_entries(const char *filename, const char *name83,
                     uint8_t *entries, int max_entries)
{
    uint8_t chk;
    int utf16_len;
    int num_entries;
    int seq;
    int written;
    /* UTF-16LE code units — ASCII filenames only use the low byte */
    uint16_t utf16[256];
    int j;

    chk = lfn_checksum((const uint8_t *)name83);

    /* Build UTF-16LE array from filename (ASCII subset only) */
    utf16_len = (int)strlen(filename);
    if (utf16_len > 255) utf16_len = 255;
    for (j = 0; j < utf16_len; ++j)
        utf16[j] = (uint16_t)(unsigned char)filename[j];

    num_entries = (utf16_len + 12) / 13;
    if (num_entries > max_entries)
        num_entries = max_entries;

    /*
     * Build entries in logical order (seq 1, 2, … num_entries) into a
     * temporary stack buffer, then reverse into the output buffer so that the
     * last logical entry (marked 0x40) appears first on disk.
     */
    written = 0;
    {
        /* Temporary storage: up to 20 LFN entries per filename */
        uint8_t tmp[20 * 32];
        int e;

        if (num_entries > 20) num_entries = 20;

        for (seq = 1; seq <= num_entries; ++seq) {
            uint8_t *entry = tmp + (seq - 1) * 32;
            int is_last = (seq == num_entries);
            int start   = (seq - 1) * 13;
            uint16_t chars[13];

            memset(entry, 0, 32);

            entry[0]  = (uint8_t)(seq | (is_last ? 0x40 : 0));
            entry[11] = 0x0F;   /* ATTR_LONG_NAME */
            entry[12] = 0;      /* type */
            entry[13] = chk;
            entry[26] = 0;      /* first cluster lo */
            entry[27] = 0;

            /* Build 13 UTF-16LE chars for this entry */
            for (j = 0; j < 13; ++j) {
                int idx = start + j;
                if (idx < utf16_len)
                    chars[j] = utf16[idx];
                else if (idx == utf16_len)
                    chars[j] = 0x0000;     /* NUL terminator */
                else
                    chars[j] = 0xFFFF;     /* padding */
            }

            /* Chars 1-5  → offsets 1-10  (5 × 2 bytes) */
            for (j = 0; j < 5; ++j) {
                entry[1  + j * 2] = (uint8_t)(chars[j] & 0xFF);
                entry[2  + j * 2] = (uint8_t)(chars[j] >> 8);
            }
            /* Chars 6-11 → offsets 14-25 (6 × 2 bytes) */
            for (j = 0; j < 6; ++j) {
                entry[14 + j * 2] = (uint8_t)(chars[5 + j] & 0xFF);
                entry[15 + j * 2] = (uint8_t)(chars[5 + j] >> 8);
            }
            /* Chars 12-13 → offsets 28-31 (2 × 2 bytes) */
            for (j = 0; j < 2; ++j) {
                entry[28 + j * 2] = (uint8_t)(chars[11 + j] & 0xFF);
                entry[29 + j * 2] = (uint8_t)(chars[11 + j] >> 8);
            }
        }

        /* Reverse into output: entry[num_entries-1] first, entry[0] last */
        for (e = 0; e < num_entries; ++e) {
            memcpy(entries + e * 32,
                   tmp + (num_entries - 1 - e) * 32,
                   32);
        }
        written = num_entries;
    }

    return written;
}

/* ═══════════════════════════════════════════════════════════════════════════
 * FAT16 internal helpers
 * ═══════════════════════════════════════════════════════════════════════════ */

/* Convert filesystem-relative sector to absolute image sector. */
static uint32_t fat16_abs_sector(const Fat16 *fs, uint32_t rel)
{
    return fs->fs_start + rel;
}

/* Write 512 bytes to a filesystem-relative sector. */
static void fat16_write_sector(Fat16 *fs, uint32_t rel, const uint8_t *data)
{
    uint32_t offset = fat16_abs_sector(fs, rel) * SECTOR_SIZE;
    memcpy(fs->image + offset, data, SECTOR_SIZE);
}

/* Read 512 bytes from a filesystem-relative sector into out[]. */
static void fat16_read_sector(const Fat16 *fs, uint32_t rel, uint8_t *out)
{
    uint32_t offset = fat16_abs_sector(fs, rel) * SECTOR_SIZE;
    memcpy(out, fs->image + offset, SECTOR_SIZE);
}

/* Set a FAT16 entry for cluster in both FAT copies. */
static void fat16_set_fat_entry(Fat16 *fs, uint32_t cluster, uint16_t value)
{
    uint32_t fat_offset       = cluster * 2;
    uint32_t sector_in_fat    = fat_offset / SECTOR_SIZE;
    uint32_t offset_in_sector = fat_offset % SECTOR_SIZE;
    uint32_t fat_idx;
    uint8_t  sector_data[SECTOR_SIZE];

    for (fat_idx = 0; fat_idx < fs->num_fats; ++fat_idx) {
        uint32_t abs_sector =
            fs->first_fat_sector + fat_idx * fs->fat_size + sector_in_fat;
        fat16_read_sector(fs, abs_sector, sector_data);
        write_le16(sector_data + offset_in_sector, value);
        fat16_write_sector(fs, abs_sector, sector_data);
    }
}

/* Convert cluster number to filesystem-relative sector. */
static uint32_t fat16_cluster_to_sector(const Fat16 *fs, uint32_t cluster)
{
    return fs->first_data_sector + (cluster - 2) * fs->sectors_per_cluster;
}

/* Allocate a chain of clusters. Returns the first cluster number. */
static uint32_t fat16_alloc_clusters(Fat16 *fs, uint32_t count)
{
    uint32_t first;
    uint32_t i;

    if (count == 0)
        return 0;

    first = fs->next_cluster;
    for (i = 0; i < count; ++i) {
        uint32_t current = fs->next_cluster++;
        if (i < count - 1)
            fat16_set_fat_entry(fs, current, (uint16_t)(current + 1));
        else
            fat16_set_fat_entry(fs, current, FAT16_END_OF_CHAIN);
    }
    return first;
}

/* Write data to a cluster chain starting at first_cluster. */
static void fat16_write_to_clusters(Fat16 *fs, uint32_t first_cluster,
                                    const uint8_t *data, size_t len)
{
    uint32_t cluster     = first_cluster;
    size_t   offset      = 0;
    uint32_t cluster_size = fs->sectors_per_cluster * SECTOR_SIZE;
    uint8_t  sector_data[SECTOR_SIZE];

    while (offset < len) {
        /* How many bytes of this cluster to write */
        size_t   chunk_size = len - offset;
        uint32_t sector;
        uint32_t s;

        if (chunk_size > cluster_size)
            chunk_size = cluster_size;

        sector = fat16_cluster_to_sector(fs, cluster);

        for (s = 0; s < fs->sectors_per_cluster; ++s) {
            size_t s_offset = s * SECTOR_SIZE;
            if (s_offset >= chunk_size)
                break;

            {
                size_t s_len = chunk_size - s_offset;
                if (s_len > SECTOR_SIZE) s_len = SECTOR_SIZE;

                memcpy(sector_data, data + offset + s_offset, s_len);
                if (s_len < SECTOR_SIZE)
                    memset(sector_data + s_len, 0, SECTOR_SIZE - s_len);
                fat16_write_sector(fs, sector + s, sector_data);
            }
        }

        offset += cluster_size;

        /* Follow FAT chain to next cluster */
        if (offset < len) {
            uint32_t fat_offset_b    = cluster * 2;
            uint32_t sector_in_fat   = fat_offset_b / SECTOR_SIZE;
            uint32_t offset_in_sect  = fat_offset_b % SECTOR_SIZE;
            uint8_t  fat_sec[SECTOR_SIZE];

            fat16_read_sector(fs, fs->first_fat_sector + sector_in_fat, fat_sec);
            cluster = read_le16(fat_sec + offset_in_sect);
            if (cluster >= 0xFFF8)
                break;
        }
    }
}

/*
 * Simple 8.3 name conversion (no collision tracking).
 * Used for "." and ".." entries and for non-LFN filenames.
 */
static void fat16_make_83_name(const char *filename, char *out11)
{
    char name_up[256];
    char base[9];
    char ext[4];
    size_t fi;
    const char *dot;
    int i;

    /* Uppercase */
    for (fi = 0; filename[fi] && fi < sizeof(name_up) - 1; ++fi)
        name_up[fi] = (char)toupper((unsigned char)filename[fi]);
    name_up[fi] = '\0';

    dot = strrchr(name_up, '.');
    if (dot) {
        size_t blen = (size_t)(dot - name_up);
        if (blen > 8) blen = 8;
        memcpy(base, name_up, blen);
        base[blen] = '\0';
        strncpy(ext, dot + 1, 3);
        ext[3] = '\0';
    } else {
        strncpy(base, name_up, 8);
        base[8] = '\0';
        ext[0]  = '\0';
    }

    for (i = 0; i < 8; ++i)
        out11[i] = base[i] ? base[i] : ' ';
    for (i = 0; i < 3; ++i)
        out11[8 + i] = ext[i] ? ext[i] : ' ';
}

/* Write a 32-byte entry at a specific root directory index. */
static void fat16_write_root_entry_at(Fat16 *fs, uint32_t index,
                                      const uint8_t *entry32)
{
    uint32_t entry_offset     = index * 32;
    uint32_t sector_in_root   = entry_offset / SECTOR_SIZE;
    uint32_t offset_in_sector = entry_offset % SECTOR_SIZE;
    uint32_t sector           = fs->first_root_dir_sector + sector_in_root;
    uint8_t  sector_data[SECTOR_SIZE];

    fat16_read_sector(fs, sector, sector_data);
    memcpy(sector_data + offset_in_sector, entry32, 32);
    fat16_write_sector(fs, sector, sector_data);
}

/*
 * Add a directory entry to the root directory, with LFN if needed.
 * Mirrors Python add_root_dir_entry().
 */
static void fat16_add_root_dir_entry(Fat16 *fs, const char *filename,
                                     uint32_t first_cluster, uint32_t file_size,
                                     int is_dir)
{
    int     use_lfn = needs_lfn(filename);
    char    name83[11];
    uint8_t entry[32];

    if (use_lfn) {
        generate_short_name(filename, name83);
    } else {
        fat16_make_83_name(filename, name83);
    }

    /* Write LFN entries first */
    if (use_lfn) {
        uint8_t lfn_buf[20 * 32];
        int     n;
        int     i;

        n = make_lfn_entries(filename, name83, lfn_buf, 20);
        for (i = 0; i < n; ++i) {
            fat16_write_root_entry_at(fs, fs->next_root_entry,
                                      lfn_buf + i * 32);
            fs->next_root_entry++;
        }
    }

    /* Write the 8.3 entry */
    memset(entry, 0, 32);
    memcpy(entry, name83, 11);

    entry[11] = (uint8_t)(is_dir ? 0x10 : 0x20);   /* DIRECTORY or ARCHIVE */
    write_le16(entry + 26, (uint16_t)(first_cluster & 0xFFFF));  /* cluster lo */
    write_le16(entry + 20, 0);                                    /* cluster hi = 0 */
    write_le32(entry + 28, is_dir ? 0 : file_size);

    fat16_write_root_entry_at(fs, fs->next_root_entry, entry);
    fs->next_root_entry++;
}

/*
 * Add a directory entry to a subdirectory cluster, with LFN if needed.
 * Mirrors Python add_subdir_entry().
 */
static void fat16_add_subdir_entry(Fat16 *fs, uint32_t parent_cluster,
                                   const char *filename,
                                   uint32_t first_cluster, uint32_t file_size,
                                   int is_dir)
{
    int     use_lfn = needs_lfn(filename);
    char    name83[11];
    uint8_t lfn_buf[20 * 32];
    int     lfn_count  = 0;
    int     total_needed;
    uint32_t cluster_size;
    uint8_t *dir_data;
    uint32_t sector;
    uint32_t s;
    int      found_start;
    int      consecutive;
    int      i;
    uint32_t pos;
    uint8_t  entry[32];

    if (use_lfn) {
        generate_short_name(filename, name83);
        lfn_count    = make_lfn_entries(filename, name83, lfn_buf, 20);
        total_needed = lfn_count + 1;
    } else {
        fat16_make_83_name(filename, name83);
        total_needed = 1;
    }

    cluster_size = fs->sectors_per_cluster * SECTOR_SIZE;
    dir_data     = (uint8_t *)malloc(cluster_size);
    if (!dir_data)
        fatal("fat16_add_subdir_entry: malloc(%u) failed", cluster_size);

    /* Read existing directory data */
    sector = fat16_cluster_to_sector(fs, parent_cluster);
    for (s = 0; s < fs->sectors_per_cluster; ++s) {
        fat16_read_sector(fs, sector + s, dir_data + s * SECTOR_SIZE);
    }

    /* Find N consecutive free entries (0x00 = free, 0xE5 = deleted) */
    found_start  = -1;
    consecutive  = 0;
    for (i = 0; (uint32_t)i < cluster_size; i += 32) {
        if (dir_data[i] == 0x00 || dir_data[i] == 0xE5) {
            if (consecutive == 0)
                found_start = i;
            consecutive++;
            if (consecutive >= total_needed)
                break;
        } else {
            consecutive = 0;
            found_start = -1;
        }
    }

    if (found_start < 0 || consecutive < total_needed) {
        fprintf(stderr, "  WARNING: No room in subdir for %s\n", filename);
        free(dir_data);
        return;
    }

    /* Write LFN entries */
    pos = (uint32_t)found_start;
    for (i = 0; i < lfn_count; ++i) {
        memcpy(dir_data + pos, lfn_buf + i * 32, 32);
        pos += 32;
    }

    /* Write 8.3 entry */
    memset(entry, 0, 32);
    memcpy(entry, name83, 11);
    entry[11] = (uint8_t)(is_dir ? 0x10 : 0x20);
    write_le16(entry + 26, (uint16_t)(first_cluster & 0xFFFF));
    write_le16(entry + 20, 0);
    write_le32(entry + 28, is_dir ? 0 : file_size);
    memcpy(dir_data + pos, entry, 32);

    /* Write back */
    for (s = 0; s < fs->sectors_per_cluster; ++s) {
        fat16_write_sector(fs, sector + s,
                           dir_data + s * SECTOR_SIZE);
    }

    free(dir_data);
}

/* ═══════════════════════════════════════════════════════════════════════════
 * FAT16 public API
 * ═══════════════════════════════════════════════════════════════════════════ */

/*
 * Initialise FAT16 parameters.
 * Mirrors Fat16Formatter.__init__().
 */
void fat16_init(Fat16 *fs, uint8_t *image, uint32_t fs_start,
                uint32_t fs_sectors, uint32_t spc)
{
    uint32_t data_sectors;
    uint32_t total_clusters;

    reset_short_name_counters();

    fs->image              = image;
    fs->fs_start           = fs_start;
    fs->fs_sectors         = fs_sectors;
    fs->sectors_per_cluster = spc;
    fs->reserved_sectors   = 1;
    fs->num_fats           = 2;
    fs->root_entry_count   = 512;

    /* root_dir_sectors = ceil(512 * 32 / 512) = 32 */
    fs->root_dir_sectors =
        (fs->root_entry_count * 32 + SECTOR_SIZE - 1) / SECTOR_SIZE;

    /* First estimate of fat_size (without FAT overhead) */
    data_sectors   = fs_sectors - fs->reserved_sectors - fs->root_dir_sectors;
    total_clusters = data_sectors / spc;
    /* FAT16: 2 bytes per entry */
    fs->fat_size   = (total_clusters * 2 + SECTOR_SIZE - 1) / SECTOR_SIZE;

    /* Recalculate with FAT overhead accounted for */
    data_sectors       = fs_sectors
                         - fs->reserved_sectors
                         - fs->num_fats * fs->fat_size
                         - fs->root_dir_sectors;
    fs->total_clusters = data_sectors / spc;

    fs->first_fat_sector     = fs->reserved_sectors;
    fs->first_root_dir_sector = fs->reserved_sectors
                                + fs->num_fats * fs->fat_size;
    fs->first_data_sector    = fs->first_root_dir_sector + fs->root_dir_sectors;

    fs->next_cluster    = 2;
    fs->next_root_entry = 0;

    printf("  FAT16: %u clusters, %u sec/cluster, FAT size=%u sectors\n",
           fs->total_clusters, fs->sectors_per_cluster, fs->fat_size);
    printf("  FAT16: first_fat=%u, root_dir=%u, data=%u\n",
           fs->first_fat_sector,
           fs->first_root_dir_sector,
           fs->first_data_sector);
}

/*
 * Write the FAT16 BPB (BIOS Parameter Block) / boot sector.
 * Mirrors Fat16Formatter.write_boot_sector().
 */
void fat16_write_bpb(Fat16 *fs)
{
    uint8_t bpb[SECTOR_SIZE];

    memset(bpb, 0, SECTOR_SIZE);

    /* Jump instruction */
    bpb[0] = 0xEB;
    bpb[1] = 0x3C;
    bpb[2] = 0x90;

    /* OEM name */
    memcpy(bpb + 3, "ANYOS   ", 8);

    /* BPB fields */
    write_le16(bpb + 11, 512);                          /* bytes per sector */
    bpb[13] = (uint8_t)fs->sectors_per_cluster;         /* sectors per cluster */
    write_le16(bpb + 14, fs->reserved_sectors);         /* reserved sectors */
    bpb[16] = (uint8_t)fs->num_fats;                   /* number of FATs */
    write_le16(bpb + 17, fs->root_entry_count);         /* root entry count */

    if (fs->fs_sectors < 0x10000) {
        write_le16(bpb + 19, (uint16_t)fs->fs_sectors); /* total sectors 16 */
    } else {
        write_le16(bpb + 19, 0);
    }

    bpb[21] = 0xF8;                                     /* media type: hard disk */
    write_le16(bpb + 22, fs->fat_size);                 /* FAT size 16 */
    write_le16(bpb + 24, 63);                            /* sectors per track */
    write_le16(bpb + 26, 16);                            /* number of heads */
    write_le32(bpb + 28, fs->fs_start);                  /* hidden sectors */

    if (fs->fs_sectors >= 0x10000) {
        write_le32(bpb + 32, fs->fs_sectors);            /* total sectors 32 */
    }

    /* Extended BPB (FAT16) */
    bpb[36] = 0x80;                                     /* drive number */
    bpb[37] = 0x00;                                     /* reserved */
    bpb[38] = 0x29;                                     /* extended boot signature */
    write_le32(bpb + 39, 0x12345678u);                  /* volume serial number */
    memcpy(bpb + 43, "ANYOS      ", 11);                /* volume label (11 bytes) */
    memcpy(bpb + 54, "FAT16   ",   8);                  /* filesystem type */

    /* Boot signature */
    bpb[510] = 0x55;
    bpb[511] = 0xAA;

    fat16_write_sector(fs, 0, bpb);
    printf("  FAT16: BPB written at sector %u\n", fs->fs_start);
}

/*
 * Initialise the FAT tables with reserved entries 0 and 1.
 * Mirrors Fat16Formatter.init_fat().
 */
void fat16_init_fat(Fat16 *fs)
{
    uint8_t  fat_sector[SECTOR_SIZE];
    uint32_t fat_idx;

    memset(fat_sector, 0, SECTOR_SIZE);
    write_le16(fat_sector + 0, 0xFFF8u);  /* Entry 0: media descriptor */
    write_le16(fat_sector + 2, 0xFFFFu);  /* Entry 1: end-of-chain */

    for (fat_idx = 0; fat_idx < fs->num_fats; ++fat_idx) {
        uint32_t fat_start = fs->first_fat_sector + fat_idx * fs->fat_size;
        fat16_write_sector(fs, fat_start, fat_sector);
    }
}

/*
 * Create a subdirectory.  Returns the new directory's first cluster.
 * Mirrors Fat16Formatter.create_directory().
 */
uint32_t fat16_create_dir(Fat16 *fs, uint32_t parent, const char *name,
                          int is_root_parent)
{
    uint32_t dir_cluster;
    uint32_t cluster_size;
    uint8_t *dir_data;
    uint8_t  dot[32];
    uint8_t  dotdot[32];
    uint32_t sector;
    uint32_t s;

    dir_cluster  = fat16_alloc_clusters(fs, 1);
    cluster_size = fs->sectors_per_cluster * SECTOR_SIZE;
    dir_data     = (uint8_t *)calloc(1, cluster_size);
    if (!dir_data)
        fatal("fat16_create_dir: calloc(%u) failed", cluster_size);

    /* "." entry */
    memset(dot, 0, 32);
    memcpy(dot, ".          ", 11);
    dot[11] = 0x10;                             /* DIRECTORY */
    write_le16(dot + 26, (uint16_t)dir_cluster);
    memcpy(dir_data, dot, 32);

    /* ".." entry */
    memset(dotdot, 0, 32);
    memcpy(dotdot, "..         ", 11);
    dotdot[11] = 0x10;                          /* DIRECTORY */
    /* parent_val = 0 if root parent, else parent cluster */
    {
        uint16_t parent_val = is_root_parent ? 0 : (uint16_t)parent;
        write_le16(dotdot + 26, parent_val);
    }
    memcpy(dir_data + 32, dotdot, 32);

    /* Write directory cluster */
    sector = fat16_cluster_to_sector(fs, dir_cluster);
    for (s = 0; s < fs->sectors_per_cluster; ++s) {
        fat16_write_sector(fs, sector + s,
                           dir_data + s * SECTOR_SIZE);
    }
    free(dir_data);

    /* Add entry to parent */
    if (is_root_parent) {
        fat16_add_root_dir_entry(fs, name, dir_cluster, 0, 1 /* is_dir */);
    } else {
        fat16_add_subdir_entry(fs, parent, name, dir_cluster, 0, 1 /* is_dir */);
    }

    return dir_cluster;
}

/*
 * Add a file to a directory.
 * Mirrors Fat16Formatter.add_file().
 */
void fat16_add_file(Fat16 *fs, uint32_t parent, const char *name,
                    const uint8_t *data, size_t size,
                    int is_root_parent)
{
    uint32_t cluster_size;
    uint32_t num_clusters;
    uint32_t first_cluster;

    if (size == 0) {
        /* Empty file: no clusters needed */
        if (is_root_parent)
            fat16_add_root_dir_entry(fs, name, 0, 0, 0);
        else
            fat16_add_subdir_entry(fs, parent, name, 0, 0, 0);
        return;
    }

    cluster_size  = fs->sectors_per_cluster * SECTOR_SIZE;
    num_clusters  = (uint32_t)((size + cluster_size - 1) / cluster_size);
    first_cluster = fat16_alloc_clusters(fs, num_clusters);
    fat16_write_to_clusters(fs, first_cluster, data, size);

    if (is_root_parent)
        fat16_add_root_dir_entry(fs, name, first_cluster, (uint32_t)size, 0);
    else
        fat16_add_subdir_entry(fs, parent, name, first_cluster, (uint32_t)size, 0);

    printf("    File: %s (%zu bytes, %u cluster(s), start=%u)\n",
           name, size, num_clusters, first_cluster);
}

/* ── Recursive sysroot population ─────────────────────────────────────────── */

/*
 * Names to skip when traversing the sysroot.
 * Mirrors Python's `if entry_name in ('.DS_Store', '.git', '.gitignore', '.gitkeep')`.
 */
static int should_skip(const char *name)
{
    return (strcmp(name, ".DS_Store") == 0 ||
            strcmp(name, ".git")      == 0 ||
            strcmp(name, ".gitignore")== 0 ||
            strcmp(name, ".gitkeep")  == 0);
}

/*
 * Internal recursive worker.  Mirrors Fat16Formatter._populate_dir().
 */
static void fat16_populate_dir(Fat16 *fs, const char *host_path,
                               uint32_t parent_cluster, int is_root)
{
    DIR           *d;
    struct dirent *ent;
    /* We collect names first to sort them (matches Python's sorted(os.listdir())) */
    char         **names      = NULL;
    int            name_count = 0;
    int            name_cap   = 0;
    int            i;

    d = opendir(host_path);
    if (!d) {
        fprintf(stderr, "  WARNING: Cannot open directory %s\n", host_path);
        return;
    }

    /* Collect non-skipped entry names */
    while ((ent = readdir(d)) != NULL) {
        const char *n = ent->d_name;

        /* Skip "." and ".." */
        if (n[0] == '.' && (n[1] == '\0' || (n[1] == '.' && n[2] == '\0')))
            continue;

        if (should_skip(n))
            continue;

        /* Grow array */
        if (name_count >= name_cap) {
            int new_cap = (name_cap == 0) ? 64 : name_cap * 2;
            char **tmp  = (char **)realloc(names, (size_t)new_cap * sizeof(char *));
            if (!tmp)
                fatal("fat16_populate_dir: realloc failed");
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
        char full_path[4096];
        struct stat st;

        snprintf(full_path, sizeof(full_path), "%s/%s", host_path, entry_name);

        if (stat(full_path, &st) != 0)
            continue;

        if (S_ISDIR(st.st_mode)) {
            uint32_t dir_cluster = fat16_create_dir(fs, parent_cluster,
                                                    entry_name, is_root);
            printf("    Dir:  %s/ (cluster=%u)\n", entry_name, dir_cluster);
            fat16_populate_dir(fs, full_path, dir_cluster, 0 /* not root */);
        } else if (S_ISREG(st.st_mode)) {
            size_t   file_size;
            uint8_t *file_data = read_file(full_path, &file_size);
            fat16_add_file(fs, parent_cluster, entry_name,
                           file_data, file_size, is_root);
            free(file_data);
        }

        free(names[i]);
    }

    free(names);
}

/*
 * Recursively populate the FAT16 filesystem from a sysroot directory.
 * Mirrors Fat16Formatter.populate_from_sysroot().
 */
void fat16_populate_sysroot(Fat16 *fs, const char *sysroot_path)
{
    struct stat st;

    if (stat(sysroot_path, &st) != 0 || !S_ISDIR(st.st_mode)) {
        printf("  Warning: sysroot path '%s' does not exist, skipping\n",
               sysroot_path);
        return;
    }

    /* Write volume label entry as root entry 0 */
    {
        uint8_t  label_entry[32];
        uint32_t entry_offset;
        uint32_t sector_in_root;
        uint32_t offset_in_sector;
        uint32_t sector;
        uint8_t  sector_data[SECTOR_SIZE];

        memset(label_entry, 0, 32);
        memcpy(label_entry, "ANYOS      ", 11);
        label_entry[11] = 0x08;     /* Volume label attribute */

        entry_offset     = fs->next_root_entry * 32;
        sector_in_root   = entry_offset / SECTOR_SIZE;
        offset_in_sector = entry_offset % SECTOR_SIZE;
        sector           = fs->first_root_dir_sector + sector_in_root;

        fat16_read_sector(fs, sector, sector_data);
        memcpy(sector_data + offset_in_sector, label_entry, 32);
        fat16_write_sector(fs, sector, sector_data);
        fs->next_root_entry++;
    }

    fat16_populate_dir(fs, sysroot_path, 0 /* unused for root */, 1 /* is_root */);
}
