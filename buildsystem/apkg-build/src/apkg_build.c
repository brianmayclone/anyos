/**
 * apkg-build — Create anyOS package archives (.tar.gz)
 *
 * Takes a package source directory containing pkg.json and a files/
 * subdirectory, and produces a .tar.gz archive suitable for distribution
 * via an apkg repository.
 *
 * Usage:
 *   apkg-build -d <package-dir> -o <output.tar.gz>
 *   apkg-build -d <package-dir>                      (auto-name from pkg.json)
 *
 * Package directory layout:
 *   <package-dir>/
 *     pkg.json          Metadata (required)
 *     files/            Payload files (required)
 *       System/bin/...  Files to install at /System/bin/...
 *
 * The archive will contain:
 *   <name>-<version>/pkg.json
 *   <name>-<version>/files/...
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <dirent.h>
#include <sys/stat.h>
#include <unistd.h>
#include <time.h>

/* ── Tar format constants ──────────────────────────────────────────── */

#define TAR_BLOCK  512
#define TAR_NAME_LEN 100
#define TAR_PREFIX_LEN 155

/* Maximum files in a package */
#define MAX_FILES 4096

/* ── Simple JSON value extraction (no full parser needed) ──────────── */

/**
 * Extract a string value for a key from a JSON object (top-level only).
 * Returns pointer into json_buf (modifies buf), or NULL.
 */
static char *json_get_string(const char *json, const char *key, char *buf, size_t buf_sz)
{
    char search[256];
    snprintf(search, sizeof(search), "\"%s\"", key);
    const char *p = strstr(json, search);
    if (!p) return NULL;

    p += strlen(search);
    /* Skip whitespace and colon */
    while (*p == ' ' || *p == '\t' || *p == '\n' || *p == '\r' || *p == ':')
        p++;

    if (*p != '"') return NULL;
    p++; /* skip opening quote */

    size_t i = 0;
    while (*p && *p != '"' && i < buf_sz - 1) {
        if (*p == '\\' && *(p + 1)) {
            p++; /* skip escape */
        }
        buf[i++] = *p++;
    }
    buf[i] = '\0';
    return buf;
}

/* ── Tar header creation ──────────────────────────────────────────── */

static void tar_write_octal(char *dst, size_t len, unsigned long val)
{
    char tmp[32];
    snprintf(tmp, sizeof(tmp), "%0*lo", (int)(len - 1), val);
    memcpy(dst, tmp, len - 1);
    dst[len - 1] = '\0';
}

static unsigned int tar_checksum(const char *header)
{
    unsigned int sum = 0;
    for (int i = 0; i < TAR_BLOCK; i++) {
        if (i >= 148 && i < 156)
            sum += ' '; /* checksum field treated as spaces */
        else
            sum += (unsigned char)header[i];
    }
    return sum;
}

static void tar_make_header(char *header, const char *name, size_t size,
                             unsigned int mode, int is_dir)
{
    memset(header, 0, TAR_BLOCK);

    /* Handle long names with prefix split */
    if (strlen(name) <= TAR_NAME_LEN) {
        strncpy(header, name, TAR_NAME_LEN);
    } else {
        /* Find split point: last '/' within first PREFIX_LEN chars */
        const char *split = NULL;
        for (int i = TAR_PREFIX_LEN - 1; i >= 0; i--) {
            if (name[i] == '/') {
                split = &name[i];
                break;
            }
        }
        if (split) {
            size_t prefix_len = split - name;
            memcpy(header + 345, name, prefix_len);         /* prefix at offset 345 */
            strncpy(header, split + 1, TAR_NAME_LEN);       /* name = rest after / */
        } else {
            strncpy(header, name, TAR_NAME_LEN);  /* truncate */
        }
    }

    tar_write_octal(header + 100, 8, mode);            /* mode */
    tar_write_octal(header + 108, 8, 0);               /* uid */
    tar_write_octal(header + 116, 8, 0);               /* gid */
    tar_write_octal(header + 124, 12, size);            /* size */
    tar_write_octal(header + 136, 12, (unsigned long)time(NULL)); /* mtime */
    header[156] = is_dir ? '5' : '0';                  /* typeflag */
    memcpy(header + 257, "ustar", 5);                  /* magic */
    memcpy(header + 263, "00", 2);                     /* version */
    strcpy(header + 265, "root");                      /* uname */
    strcpy(header + 297, "root");                      /* gname */

    /* Compute checksum */
    unsigned int cksum = tar_checksum(header);
    snprintf(header + 148, 7, "%06o", cksum);
    header[154] = '\0';
    header[155] = ' ';
}

/* ── File list collection ──────────────────────────────────────────── */

