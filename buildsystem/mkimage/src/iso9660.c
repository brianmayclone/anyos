/*
 * iso9660.c — ISO 9660 + El Torito bootable CD-ROM image creation
 *
 * Faithful C99 port of the Iso9660Creator class and create_iso_image()
 * function from mkimage.py (lines 1497-1967).
 *
 * Written in C99 for TCC compatibility.
 */

#include "mkimage.h"

#include <ctype.h>
#include <dirent.h>
#include <sys/stat.h>
#include <time.h>

/* ── Constants ───────────────────────────────────────────────────────────── */

#define MAX_ISO_DIRS  256
#define MAX_ISO_FILES 1024

/* El Torito boot image starts at CD sector 22, occupies 16 sectors (32 KiB). */
#define BOOT_IMAGE_LBA     22
#define BOOT_IMAGE_SECTORS 16

/* First directory extent LBA = boot_image_lba + boot_image_sectors */
#define DIR_LBA_START      (BOOT_IMAGE_LBA + BOOT_IMAGE_SECTORS)  /* 38 */

/* Kernel physical base address (1 MiB) */
#define KERNEL_LMA  0x00100000ULL

/* ── Data structures ─────────────────────────────────────────────────────── */

typedef struct {
    char path[256];         /* e.g. "/", "/bin", "/System" */
    char children[64][64];  /* child directory names */
    int  nchildren;
    char files[128][64];    /* file names in this dir */
    int  nfiles;
    uint32_t lba;           /* assigned LBA */
} IsoDir;

typedef struct {
    char     path[256];     /* e.g. "/bin/cat" */
    uint8_t *data;
    size_t   size;
    uint32_t lba;
} IsoFile;

/* ── both-endian helpers ─────────────────────────────────────────────────── */

/*
 * both_endian_u32 — write val at out[0..7] as LE32 followed by BE32.
 * ISO 9660 "both byte order" format for 32-bit fields.
 */
static void both_endian_u32(uint8_t *out, uint32_t val)
{
    write_le32(out,     val);
    write_be32(out + 4, val);
}

/*
 * both_endian_u16 — write val at out[0..3] as LE16 followed by BE16.
 * ISO 9660 "both byte order" format for 16-bit fields.
 */
static void both_endian_u16(uint8_t *out, uint16_t val)
{
    write_le16(out,     val);
    write_be16(out + 2, val);
}

/* ── Timestamp helpers ───────────────────────────────────────────────────── */

/*
 * iso_datetime_now — fill out[0..6] with a 7-byte ISO 9660 directory-record
 * date/time: year-1900, month, day, hour, min, sec, GMT offset (0 = UTC).
 */
static void iso_datetime_now(uint8_t *out)
{
    time_t     t  = time(NULL);
    struct tm *tm = localtime(&t);

    out[0] = (uint8_t)(tm->tm_year);   /* years since 1900 */
    out[1] = (uint8_t)(tm->tm_mon + 1);
    out[2] = (uint8_t)(tm->tm_mday);
    out[3] = (uint8_t)(tm->tm_hour);
    out[4] = (uint8_t)(tm->tm_min);
    out[5] = (uint8_t)(tm->tm_sec);
    out[6] = 0;                         /* GMT offset in 15-min units */
}

/*
 * iso_dec_datetime_now — fill out[0..16] with a 17-byte ISO 9660 PVD
 * date/time string: "YYYYMMDDHHMMSSCC" (16 ASCII digits) + GMT offset byte.
 */
static void iso_dec_datetime_now(uint8_t *out)
{
    time_t     t  = time(NULL);
    struct tm *tm = localtime(&t);
    char       buf[17];

    /* 16 ASCII characters: YYYYMMDDHHMMSSCC */
    snprintf(buf, sizeof(buf), "%04d%02d%02d%02d%02d%02d%02d",
             tm->tm_year + 1900,
             tm->tm_mon + 1,
             tm->tm_mday,
             tm->tm_hour,
             tm->tm_min,
             tm->tm_sec,
             0 /* centiseconds */);

    memcpy(out, buf, 16);
    out[16] = 0;  /* GMT offset */
}

/* ── Directory record builder ────────────────────────────────────────────── */

/*
 * make_dir_record — write an ISO 9660 directory record into out[].
 *
 * Returns the number of bytes written (33 + name_len, rounded up to even).
 *
 * Parameters:
 *   out      — destination buffer (must have at least 34 + name_len bytes)
 *   lba      — extent location (CD sector number)
 *   data_len — data length of the extent
 *   flags    — file flags (0x02 = directory, 0x00 = file)
 *   name     — identifier bytes
 *   name_len — length of identifier
 */
static int make_dir_record(uint8_t *out,
                            uint32_t lba, uint32_t data_len,
                            uint8_t flags,
                            const uint8_t *name, int name_len)
{
    int rec_len = 33 + name_len;
    if (rec_len & 1)
        rec_len++;  /* pad to even */

    memset(out, 0, (size_t)rec_len);

    out[0] = (uint8_t)rec_len;   /* Length of Directory Record */
    out[1] = 0;                   /* Extended Attribute Record Length */

    both_endian_u32(out + 2, lba);        /* Location of Extent (8 bytes) */
    both_endian_u32(out + 10, data_len);  /* Data Length (8 bytes) */

    iso_datetime_now(out + 18);           /* Recording Date and Time (7 bytes) */

    out[25] = flags;  /* File Flags */
    out[26] = 0;      /* File Unit Size */
    out[27] = 0;      /* Interleave Gap Size */

    both_endian_u16(out + 28, 1);  /* Volume Sequence Number (4 bytes) */

    out[32] = (uint8_t)name_len;   /* Length of File Identifier */
    memcpy(out + 33, name, (size_t)name_len);

    return rec_len;
}

