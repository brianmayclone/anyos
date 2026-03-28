/*
 * mkmanifest — Generate system-manifest.json from a sysroot directory.
 *
 * Recursively scans a sysroot directory and produces a JSON manifest
 * with MD5 checksums and file sizes for every file. Used by the
 * Software Update app to detect which files need updating.
 *
 * Usage:
 *   mkmanifest --sysroot /path/to/sysroot --version 0.4.241 -o system-manifest.json
 *
 * The output JSON has this structure:
 * {
 *   "version": "0.4.241",
 *   "file_count": 1234,
 *   "files": {
 *     "/System/bin/ls":  { "md5": "abc...", "size": 32768 },
 *     "/System/krnl64":  { "md5": "def...", "size": 524288 },
 *     ...
 *   },
 *   "bootloader": {
 *     "stage1": "stage1.bin",
 *     "stage2": "stage2.bin"
 *   }
 * }
 */

#ifdef _POSIX_C_SOURCE
/* already defined */
#elif defined(__linux__)
#define _POSIX_C_SOURCE 200809L
#endif

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <dirent.h>
#include <sys/stat.h>
#include <stdint.h>

/* ── MD5 (RFC 1321, minimal inline implementation) ─────────────────────── */

static uint32_t md5_s[64] = {
    7,12,17,22, 7,12,17,22, 7,12,17,22, 7,12,17,22,
    5, 9,14,20, 5, 9,14,20, 5, 9,14,20, 5, 9,14,20,
    4,11,16,23, 4,11,16,23, 4,11,16,23, 4,11,16,23,
    6,10,15,21, 6,10,15,21, 6,10,15,21, 6,10,15,21
};

static uint32_t md5_k[64] = {
    0xd76aa478,0xe8c7b756,0x242070db,0xc1bdceee,
    0xf57c0faf,0x4787c62a,0xa8304613,0xfd469501,
    0x698098d8,0x8b44f7af,0xffff5bb1,0x895cd7be,
    0x6b901122,0xfd987193,0xa679438e,0x49b40821,
    0xf61e2562,0xc040b340,0x265e5a51,0xe9b6c7aa,
    0xd62f105d,0x02441453,0xd8a1e681,0xe7d3fbc8,
    0x21e1cde6,0xc33707d6,0xf4d50d87,0x455a14ed,
    0xa9e3e905,0xfcefa3f8,0x676f02d9,0x8d2a4c8a,
    0xfffa3942,0x8771f681,0x6d9d6122,0xfde5380c,
    0xa4beea44,0x4bdecfa9,0xf6bb4b60,0xbebfbc70,
    0x289b7ec6,0xeaa127fa,0xd4ef3085,0x04881d05,
    0xd9d4d039,0xe6db99e5,0x1fa27cf8,0xc4ac5665,
    0xf4292244,0x432aff97,0xab9423a7,0xfc93a039,
    0x655b59c3,0x8f0ccc92,0xffeff47d,0x85845dd1,
    0x6fa87e4f,0xfe2ce6e0,0xa3014314,0x4e0811a1,
    0xf7537e82,0xbd3af235,0x2ad7d2bb,0xeb86d391
};

static uint32_t md5_left_rotate(uint32_t x, uint32_t c) {
    return (x << c) | (x >> (32 - c));
}

static void md5_hash(const uint8_t *data, size_t len, uint8_t out[16]) {
    uint32_t a0 = 0x67452301;
    uint32_t b0 = 0xefcdab89;
    uint32_t c0 = 0x98badcfe;
    uint32_t d0 = 0x10325476;

    /* pre-processing: padding */
    size_t padded_len = ((len + 8) / 64 + 1) * 64;
    uint8_t *msg = (uint8_t *)calloc(padded_len, 1);
    if (!msg) return;
    memcpy(msg, data, len);
    msg[len] = 0x80;
    uint64_t bits = (uint64_t)len * 8;
    memcpy(msg + padded_len - 8, &bits, 8);

    for (size_t offset = 0; offset < padded_len; offset += 64) {
        uint32_t *m = (uint32_t *)(msg + offset);
        uint32_t a = a0, b = b0, c = c0, d = d0;
        for (int i = 0; i < 64; i++) {
            uint32_t f, g;
            if (i < 16)      { f = (b & c) | (~b & d); g = i; }
            else if (i < 32) { f = (d & b) | (~d & c); g = (5*i+1) % 16; }
            else if (i < 48) { f = b ^ c ^ d;          g = (3*i+5) % 16; }
            else              { f = c ^ (b | ~d);       g = (7*i) % 16; }
            f = f + a + md5_k[i] + m[g];
            a = d; d = c; c = b;
            b = b + md5_left_rotate(f, md5_s[i]);
        }
        a0 += a; b0 += b; c0 += c; d0 += d;
    }
    free(msg);

    memcpy(out +  0, &a0, 4);
    memcpy(out +  4, &b0, 4);
    memcpy(out +  8, &c0, 4);
    memcpy(out + 12, &d0, 4);
}

static void md5_hex(const uint8_t hash[16], char out[33]) {
    static const char hex[] = "0123456789abcdef";
    for (int i = 0; i < 16; i++) {
        out[i*2]   = hex[hash[i] >> 4];
        out[i*2+1] = hex[hash[i] & 0xf];
    }
    out[32] = '\0';
}

/* ── File entry list ───────────────────────────────────────────────────── */

typedef struct {
    char  path[512];   /* path relative to sysroot (e.g. "/System/bin/ls") */
    char  md5[33];
    long  size;
} FileEntry;