typedef struct {
    char path[512];    /* path on host filesystem */
    char arcname[512]; /* path inside archive */
    int is_dir;
    size_t size;
} FileEntry;

static FileEntry g_files[MAX_FILES];
static int g_file_count = 0;

static void collect_files(const char *base_dir, const char *arc_prefix)
{
    DIR *dir = opendir(base_dir);
    if (!dir) return;

    struct dirent *ent;
    while ((ent = readdir(dir)) != NULL) {
        if (strcmp(ent->d_name, ".") == 0 || strcmp(ent->d_name, "..") == 0)
            continue;
        if (g_file_count >= MAX_FILES) break;

        char path[512], arcname[512];
        snprintf(path, sizeof(path), "%s/%s", base_dir, ent->d_name);
        snprintf(arcname, sizeof(arcname), "%s%s", arc_prefix, ent->d_name);

        struct stat st;
        if (stat(path, &st) != 0) continue;

        if (S_ISDIR(st.st_mode)) {
            /* Add directory entry */
            FileEntry *fe = &g_files[g_file_count++];
            strncpy(fe->path, path, sizeof(fe->path) - 1);
            snprintf(fe->arcname, sizeof(fe->arcname), "%s/", arcname);
            fe->is_dir = 1;
            fe->size = 0;
            /* Recurse */
            char sub_prefix[512];
            snprintf(sub_prefix, sizeof(sub_prefix), "%s/", arcname);
            collect_files(path, sub_prefix);
        } else if (S_ISREG(st.st_mode)) {
            FileEntry *fe = &g_files[g_file_count++];
            strncpy(fe->path, path, sizeof(fe->path) - 1);
            strncpy(fe->arcname, arcname, sizeof(fe->arcname) - 1);
            fe->is_dir = 0;
            fe->size = st.st_size;
        }
    }
    closedir(dir);
}

/* ── Gzip wrapper (uses system gzip command) ──────────────────────── */

static int gzip_file(const char *in_path, const char *out_path)
{
    char cmd[1024];
    snprintf(cmd, sizeof(cmd), "gzip -c '%s' > '%s'", in_path, out_path);
    return system(cmd);
}

/* ── Main ──────────────────────────────────────────────────────────── */

static void usage(void)
{
    fprintf(stderr,
        "Usage: apkg-build -d <package-dir> [-o <output.tar.gz>]\n"
        "\n"
        "Create an anyOS package archive from a package directory.\n"
        "\n"
        "The package directory must contain:\n"
        "  pkg.json    Package metadata\n"
        "  files/      Payload files to install\n"
        "\n"
        "Options:\n"
        "  -d <dir>    Package source directory (required)\n"
        "  -o <file>   Output .tar.gz file (default: <name>-<version>.tar.gz)\n"
        "  -h          Show this help\n"
    );
}