/* ── Sysroot collector ───────────────────────────────────────────────────── */

/*
 * collect_sysroot — recursively walk host_path and register entries in dirs[]
 * and files[].  Skips names that begin with '.'.  Mirrors _collect_dir() from
 * the Python Iso9660Creator class.
 *
 * host_path: absolute path on the build host (e.g. /path/to/sysroot/bin)
 * iso_path:  corresponding ISO path (e.g. /bin)
 * dirs:      flat array of IsoDir, length *ndirs
 * files:     flat array of IsoFile, length *nfiles
 */
static void collect_sysroot(const char *host_path, const char *iso_path,
                             IsoDir *dirs, int *ndirs,
                             IsoFile *files, int *nfiles);

/*
 * find_or_add_dir — locate iso_path in dirs[], creating it if absent.
 * Returns the index into dirs[].
 */
static int find_or_add_dir(IsoDir *dirs, int *ndirs, const char *iso_path)
{
    int i;
    for (i = 0; i < *ndirs; i++) {
        if (strcmp(dirs[i].path, iso_path) == 0)
            return i;
    }
    if (*ndirs >= MAX_ISO_DIRS)
        fatal("collect_sysroot: too many directories (max %d)", MAX_ISO_DIRS);

    i = *ndirs;
    memset(&dirs[i], 0, sizeof(IsoDir));
    strncpy(dirs[i].path, iso_path, sizeof(dirs[i].path) - 1);
    (*ndirs)++;
    return i;
}

static void collect_sysroot(const char *host_path, const char *iso_path,
                             IsoDir *dirs, int *ndirs,
                             IsoFile *files, int *nfiles)
{
    DIR *dp = opendir(host_path);
    if (!dp)
        return;

    /* Ensure the directory entry exists */
    int didx = find_or_add_dir(dirs, ndirs, iso_path);

    /* Collect and sort entries (sorted() in Python) */
    struct dirent *ent;

    /* First pass: gather names into a temporary list for sorting */
#define MAX_ENTRIES 1024
    char enames[MAX_ENTRIES][256];
    int  nent = 0;

    while ((ent = readdir(dp)) != NULL) {
        if (ent->d_name[0] == '.')
            continue;
        if (nent < MAX_ENTRIES) {
            strncpy(enames[nent], ent->d_name, 255);
            enames[nent][255] = '\0';
            nent++;
        }
    }
    closedir(dp);

    /* Simple insertion sort (mirrors Python's sorted()) */
    {
        int a, b;
        char tmp[256];
        for (a = 1; a < nent; a++) {
            strncpy(tmp, enames[a], 256);
            for (b = a - 1; b >= 0 && strcmp(enames[b], tmp) > 0; b--)
                strncpy(enames[b + 1], enames[b], 256);
            strncpy(enames[b + 1], tmp, 256);
        }
    }

    /* Second pass: categorise each entry */
    {
        int e;
        for (e = 0; e < nent; e++) {
            char full[512];
            char child_iso[512];
            struct stat st;

            snprintf(full, sizeof(full), "%s/%s", host_path, enames[e]);

            /* Build child ISO path */
            {
                size_t plen = strlen(iso_path);
                if (plen > 0 && iso_path[plen - 1] == '/')
                    snprintf(child_iso, sizeof(child_iso), "%s%s", iso_path, enames[e]);
                else
                    snprintf(child_iso, sizeof(child_iso), "%s/%s", iso_path, enames[e]);
            }

            if (stat(full, &st) != 0)
                continue;

            /* Re-fetch didx after recursive calls may have grown dirs[] */
            didx = find_or_add_dir(dirs, ndirs, iso_path);

            if (S_ISDIR(st.st_mode)) {
                /* Register as a child directory */
                if (dirs[didx].nchildren < 64) {
                    strncpy(dirs[didx].children[dirs[didx].nchildren],
                            enames[e], 63);
                    dirs[didx].children[dirs[didx].nchildren][63] = '\0';
                    dirs[didx].nchildren++;
                }
                /* Recurse */
                collect_sysroot(full, child_iso, dirs, ndirs, files, nfiles);

            } else if (S_ISREG(st.st_mode)) {
                /* Register file in this directory */
                if (dirs[didx].nfiles < 128) {
                    strncpy(dirs[didx].files[dirs[didx].nfiles],
                            enames[e], 63);
                    dirs[didx].files[dirs[didx].nfiles][63] = '\0';
                    dirs[didx].nfiles++;
                }

                /* Add to global file list */
                if (*nfiles >= MAX_ISO_FILES)
                    fatal("collect_sysroot: too many files (max %d)", MAX_ISO_FILES);

                size_t fsize;
                uint8_t *fdata = read_file(full, &fsize);

                strncpy(files[*nfiles].path, child_iso,
                        sizeof(files[*nfiles].path) - 1);
                files[*nfiles].path[sizeof(files[*nfiles].path) - 1] = '\0';
                files[*nfiles].data = fdata;
                files[*nfiles].size = fsize;
                files[*nfiles].lba  = 0;
                (*nfiles)++;
            }
        }
    }
#undef MAX_ENTRIES
}