static FileEntry *g_entries = NULL;
static int g_count = 0;
static int g_cap = 0;

static void add_entry(const char *rel_path, const char *full_path) {
    FILE *f = fopen(full_path, "rb");
    if (!f) return;

    fseek(f, 0, SEEK_END);
    long sz = ftell(f);
    fseek(f, 0, SEEK_SET);

    uint8_t *buf = (uint8_t *)malloc(sz > 0 ? sz : 1);
    if (!buf) { fclose(f); return; }
    if (sz > 0) fread(buf, 1, sz, f);
    fclose(f);

    uint8_t hash[16];
    md5_hash(buf, sz, hash);
    free(buf);

    if (g_count >= g_cap) {
        g_cap = g_cap ? g_cap * 2 : 1024;
        g_entries = (FileEntry *)realloc(g_entries, g_cap * sizeof(FileEntry));
    }

    FileEntry *e = &g_entries[g_count++];
    snprintf(e->path, sizeof(e->path), "%s", rel_path);
    md5_hex(hash, e->md5);
    e->size = sz;
}

/* ── Recursive directory scan ──────────────────────────────────────────── */

static void scan_dir(const char *sysroot, const char *rel_prefix) {
    char full[1024];
    snprintf(full, sizeof(full), "%s%s", sysroot, rel_prefix);

    DIR *d = opendir(full);
    if (!d) return;

    struct dirent *ent;
    while ((ent = readdir(d)) != NULL) {
        if (ent->d_name[0] == '.') continue;  /* skip hidden files */

        char rel[512], abs[1024];
        snprintf(rel, sizeof(rel), "%s/%s", rel_prefix, ent->d_name);
        snprintf(abs, sizeof(abs), "%s%s", sysroot, rel);

        struct stat st;
        if (stat(abs, &st) != 0) continue;

        if (S_ISDIR(st.st_mode)) {
            scan_dir(sysroot, rel);
        } else if (S_ISREG(st.st_mode)) {
            add_entry(rel, abs);
        }
    }
    closedir(d);
}

/* ── JSON output with proper escaping ──────────────────────────────────── */

static void write_json_string(FILE *f, const char *s) {
    fputc('"', f);
    for (const char *p = s; *p; p++) {
        switch (*p) {
            case '"':  fputs("\\\"", f); break;
            case '\\': fputs("\\\\", f); break;
            case '\n': fputs("\\n", f);  break;
            case '\t': fputs("\\t", f);  break;
            default:   fputc(*p, f);     break;
        }
    }
    fputc('"', f);
}

/* ── Compare for qsort ────────────────────────────────────────────────── */

static int cmp_entry(const void *a, const void *b) {
    return strcmp(((const FileEntry *)a)->path, ((const FileEntry *)b)->path);
}

/* ── Main ──────────────────────────────────────────────────────────────── */

int main(int argc, char **argv) {
    const char *sysroot = NULL;
    const char *version = "0.0.0";
    const char *output = "system-manifest.json";

    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], "--sysroot") == 0 && i+1 < argc) {
            sysroot = argv[++i];
        } else if (strcmp(argv[i], "--version") == 0 && i+1 < argc) {
            version = argv[++i];
        } else if ((strcmp(argv[i], "-o") == 0 || strcmp(argv[i], "--output") == 0) && i+1 < argc) {
            output = argv[++i];
        } else if (strcmp(argv[i], "--help") == 0 || strcmp(argv[i], "-h") == 0) {
            fprintf(stderr, "Usage: mkmanifest --sysroot DIR --version VER [-o FILE]\n");
            return 0;
        }
    }

    if (!sysroot) {
        fprintf(stderr, "mkmanifest: --sysroot is required\n");
        return 1;
    }

    /* Scan all directories that are part of the system */
    fprintf(stderr, "mkmanifest: scanning %s ...\n", sysroot);

    scan_dir(sysroot, "/System");
    scan_dir(sysroot, "/Applications");
    scan_dir(sysroot, "/boot");

    /* Sort entries by path for deterministic output */
    if (g_count > 0) {
        qsort(g_entries, g_count, sizeof(FileEntry), cmp_entry);
    }

    fprintf(stderr, "mkmanifest: %d files found\n", g_count);

    /* Write JSON */
    FILE *out_f = fopen(output, "w");
    if (!out_f) {
        fprintf(stderr, "mkmanifest: cannot create %s\n", output);
        return 1;
    }

    fprintf(out_f, "{\n");
    fprintf(out_f, "  \"version\": \"%s\",\n", version);
    fprintf(out_f, "  \"file_count\": %d,\n", g_count);

    /* Bootloader section */
    fprintf(out_f, "  \"bootloader\": {\n");
    fprintf(out_f, "    \"stage1\": \"stage1.bin\",\n");
    fprintf(out_f, "    \"stage2\": \"stage2.bin\"\n");
    fprintf(out_f, "  },\n");

    /* Files */
    fprintf(out_f, "  \"files\": {\n");
    for (int i = 0; i < g_count; i++) {
        FileEntry *e = &g_entries[i];
        fprintf(out_f, "    ");
        write_json_string(out_f, e->path);
        fprintf(out_f, ": { \"md5\": \"%s\", \"size\": %ld }", e->md5, e->size);
        if (i < g_count - 1) fputc(',', out_f);
        fputc('\n', out_f);
    }
    fprintf(out_f, "  }\n");
    fprintf(out_f, "}\n");

    fclose(out_f);
    fprintf(stderr, "mkmanifest: wrote %s (%d files, version %s)\n", output, g_count, version);

    free(g_entries);
    return 0;
}