int main(int argc, char **argv)
{
    const char *pkg_dir = NULL;
    const char *output = NULL;

    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], "-d") == 0 && i + 1 < argc) {
            pkg_dir = argv[++i];
        } else if (strcmp(argv[i], "-o") == 0 && i + 1 < argc) {
            output = argv[++i];
        } else if (strcmp(argv[i], "-h") == 0 || strcmp(argv[i], "--help") == 0) {
            usage();
            return 0;
        } else {
            fprintf(stderr, "apkg-build: unknown option '%s'\n", argv[i]);
            usage();
            return 1;
        }
    }

    if (!pkg_dir) {
        fprintf(stderr, "apkg-build: -d <package-dir> is required\n");
        usage();
        return 1;
    }

    /* Read pkg.json */
    char pkg_json_path[512];
    snprintf(pkg_json_path, sizeof(pkg_json_path), "%s/pkg.json", pkg_dir);

    FILE *f = fopen(pkg_json_path, "r");
    if (!f) {
        fprintf(stderr, "apkg-build: cannot open %s\n", pkg_json_path);
        return 1;
    }

    fseek(f, 0, SEEK_END);
    long json_len = ftell(f);
    fseek(f, 0, SEEK_SET);
    char *json_buf = malloc(json_len + 1);
    if (!json_buf) {
        fprintf(stderr, "apkg-build: out of memory\n");
        fclose(f);
        return 1;
    }
    fread(json_buf, 1, json_len, f);
    json_buf[json_len] = '\0';
    fclose(f);

    /* Extract name and version */
    char name[128], version[64];
    if (!json_get_string(json_buf, "name", name, sizeof(name))) {
        fprintf(stderr, "apkg-build: pkg.json missing 'name' field\n");
        free(json_buf);
        return 1;
    }
    if (!json_get_string(json_buf, "version", version, sizeof(version))) {
        fprintf(stderr, "apkg-build: pkg.json missing 'version' field\n");
        free(json_buf);
        return 1;
    }

    /* Validate name (alphanumeric + hyphens) */
    for (const char *p = name; *p; p++) {
        if (!((*p >= 'a' && *p <= 'z') || (*p >= '0' && *p <= '9') || *p == '-')) {
            fprintf(stderr, "apkg-build: invalid package name '%s' (use lowercase + hyphens)\n", name);
            free(json_buf);
            return 1;
        }
    }

    /* Validate files/ directory exists */
    char files_dir[512];
    snprintf(files_dir, sizeof(files_dir), "%s/files", pkg_dir);
    struct stat st;
    if (stat(files_dir, &st) != 0 || !S_ISDIR(st.st_mode)) {
        fprintf(stderr, "apkg-build: %s/files/ directory not found\n", pkg_dir);
        free(json_buf);
        return 1;
    }

    /* Determine output path */
    char output_buf[512];
    if (!output) {
        snprintf(output_buf, sizeof(output_buf), "%s-%s.tar.gz", name, version);
        output = output_buf;
    }

    /* Archive prefix (e.g., "wget-1.2.0/") */
    char prefix[256];
    snprintf(prefix, sizeof(prefix), "%s-%s", name, version);

    /* Write uncompressed tar first, then gzip it */
    char tar_path[512];
    snprintf(tar_path, sizeof(tar_path), "%s.tar.tmp", output);

    FILE *tar = fopen(tar_path, "wb");
    if (!tar) {
        fprintf(stderr, "apkg-build: cannot create %s\n", tar_path);
        free(json_buf);
        return 1;
    }

    char header[TAR_BLOCK];
    char zeros[TAR_BLOCK];
    memset(zeros, 0, TAR_BLOCK);

    /* Add top-level directory */
    char dir_name[256];
    snprintf(dir_name, sizeof(dir_name), "%s/", prefix);
    tar_make_header(header, dir_name, 0, 0755, 1);
    fwrite(header, 1, TAR_BLOCK, tar);

    /* Add pkg.json */
    char arc_pkg_json[256];
    snprintf(arc_pkg_json, sizeof(arc_pkg_json), "%s/pkg.json", prefix);
    tar_make_header(header, arc_pkg_json, json_len, 0644, 0);
    fwrite(header, 1, TAR_BLOCK, tar);
    fwrite(json_buf, 1, json_len, tar);
    /* Pad to block boundary */
    size_t remainder = json_len % TAR_BLOCK;
    if (remainder > 0) {
        fwrite(zeros, 1, TAR_BLOCK - remainder, tar);
    }

    /* Collect and add files */
    char files_prefix[256];
    snprintf(files_prefix, sizeof(files_prefix), "%s/files/", prefix);

    /* Add files/ directory entry */
    tar_make_header(header, files_prefix, 0, 0755, 1);
    fwrite(header, 1, TAR_BLOCK, tar);

    g_file_count = 0;
    collect_files(files_dir, files_prefix);

    int file_count = 0;
    size_t total_size = 0;

    for (int i = 0; i < g_file_count; i++) {
        FileEntry *fe = &g_files[i];

        if (fe->is_dir) {
            tar_make_header(header, fe->arcname, 0, 0755, 1);
            fwrite(header, 1, TAR_BLOCK, tar);
        } else {
            tar_make_header(header, fe->arcname, fe->size, 0755, 0);
            fwrite(header, 1, TAR_BLOCK, tar);

            /* Write file content */
            FILE *src = fopen(fe->path, "rb");
            if (src) {
                char buf[4096];
                size_t written = 0;
                size_t n;
                while ((n = fread(buf, 1, sizeof(buf), src)) > 0) {
                    fwrite(buf, 1, n, tar);
                    written += n;
                }
                fclose(src);

                /* Pad to block boundary */
                remainder = written % TAR_BLOCK;
                if (remainder > 0) {
                    fwrite(zeros, 1, TAR_BLOCK - remainder, tar);
                }

                file_count++;
                total_size += fe->size;
            } else {
                fprintf(stderr, "apkg-build: warning: cannot read '%s'\n", fe->path);
            }
        }
    }

    /* End of archive marker (two zero blocks) */
    fwrite(zeros, 1, TAR_BLOCK, tar);
    fwrite(zeros, 1, TAR_BLOCK, tar);
    fclose(tar);

    /* Gzip the tar file */
    if (gzip_file(tar_path, output) != 0) {
        fprintf(stderr, "apkg-build: gzip compression failed\n");
        unlink(tar_path);
        free(json_buf);
        return 1;
    }
    unlink(tar_path); /* Remove uncompressed tar */

    printf("apkg-build: created %s (%d files, %zu bytes payload)\n",
           output, file_count, total_size);

    free(json_buf);
    return 0;
}