/* ── Directory-list sorting (mirrors Python's sorted(self.dirs.keys())) ─── */

static int cmp_str(const void *a, const void *b)
{
    return strcmp((const char *)a, (const char *)b);
}

/* ── Path Table builder helpers ─────────────────────────────────────────── */

/*
 * parent_iso_path — fill parent[] with the ISO path of d's parent.
 * e.g.  "/bin/utils" -> "/bin",  "/bin" -> "/",  "/" -> "/"
 */
static void parent_iso_path(const char *d, char *parent, size_t parent_sz)
{
    const char *last_slash;
    size_t plen;

    /* Find last '/' */
    last_slash = strrchr(d, '/');
    if (!last_slash || last_slash == d) {
        /* d is "/" or has no parent above root */
        strncpy(parent, "/", parent_sz - 1);
        parent[parent_sz - 1] = '\0';
        return;
    }

    plen = (size_t)(last_slash - d);
    if (plen >= parent_sz)
        plen = parent_sz - 1;
    strncpy(parent, d, plen);
    parent[plen] = '\0';
}

/* ── ISO directory name → basename (last component) ─────────────────────── */

static const char *iso_basename(const char *path)
{
    const char *p = strrchr(path, '/');
    return p ? p + 1 : path;
}

/* ── ISO 9660 uppercase file name with version suffix ────────────────────── */

/*
 * iso_file_name — convert a file name to ISO 9660 level-1 form:
 *   uppercase, then add ".;1" if no extension or ";1" after the extension.
 * out must be at least strlen(name) + 4 bytes.
 */
static void iso_file_name(const char *name, char *out)
{
    char upper[64];
    size_t i, len;
    const char *dot;

    len = strlen(name);
    if (len >= sizeof(upper))
        len = sizeof(upper) - 1;
    for (i = 0; i < len; i++)
        upper[i] = (char)toupper((unsigned char)name[i]);
    upper[len] = '\0';

    /* Mirrors Python: if '.' not in iso_name: iso_name += '.' */
    dot = strchr(upper, '.');
    if (!dot) {
        /* Append '.' then ';1' */
        snprintf(out, 256, "%s.;1", upper);
    } else {
        /* Append ';1' after extension */
        snprintf(out, 256, "%s;1", upper);
    }
}

/* ── PVD builder ─────────────────────────────────────────────────────────── */

static void make_pvd(uint8_t *pvd,
                     uint32_t total_blocks,
                     uint32_t root_dir_lba,
                     uint32_t root_dir_size,
                     uint32_t path_table_lba,
                     uint32_t path_table_size)
{
    memset(pvd, 0, ISO_BLOCK_SIZE);

    pvd[0] = 1;                      /* Type: Primary Volume Descriptor */
    memcpy(pvd + 1, "CD001", 5);     /* Standard Identifier */
    pvd[6] = 1;                      /* Version */

    /* System Identifier (bytes 8-39, 32 chars, space-padded) */
    memset(pvd + 8, ' ', 32);
    memcpy(pvd + 8, "ANYOS", 5);

    /* Volume Identifier (bytes 40-71, 32 chars, space-padded) */
    {
        const char *vol_id = "ANYOS_LIVE";
        size_t vlen = strlen(vol_id);
        memset(pvd + 40, ' ', 32);
        memcpy(pvd + 40, vol_id, vlen < 32 ? vlen : 32);
    }

    /* Volume Space Size (bytes 80-87, both-endian u32) */
    both_endian_u32(pvd + 80, total_blocks);

    /* Volume Set Size (bytes 120-123, both-endian u16) */
    both_endian_u16(pvd + 120, 1);

    /* Volume Sequence Number (bytes 124-127, both-endian u16) */
    both_endian_u16(pvd + 124, 1);

    /* Logical Block Size (bytes 128-131, both-endian u16) */
    both_endian_u16(pvd + 128, (uint16_t)ISO_BLOCK_SIZE);

    /* Path Table Size (bytes 132-139, both-endian u32) */
    both_endian_u32(pvd + 132, path_table_size);

    /* Type L Path Table Location (bytes 140-143, u32 LE) */
    write_le32(pvd + 140, path_table_lba);
    /* Optional Type L (bytes 144-147): 0 */

    /* Type M Path Table Location (bytes 148-151, u32 BE) */
    write_be32(pvd + 148, path_table_lba + 1);
    /* Optional Type M (bytes 152-155): 0 */

    /* Root Directory Record (bytes 156-189, 34 bytes) */
    {
        uint8_t dot_name = 0x00;
        make_dir_record(pvd + 156, root_dir_lba, root_dir_size,
                        0x02, &dot_name, 1);
    }

    /* Application Identifier (bytes 574-701, 128 chars, space-padded) */
    memset(pvd + 574, ' ', 128);
    memcpy(pvd + 574, "ANYOS MKIMAGE", 13);

    /* Volume Creation Date/Time (bytes 813-829, 17 bytes) */
    iso_dec_datetime_now(pvd + 813);

    /* Volume Modification Date/Time (bytes 830-846, 17 bytes) */
    iso_dec_datetime_now(pvd + 830);

    /* File Structure Version (byte 881) */
    pvd[881] = 1;
}

/* ── Path-table directory-number lookup ──────────────────────────────────── */

/*
 * dir_num_for — return the 1-based directory number for iso_path.
 * sorted_dirs[][256] and dir_numbers[] are parallel arrays of length nsorted.
 * Returns 1 (root) if not found.
 */
static int dir_num_for(const char *iso_path,
                        char sorted_dirs[][256],
                        const int *dir_numbers,
                        int nsorted)
{
    int i;
    for (i = 0; i < nsorted; i++)
        if (strcmp(sorted_dirs[i], iso_path) == 0)
            return dir_numbers[i];
    return 1;
}

/* ── Main entry point ────────────────────────────────────────────────────── */

void create_iso_image(const Args *args)
{
    /* ── Validate arguments ──────────────────────────────────────────────── */
    if (!args->stage1 || !args->stage2 || !args->kernel) {
        fprintf(stderr,
                "ERROR: --stage1, --stage2, and --kernel are required for ISO mode\n");
        exit(1);
    }

    /* ── Read stage1 (must be exactly 512 bytes) ─────────────────────────── */
    size_t   stage1_size;
    uint8_t *stage1 = read_file(args->stage1, &stage1_size);
    if (stage1_size != SECTOR_SIZE)
        fatal("Stage 1 must be exactly %d bytes (got %zu)", SECTOR_SIZE, stage1_size);

    /* ── Read stage2 (max 63 * 512 = 32256 bytes) ───────────────────────── */
    size_t   stage2_size;
    uint8_t *stage2 = read_file(args->stage2, &stage2_size);
    {
        size_t stage2_max = 63 * SECTOR_SIZE;
        if (stage2_size > stage2_max)
            fatal("Stage 2 is %zu bytes, max is %zu", stage2_size, stage2_max);
    }

    /* ── Read kernel ELF and convert to flat binary ─────────────────────── */
    size_t   kernel_elf_size;
    uint8_t *kernel_elf = read_file(args->kernel, &kernel_elf_size);

    printf("Kernel ELF: %zu bytes\n", kernel_elf_size);

    size_t   kernel_flat_size;
    uint8_t *kernel_flat = elf_to_flat(kernel_elf, kernel_elf_size,
                                        KERNEL_LMA, &kernel_flat_size);
    if (!kernel_flat)
        fatal("Failed to convert kernel ELF to flat binary");

    free(kernel_elf);

    {
        uint32_t kernel_disk_sectors =
            (uint32_t)((kernel_flat_size + SECTOR_SIZE - 1) / SECTOR_SIZE);
        printf("\nISO 9660 Live CD image:\n");
        printf("  Stage 1: %zu bytes\n", stage1_size);
        printf("  Stage 2: %zu bytes\n", stage2_size);
        printf("  Kernel:  %zu bytes (%u disk sectors)\n",
               kernel_flat_size, kernel_disk_sectors);
    }

    /* ── Collect sysroot ─────────────────────────────────────────────────── */
    IsoDir  *dirs  = (IsoDir  *)calloc(MAX_ISO_DIRS,  sizeof(IsoDir));
    IsoFile *files = (IsoFile *)calloc(MAX_ISO_FILES, sizeof(IsoFile));
    if (!dirs || !files)
        fatal("create_iso_image: out of memory");

    int ndirs  = 0;
    int nfiles = 0;

    /* Always have a root directory entry */
    find_or_add_dir(dirs, &ndirs, "/");

    if (args->sysroot) {
        printf("  Populating ISO from sysroot: %s\n", args->sysroot);
        collect_sysroot(args->sysroot, "/", dirs, &ndirs, files, &nfiles);
    }

    /* ── Sort directory list (mirrors Python's sorted(self.dirs.keys())) ── */
    /*
     * We need a sorted array of directory paths for stable LBA assignment
     * and path table generation.  Build a separate index array and sort it.
     */
    char sorted_dirs[MAX_ISO_DIRS][256];
    int  nsorted = ndirs;
    {
        int i;
        for (i = 0; i < ndirs; i++)
            strncpy(sorted_dirs[i], dirs[i].path, 255);
        qsort(sorted_dirs, (size_t)nsorted, 256, cmp_str);
    }

    /* Helper: find IsoDir by iso_path */
#define FIND_DIR(iso_path_) ({ \
    int _fi; \
    IsoDir *_d = NULL; \
    for (_fi = 0; _fi < ndirs; _fi++) \
        if (strcmp(dirs[_fi].path, (iso_path_)) == 0) { _d = &dirs[_fi]; break; } \
    _d; \
})

    /* ── Assign LBAs to directories ──────────────────────────────────────── */
    /*
     * Directories start at sector DIR_LBA_START (38).
     * Allocate 1 sector per directory initially; expand if needed during
     * extent construction (Python comment: "for simplicity, just allocate more").
     */
    uint32_t next_lba = DIR_LBA_START;
    {
        int i;
        for (i = 0; i < nsorted; i++) {
            IsoDir *d = NULL;
            int j;
            for (j = 0; j < ndirs; j++)
                if (strcmp(dirs[j].path, sorted_dirs[i]) == 0) { d = &dirs[j]; break; }
            if (!d) continue;
            d->lba = next_lba;
            next_lba++;
        }
    }

    /* ── Kernel data LBA (after directories) ─────────────────────────────── */
    uint32_t kernel_lba     = next_lba;
    uint32_t kernel_cd_secs = (uint32_t)((kernel_flat_size + ISO_BLOCK_SIZE - 1)
                                          / ISO_BLOCK_SIZE);
    uint32_t file_data_lba  = kernel_lba + kernel_cd_secs;

    /* ── Assign LBAs to files (sorted by path, mirrors Python) ──────────── */
    {
        /* Sort file paths */
        char sorted_fpaths[MAX_ISO_FILES][256];
        int i;
        for (i = 0; i < nfiles; i++)
            strncpy(sorted_fpaths[i], files[i].path, 255);
        qsort(sorted_fpaths, (size_t)nfiles, 256, cmp_str);

        uint32_t cur_lba = file_data_lba;
        for (i = 0; i < nfiles; i++) {
            /* Find the file entry */
            int j;
            for (j = 0; j < nfiles; j++) {
                if (strcmp(files[j].path, sorted_fpaths[i]) == 0) {
                    files[j].lba = cur_lba;
                    uint32_t secs = (uint32_t)((files[j].size + ISO_BLOCK_SIZE - 1)
                                                / ISO_BLOCK_SIZE);
                    cur_lba += secs > 0 ? secs : 1;
                    break;
                }
            }
        }
        next_lba = cur_lba;
    }

    uint32_t total_sectors = next_lba;

    /* ── Build directory extents ─────────────────────────────────────────── */
    /*
     * Each directory gets a flat byte buffer of its extent (rounded to
     * ISO_BLOCK_SIZE).  We keep them in a parallel array indexed by
     * sorted_dirs[].
     */
    uint8_t *dir_extents[MAX_ISO_DIRS];
    size_t   dir_extent_sizes[MAX_ISO_DIRS];
    memset(dir_extents,      0, sizeof(dir_extents));
    memset(dir_extent_sizes, 0, sizeof(dir_extent_sizes));

    {
        int i;
        for (i = 0; i < nsorted; i++) {
            const char *dpath = sorted_dirs[i];

            /* Locate IsoDir */
            IsoDir *d = NULL;
            int j;
            for (j = 0; j < ndirs; j++)
                if (strcmp(dirs[j].path, dpath) == 0) { d = &dirs[j]; break; }
            if (!d) continue;

            /* Allocate a generous work buffer (we'll trim to sector boundary) */
            size_t   cap  = (size_t)(ISO_BLOCK_SIZE * 16);  /* enough for most dirs */
            uint8_t *ext  = (uint8_t *)calloc(1, cap);
            size_t   pos  = 0;

            if (!ext)
                fatal("create_iso_image: out of memory for dir extent");

#define APPEND_REC(lba_, dlen_, flags_, namep_, namelen_) do { \
    int _rlen = make_dir_record(ext + pos, (lba_), (dlen_), \
                                (flags_), (namep_), (namelen_)); \
    pos += (size_t)_rlen; \
} while (0)

            /* "." entry */
            {
                uint8_t dot = 0x00;
                APPEND_REC(d->lba, ISO_BLOCK_SIZE, 0x02, &dot, 1);
            }

            /* ".." entry */
            {
                char parent[256];
                parent_iso_path(dpath, parent, sizeof(parent));
                uint32_t parent_lba = d->lba;  /* fallback */
                int k;
                for (k = 0; k < ndirs; k++) {
                    if (strcmp(dirs[k].path, parent) == 0) {
                        parent_lba = dirs[k].lba;
                        break;
                    }
                }
                uint8_t dotdot = 0x01;
                APPEND_REC(parent_lba, ISO_BLOCK_SIZE, 0x02, &dotdot, 1);
            }

            /* Child directories */
            {
                int c;
                for (c = 0; c < d->nchildren; c++) {
                    char child_iso[512];
                    size_t plen = strlen(dpath);
                    if (plen > 0 && dpath[plen - 1] == '/')
                        snprintf(child_iso, sizeof(child_iso), "%s%s",
                                 dpath, d->children[c]);
                    else
                        snprintf(child_iso, sizeof(child_iso), "%s/%s",
                                 dpath, d->children[c]);

                    uint32_t child_lba = 0;
                    int k;
                    for (k = 0; k < ndirs; k++) {
                        if (strcmp(dirs[k].path, child_iso) == 0) {
                            child_lba = dirs[k].lba;
                            break;
                        }
                    }

                    /* ISO 9660 dir name: uppercase */
                    char uname[64];
                    size_t nl = strlen(d->children[c]);
                    size_t m;
                    for (m = 0; m < nl && m < 63; m++)
                        uname[m] = (char)toupper(
                            (unsigned char)d->children[c][m]);
                    uname[nl < 63 ? nl : 63] = '\0';

                    APPEND_REC(child_lba, ISO_BLOCK_SIZE, 0x02,
                               (const uint8_t *)uname, (int)strlen(uname));
                }
            }

            /* Files in this directory */
            {
                int f;
                for (f = 0; f < d->nfiles; f++) {
                    /* Build full ISO path of this file */
                    char fiso[512];
                    size_t plen = strlen(dpath);
                    if (plen > 0 && dpath[plen - 1] == '/')
                        snprintf(fiso, sizeof(fiso), "%s%s",
                                 dpath, d->files[f]);
                    else
                        snprintf(fiso, sizeof(fiso), "%s/%s",
                                 dpath, d->files[f]);

                    /* Find file entry */
                    uint32_t flba  = 0;
                    size_t   fsize = 0;
                    int k;
                    for (k = 0; k < nfiles; k++) {
                        if (strcmp(files[k].path, fiso) == 0) {
                            flba  = files[k].lba;
                            fsize = files[k].size;
                            break;
                        }
                    }

                    /* ISO 9660 file name: uppercase + ";1" suffix */
                    char iso_name[256];
                    iso_file_name(d->files[f], iso_name);

                    APPEND_REC(flba, (uint32_t)fsize, 0x00,
                               (const uint8_t *)iso_name,
                               (int)strlen(iso_name));
                }
            }

#undef APPEND_REC

            /* Pad to ISO_BLOCK_SIZE boundary */
            while (pos % ISO_BLOCK_SIZE != 0)
                ext[pos++] = 0x00;

            dir_extents[i]      = ext;
            dir_extent_sizes[i] = pos;
        }
    }

    /* ── Build Path Table (L-type, little-endian) ────────────────────────── */
    /*
     * Entry format: dir_id_len(1), ext_attr_len(1), extent_lba(4 LE),
     *               parent_dir_num(2 LE), dir_id(N), pad(1 if N is odd).
     * Root directory identifier = 0x01.
     */

    /* Assign directory numbers (1-based, in sorted order) */
    int dir_numbers[MAX_ISO_DIRS];  /* indexed by sorted_dirs[] position */
    {
        int i;
        for (i = 0; i < nsorted; i++)
            dir_numbers[i] = i + 1;
    }

    /* Helper: find dir_number for a given iso_path */
#define DIR_NUM_FOR(iso_path_) \
    dir_num_for((iso_path_), sorted_dirs, dir_numbers, nsorted)

    uint8_t path_table_l[ISO_BLOCK_SIZE];
    uint8_t path_table_m[ISO_BLOCK_SIZE];
    size_t  pt_l_len = 0;
    size_t  pt_m_len = 0;

    memset(path_table_l, 0, sizeof(path_table_l));
    memset(path_table_m, 0, sizeof(path_table_m));

    {
        int i;
        for (i = 0; i < nsorted; i++) {
            const char *dpath = sorted_dirs[i];
            uint8_t name_bytes[64];
            int     name_len;
            int     parent_num;

            /* Locate LBA */
            uint32_t d_lba = 0;
            int j;
            for (j = 0; j < ndirs; j++)
                if (strcmp(dirs[j].path, dpath) == 0) { d_lba = dirs[j].lba; break; }

            if (strcmp(dpath, "/") == 0) {
                /* Root: identifier = 0x01, parent = 1 */
                name_bytes[0] = 0x01;
                name_len       = 1;
                parent_num     = 1;
            } else {
                /* Basename in uppercase */
                const char *bn = iso_basename(dpath);
                size_t blen = strlen(bn);
                size_t m;
                for (m = 0; m < blen && m < 63; m++)
                    name_bytes[m] = (uint8_t)toupper((unsigned char)bn[m]);
                name_len = (int)(blen < 63 ? blen : 63);

                char parent[256];
                parent_iso_path(dpath, parent, sizeof(parent));
                parent_num = DIR_NUM_FOR(parent);
            }

            /* L-type entry (LE) */
            {
                uint8_t *p = path_table_l + pt_l_len;
                p[0] = (uint8_t)name_len;
                p[1] = 0;                         /* ext attr */
                write_le32(p + 2, d_lba);
                write_le16(p + 6, (uint16_t)parent_num);
                memcpy(p + 8, name_bytes, (size_t)name_len);
                pt_l_len += 8 + (size_t)name_len;
                if (name_len & 1)
                    pt_l_len++;  /* odd-length padding byte (already 0) */
            }

            /* M-type entry (BE) */
            {
                uint8_t *p = path_table_m + pt_m_len;
                p[0] = (uint8_t)name_len;
                p[1] = 0;
                write_be32(p + 2, d_lba);
                write_be16(p + 6, (uint16_t)parent_num);
                memcpy(p + 8, name_bytes, (size_t)name_len);
                pt_m_len += 8 + (size_t)name_len;
                if (name_len & 1)
                    pt_m_len++;
            }
        }
    }

    uint32_t path_table_size = (uint32_t)pt_l_len;

    /* ── Build PVD ───────────────────────────────────────────────────────── */
    /* Root dir is sorted_dirs[0] (should be "/") */
    uint32_t root_dir_lba  = 0;
    uint32_t root_dir_size = ISO_BLOCK_SIZE;
    {
        int j;
        for (j = 0; j < ndirs; j++) {
            if (strcmp(dirs[j].path, "/") == 0) {
                root_dir_lba = dirs[j].lba;
                break;
            }
        }
        if (dir_extent_sizes[0] > 0)
            root_dir_size = (uint32_t)dir_extent_sizes[0];
    }

    uint8_t pvd[ISO_BLOCK_SIZE];
    make_pvd(pvd, total_sectors, root_dir_lba, root_dir_size,
             20 /* path_table_lba */, path_table_size);

    /* ── Build El Torito Boot Record Volume Descriptor (sector 17) ──────── */
    uint8_t brvd[ISO_BLOCK_SIZE];
    memset(brvd, 0, ISO_BLOCK_SIZE);
    brvd[0] = 0;                          /* Boot Record type */
    memcpy(brvd + 1, "CD001", 5);
    brvd[6] = 1;                          /* Version */
    /* Boot System Identifier: "EL TORITO SPECIFICATION" (32 bytes padded) */
    memcpy(brvd + 7, "EL TORITO SPECIFICATION", 23);
    /* Boot Catalog LBA at offset 71 (LE u32) */
    write_le32(brvd + 71, 19);

    /* ── Build VD Set Terminator (sector 18) ─────────────────────────────── */
    uint8_t vdst[ISO_BLOCK_SIZE];
    memset(vdst, 0, ISO_BLOCK_SIZE);
    vdst[0] = 255;
    memcpy(vdst + 1, "CD001", 5);
    vdst[6] = 1;

    /* ── Build Boot Catalog (sector 19) ──────────────────────────────────── */
    /*
     * Validation Entry (32 bytes at offset 0):
     *   byte 0:   Header ID = 0x01
     *   byte 1:   Platform ID = 0x00 (x86)
     *   byte 2-3: Reserved = 0
     *   byte 4-27: ID string (zeroed)
     *   byte 28-29: Checksum (16-bit, sum of all 16-bit LE words == 0)
     *   byte 30:  Key byte 0x55
     *   byte 31:  Key byte 0xAA
     *
     * Initial/Default Entry (32 bytes at offset 32):
     *   byte 32: Bootable = 0x88
     *   byte 33: No emulation = 0x00
     *   byte 34-35: Load Segment = 0x0000 (default 0x07C0)
     *   byte 36: System Type = 0x00
     *   byte 37: Unused = 0x00
     *   byte 38-39: Sector Count = 64 (512-byte sectors)
     *   byte 40-43: Load RBA = BOOT_IMAGE_LBA (LE u32)
     */
    uint8_t boot_cat[ISO_BLOCK_SIZE];
    memset(boot_cat, 0, ISO_BLOCK_SIZE);

    /* Validation Entry */
    boot_cat[0]  = 0x01;   /* Header ID */
    boot_cat[1]  = 0x00;   /* Platform ID: x86 */
    boot_cat[30] = 0x55;   /* Key byte 1 */
    boot_cat[31] = 0xAA;   /* Key byte 2 */

    /* Compute checksum: sum of all 16-bit LE words in validation entry == 0 */
    {
        uint32_t sum = 0;
        int i;
        for (i = 0; i < 32; i += 2) {
            uint16_t w = (uint16_t)(boot_cat[i] | ((uint16_t)boot_cat[i + 1] << 8));
            sum += w;
        }
        uint16_t checksum = (uint16_t)((0x10000u - (sum & 0xFFFFu)) & 0xFFFFu);
        write_le16(boot_cat + 28, checksum);
    }

    /* Initial/Default Entry (at offset 32) */
    boot_cat[32] = 0x88;   /* Bootable indicator */
    boot_cat[33] = 0x00;   /* No emulation */
    write_le16(boot_cat + 34, 0x0000);   /* Load Segment (default 0x7C0) */
    boot_cat[36] = 0x00;   /* System Type */
    boot_cat[37] = 0x00;   /* Unused */
    write_le16(boot_cat + 38, 64);       /* Sector Count: 64 x 512-byte sectors */
    write_le32(boot_cat + 40, BOOT_IMAGE_LBA);  /* Load RBA */

    /* ── Patch stage2 with kernel location ───────────────────────────────── */
    /*
     * At offset 2: kernel_disk_sectors (LE16)  — number of 512-byte sectors
     * At offset 4: kernel_disk_lba     (LE32)  — CD sector * 4
     *              (CD sectors are 2048 bytes; disk sectors are 512 bytes)
     */
    uint8_t *stage2_patched = (uint8_t *)malloc(stage2_size);
    if (!stage2_patched)
        fatal("create_iso_image: out of memory for patched stage2");
    memcpy(stage2_patched, stage2, stage2_size);

    {
        uint32_t kernel_disk_lba =
            kernel_lba * (uint32_t)(ISO_BLOCK_SIZE / SECTOR_SIZE);
        uint32_t kernel_disk_secs =
            (uint32_t)((kernel_flat_size + SECTOR_SIZE - 1) / SECTOR_SIZE);

        if (stage2_size >= 8) {
            write_le16(stage2_patched + 2, (uint16_t)kernel_disk_secs);
            write_le32(stage2_patched + 4, kernel_disk_lba);
        }

        printf("  Stage2 patched: kernel at disk LBA %u, %u sectors\n",
               kernel_disk_lba, kernel_disk_secs);
        printf("  Kernel at CD sector %u (%zu bytes, %u CD sectors)\n",
               kernel_lba, kernel_flat_size, kernel_cd_secs);
    }

    /* ── Allocate and zero the image buffer ─────────────────────────────── */
    size_t   image_size = (size_t)total_sectors * ISO_BLOCK_SIZE;
    uint8_t *image      = (uint8_t *)calloc(1, image_size);
    if (!image)
        fatal("create_iso_image: calloc(%zu) failed", image_size);

    /* ── System area (sectors 0-15): stage1 + stage2 for HDD boot ───────── */
    memcpy(image, stage1, stage1_size);
    memcpy(image + SECTOR_SIZE, stage2_patched, stage2_size);

    /* ── El Torito boot image (sectors 22-37): same boot code for CD boot ── */
    {
        size_t bi_off = (size_t)BOOT_IMAGE_LBA * ISO_BLOCK_SIZE;
        memcpy(image + bi_off, stage1, stage1_size);
        memcpy(image + bi_off + SECTOR_SIZE, stage2_patched, stage2_size);
    }

    /* ── PVD at sector 16 ────────────────────────────────────────────────── */
    memcpy(image + 16 * ISO_BLOCK_SIZE, pvd, ISO_BLOCK_SIZE);

    /* ── BRVD at sector 17 ───────────────────────────────────────────────── */
    memcpy(image + 17 * ISO_BLOCK_SIZE, brvd, ISO_BLOCK_SIZE);

    /* ── VD Terminator at sector 18 ─────────────────────────────────────── */
    memcpy(image + 18 * ISO_BLOCK_SIZE, vdst, ISO_BLOCK_SIZE);

    /* ── Boot Catalog at sector 19 ───────────────────────────────────────── */
    memcpy(image + 19 * ISO_BLOCK_SIZE, boot_cat, ISO_BLOCK_SIZE);

    /* ── Path Table L at sector 20 ───────────────────────────────────────── */
    memcpy(image + 20 * ISO_BLOCK_SIZE, path_table_l, pt_l_len);

    /* ── Path Table M at sector 21 ───────────────────────────────────────── */
    memcpy(image + 21 * ISO_BLOCK_SIZE, path_table_m, pt_m_len);

    /* ── Directory extents ───────────────────────────────────────────────── */
    {
        int i;
        for (i = 0; i < nsorted; i++) {
            if (!dir_extents[i])
                continue;
            /* Find LBA for this sorted directory */
            uint32_t d_lba = 0;
            int j;
            for (j = 0; j < ndirs; j++)
                if (strcmp(dirs[j].path, sorted_dirs[i]) == 0) {
                    d_lba = dirs[j].lba;
                    break;
                }
            size_t d_off = (size_t)d_lba * ISO_BLOCK_SIZE;
            memcpy(image + d_off, dir_extents[i], dir_extent_sizes[i]);
        }
    }

    /* ── Kernel flat binary ──────────────────────────────────────────────── */
    {
        size_t k_off = (size_t)kernel_lba * ISO_BLOCK_SIZE;
        memcpy(image + k_off, kernel_flat, kernel_flat_size);
    }

    /* ── File data ───────────────────────────────────────────────────────── */
    {
        int i;
        for (i = 0; i < nfiles; i++) {
            size_t f_off = (size_t)files[i].lba * ISO_BLOCK_SIZE;
            memcpy(image + f_off, files[i].data, files[i].size);
        }
    }

    /* ── Write output image ──────────────────────────────────────────────── */
    {
        FILE *fp = fopen(args->output, "wb");
        if (!fp)
            fatal("Cannot open output file: %s", args->output);
        if (fwrite(image, 1, image_size, fp) != image_size)
            fatal("Failed to write ISO image to %s", args->output);
        fclose(fp);
    }

    {
        double iso_size_mb = (double)image_size / (1024.0 * 1024.0);
        uint32_t kernel_disk_lba =
            kernel_lba * (uint32_t)(ISO_BLOCK_SIZE / SECTOR_SIZE);

        printf("\n  ISO 9660 image: %s (%.1f MiB, %u CD sectors)\n",
               args->output, iso_size_mb, total_sectors);
        printf("  Files: %d, Directories: %d\n", nfiles, ndirs);
        printf("\nISO image created: %s\n", args->output);
        printf("  Boot: El Torito no-emulation, 64 sectors loaded at 0x7C00\n");
        printf("  Kernel at CD sector %u (disk LBA %u)\n",
               kernel_lba, kernel_disk_lba);
    }

    /* ── Cleanup ─────────────────────────────────────────────────────────── */
    {
        int i;
        for (i = 0; i < nsorted; i++)
            if (dir_extents[i]) free(dir_extents[i]);
        for (i = 0; i < nfiles; i++)
            if (files[i].data) free(files[i].data);
    }
    free(dirs);
    free(files);
    free(image);
    free(stage1);
    free(stage2);
    free(stage2_patched);
    free(kernel_flat);

#undef DIR_NUM_FOR
#undef FIND_DIR
}
