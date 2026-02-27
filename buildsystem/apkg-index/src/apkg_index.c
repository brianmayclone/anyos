/**
 * apkg-index — Generate repository index.json from package archives.
 *
 * Scans a directory of .tar.gz package archives, extracts pkg.json metadata
 * from each, computes MD5 checksums, and writes a consolidated index.json.
 *
 * Usage:
 *   apkg-index -d <packages-dir> -o <index.json> [-n <repo-name>] [-a <arch>]
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <dirent.h>
#include <sys/stat.h>
#include <unistd.h>
#include <time.h>

/* ── Simple JSON value extraction ──────────────────────────────────── */

/**
 * Extract a string value for a key from a JSON object (top-level only).
 * Returns pointer into buf, or NULL if key not found.
 */
static char *json_get_string(const char *json, const char *key, char *buf, size_t buf_sz)
{
    char search[256];
    snprintf(search, sizeof(search), "\"%s\"", key);
    const char *p = strstr(json, search);
    if (!p) return NULL;

    p += strlen(search);
    while (*p == ' ' || *p == '\t' || *p == '\n' || *p == '\r' || *p == ':')
        p++;

    if (*p != '"') return NULL;
    p++;

    size_t i = 0;
    while (*p && *p != '"' && i < buf_sz - 1) {
        if (*p == '\\' && *(p + 1)) p++;
        buf[i++] = *p++;
    }
    buf[i] = '\0';
    return buf;
}

/**
 * Extract a numeric value for a key from a JSON object (top-level only).
 * Returns the number as a long, or default_val if not found.
 */
static long json_get_number(const char *json, const char *key, long default_val)
{
    char search[256];
    snprintf(search, sizeof(search), "\"%s\"", key);
    const char *p = strstr(json, search);
    if (!p) return default_val;

    p += strlen(search);
    while (*p == ' ' || *p == '\t' || *p == '\n' || *p == '\r' || *p == ':')
        p++;

    if (*p == '-' || (*p >= '0' && *p <= '9'))
        return atol(p);
    return default_val;
}

/* ── MD5 implementation (RFC 1321) ─────────────────────────────────── */

typedef struct {
    unsigned int state[4];
    unsigned long long count;
    unsigned char buffer[64];
} MD5_CTX;

#define F(x,y,z) (((x)&(y))|((~(x))&(z)))
#define G(x,y,z) (((x)&(z))|((y)&(~(z))))
#define H(x,y,z) ((x)^(y)^(z))
#define I(x,y,z) ((y)^((x)|(~(z))))
#define ROTL(x,n) (((x)<<(n))|((x)>>(32-(n))))

static const unsigned int md5_k[64] = {
    0xd76aa478,0xe8c7b756,0x242070db,0xc1bdceee,0xf57c0faf,0x4787c62a,
    0xa8304613,0xfd469501,0x698098d8,0x8b44f7af,0xffff5bb1,0x895cd7be,
    0x6b901122,0xfd987193,0xa679438e,0x49b40821,0xf61e2562,0xc040b340,
    0x265e5a51,0xe9b6c7aa,0xd62f105d,0x02441453,0xd8a1e681,0xe7d3fbc8,
    0x21e1cde6,0xc33707d6,0xf4d50d87,0x455a14ed,0xa9e3e905,0xfcefa3f8,
    0x676f02d9,0x8d2a4c8a,0xfffa3942,0x8771f681,0x6d9d6122,0xfde5380c,
    0xa4beea44,0x4bdecfa9,0xf6bb4b60,0xbebfbc70,0x289b7ec6,0xeaa127fa,
    0xd4ef3085,0x04881d05,0xd9d4d039,0xe6db99e5,0x1fa27cf8,0xc4ac5665,
    0xf4292244,0x432aff97,0xab9423a7,0xfc93a039,0x655b59c3,0x8f0ccc92,
    0xffeff47d,0x85845dd1,0x6fa87e4f,0xfe2ce6e0,0xa3014314,0x4e0811a1,
    0xf7537e82,0xbd3af235,0x2ad7d2bb,0xeb86d391
};
static const int md5_s[64] = {
    7,12,17,22,7,12,17,22,7,12,17,22,7,12,17,22,
    5,9,14,20,5,9,14,20,5,9,14,20,5,9,14,20,
    4,11,16,23,4,11,16,23,4,11,16,23,4,11,16,23,
    6,10,15,21,6,10,15,21,6,10,15,21,6,10,15,21
};

static void md5_transform(MD5_CTX *ctx, const unsigned char *block)
{
    unsigned int a = ctx->state[0], b = ctx->state[1];
    unsigned int c = ctx->state[2], d = ctx->state[3];
    unsigned int m[16];
    for (int i = 0; i < 16; i++) {
        m[i] = (unsigned int)block[i*4] | ((unsigned int)block[i*4+1]<<8)
             | ((unsigned int)block[i*4+2]<<16) | ((unsigned int)block[i*4+3]<<24);
    }
    for (int i = 0; i < 64; i++) {
        unsigned int f, g;
        if (i < 16)      { f = F(b,c,d); g = i; }
        else if (i < 32) { f = G(b,c,d); g = (5*i+1)%16; }
        else if (i < 48) { f = H(b,c,d); g = (3*i+5)%16; }
        else              { f = I(b,c,d); g = (7*i)%16; }
        unsigned int temp = d;
        d = c; c = b;
        b = b + ROTL(a + f + md5_k[i] + m[g], md5_s[i]);
        a = temp;
    }
    ctx->state[0] += a; ctx->state[1] += b;
    ctx->state[2] += c; ctx->state[3] += d;
}

static void md5_init(MD5_CTX *ctx)
{
    ctx->state[0] = 0x67452301; ctx->state[1] = 0xefcdab89;
    ctx->state[2] = 0x98badcfe; ctx->state[3] = 0x10325476;
    ctx->count = 0;
}

static void md5_update(MD5_CTX *ctx, const unsigned char *data, size_t len)
{
    size_t idx = (size_t)(ctx->count % 64);
    ctx->count += len;
    for (size_t i = 0; i < len; i++) {
        ctx->buffer[idx++] = data[i];
        if (idx == 64) { md5_transform(ctx, ctx->buffer); idx = 0; }
    }
}

static void md5_final(MD5_CTX *ctx, unsigned char digest[16])
{
    unsigned long long bits = ctx->count * 8;
    size_t idx = (size_t)(ctx->count % 64);
    ctx->buffer[idx++] = 0x80;
    if (idx > 56) {
        while (idx < 64) ctx->buffer[idx++] = 0;
        md5_transform(ctx, ctx->buffer);
        idx = 0;
    }
    while (idx < 56) ctx->buffer[idx++] = 0;
    for (int i = 0; i < 8; i++)
        ctx->buffer[56+i] = (unsigned char)(bits >> (8*i));
    md5_transform(ctx, ctx->buffer);
    for (int i = 0; i < 4; i++) {
        digest[i*4]   = (unsigned char)(ctx->state[i]);
        digest[i*4+1] = (unsigned char)(ctx->state[i]>>8);
        digest[i*4+2] = (unsigned char)(ctx->state[i]>>16);
        digest[i*4+3] = (unsigned char)(ctx->state[i]>>24);
    }
}

static void md5_file(const char *path, char hex[33])
{
    FILE *f = fopen(path, "rb");
    if (!f) { memset(hex, '0', 32); hex[32] = '\0'; return; }

    MD5_CTX ctx;
    md5_init(&ctx);
    unsigned char buf[4096];
    size_t n;
    while ((n = fread(buf, 1, sizeof(buf), f)) > 0) {
        md5_update(&ctx, buf, n);
    }
    fclose(f);

    unsigned char digest[16];
    md5_final(&ctx, digest);
    for (int i = 0; i < 16; i++)
        sprintf(hex + i*2, "%02x", digest[i]);
    hex[32] = '\0';
}

/* ── Tar.gz pkg.json extraction (minimal) ──────────────────────────── */

/**
 * Extract pkg.json content from a .tar.gz package.
 * Uses system `tar` command for simplicity.
 * Returns allocated buffer with JSON content, or NULL.
 */
static char *extract_pkg_json(const char *archive_path)
{
    /* Create temp file for extraction */
    char cmd[1024];
    char tmpfile[] = "/tmp/apkg-index-XXXXXX";
    int fd = mkstemp(tmpfile);
    if (fd < 0) return NULL;
    close(fd);

    /* Extract pkg.json from the archive */
    snprintf(cmd, sizeof(cmd),
        "tar xzf '%s' --to-stdout --wildcards '*/pkg.json' > '%s' 2>/dev/null",
        archive_path, tmpfile);
    int ret = system(cmd);
    if (ret != 0) {
        /* Try alternate extraction */
        snprintf(cmd, sizeof(cmd),
            "tar xzf '%s' -O --include='*/pkg.json' > '%s' 2>/dev/null",
            archive_path, tmpfile);
        ret = system(cmd);
    }

    FILE *f = fopen(tmpfile, "r");
    if (!f) { unlink(tmpfile); return NULL; }

    fseek(f, 0, SEEK_END);
    long len = ftell(f);
    fseek(f, 0, SEEK_SET);

    if (len <= 0) { fclose(f); unlink(tmpfile); return NULL; }

    char *buf = malloc(len + 1);
    if (!buf) { fclose(f); unlink(tmpfile); return NULL; }
    fread(buf, 1, len, f);
    buf[len] = '\0';
    fclose(f);
    unlink(tmpfile);
    return buf;
}

/* ── JSON string escaping ──────────────────────────────────────────── */

static void json_write_escaped(FILE *out, const char *s)
{
    fputc('"', out);
    for (const char *p = s; *p; p++) {
        switch (*p) {
        case '"': fputs("\\\"", out); break;
        case '\\': fputs("\\\\", out); break;
        case '\n': fputs("\\n", out); break;
        case '\r': fputs("\\r", out); break;
        case '\t': fputs("\\t", out); break;
        default:
            if ((unsigned char)*p < 0x20) fprintf(out, "\\u%04x", *p);
            else fputc(*p, out);
        }
    }
    fputc('"', out);
}

/**
 * Extract a JSON array as a string (verbatim from source).
 * Returns pointer into json source, or "[]".
 */
static const char *json_get_array(const char *json, const char *key)
{
    char search[256];
    snprintf(search, sizeof(search), "\"%s\"", key);
    const char *p = strstr(json, search);
    if (!p) return "[]";
    p += strlen(search);
    while (*p == ' ' || *p == '\t' || *p == '\n' || *p == '\r' || *p == ':') p++;
    if (*p != '[') return "[]";
    return p;
}

/**
 * Copy a JSON array verbatim to output.
 */
static void json_copy_array(FILE *out, const char *arr_start)
{
    if (*arr_start != '[') { fputs("[]", out); return; }
    int depth = 0;
    const char *p = arr_start;
    do {
        if (*p == '[') depth++;
        else if (*p == ']') depth--;
        fputc(*p, out);
        p++;
    } while (depth > 0 && *p);
}

/* ── Main ──────────────────────────────────────────────────────────── */

static void usage(void)
{
    fprintf(stderr,
        "Usage: apkg-index -d <packages-dir> -o <index.json> [-n <name>] [-a <arch>]\n"
        "\n"
        "Generate a repository index from .tar.gz package archives.\n"
        "\n"
        "Options:\n"
        "  -d <dir>    Directory containing .tar.gz packages (required)\n"
        "  -o <file>   Output index.json file (required)\n"
        "  -n <name>   Repository name (default: \"anyOS Packages\")\n"
        "  -a <arch>   Architecture filter (default: all)\n"
        "  -h          Show this help\n"
    );
}

int main(int argc, char **argv)
{
    const char *pkg_dir = NULL;
    const char *output = NULL;
    const char *repo_name = "anyOS Packages";
    const char *arch_filter = NULL;

    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], "-d") == 0 && i + 1 < argc)
            pkg_dir = argv[++i];
        else if (strcmp(argv[i], "-o") == 0 && i + 1 < argc)
            output = argv[++i];
        else if (strcmp(argv[i], "-n") == 0 && i + 1 < argc)
            repo_name = argv[++i];
        else if (strcmp(argv[i], "-a") == 0 && i + 1 < argc)
            arch_filter = argv[++i];
        else if (strcmp(argv[i], "-h") == 0 || strcmp(argv[i], "--help") == 0) {
            usage(); return 0;
        } else {
            fprintf(stderr, "apkg-index: unknown option '%s'\n", argv[i]);
            usage(); return 1;
        }
    }

    if (!pkg_dir || !output) {
        fprintf(stderr, "apkg-index: -d and -o are required\n");
        usage();
        return 1;
    }

    /* Scan directory for .tar.gz files */
    DIR *dir = opendir(pkg_dir);
    if (!dir) {
        fprintf(stderr, "apkg-index: cannot open directory '%s'\n", pkg_dir);
        return 1;
    }

    FILE *out = fopen(output, "w");
    if (!out) {
        fprintf(stderr, "apkg-index: cannot create '%s'\n", output);
        closedir(dir);
        return 1;
    }

    /* Generate timestamp */
    time_t now = time(NULL);
    struct tm *tm = gmtime(&now);
    char timestamp[64];
    strftime(timestamp, sizeof(timestamp), "%Y-%m-%dT%H:%M:%S", tm);

    /* Write index header */
    fprintf(out, "{\n");
    fprintf(out, "  \"repository\": ");
    json_write_escaped(out, repo_name);
    fprintf(out, ",\n");
    fprintf(out, "  \"generated\": \"%s\",\n", timestamp);
    fprintf(out, "  \"packages\": [\n");

    int pkg_count = 0;
    struct dirent *ent;

    while ((ent = readdir(dir)) != NULL) {
        size_t namelen = strlen(ent->d_name);
        if (namelen < 8) continue; /* minimum: "a.tar.gz" */

        /* Check for .tar.gz extension */
        if (strcmp(ent->d_name + namelen - 7, ".tar.gz") != 0) continue;

        char filepath[512];
        snprintf(filepath, sizeof(filepath), "%s/%s", pkg_dir, ent->d_name);

        /* Get file size */
        struct stat st;
        if (stat(filepath, &st) != 0) continue;

        /* Compute MD5 */
        char md5[33];
        md5_file(filepath, md5);

        /* Extract pkg.json */
        char *pkg_json = extract_pkg_json(filepath);
        if (!pkg_json) {
            fprintf(stderr, "apkg-index: warning: cannot read pkg.json from '%s'\n",
                    ent->d_name);
            continue;
        }

        /* Extract fields */
        char name[128], version[64], description[512], category[64];
        char type[32], arch[32], min_os_version[32];
        long size_installed;

        if (!json_get_string(pkg_json, "name", name, sizeof(name))) {
            fprintf(stderr, "apkg-index: warning: '%s' has no name\n", ent->d_name);
            free(pkg_json);
            continue;
        }
        json_get_string(pkg_json, "version", version, sizeof(version));
        json_get_string(pkg_json, "description", description, sizeof(description));
        if (!json_get_string(pkg_json, "category", category, sizeof(category)))
            strcpy(category, "");
        if (!json_get_string(pkg_json, "type", type, sizeof(type)))
            strcpy(type, "bin");
        if (!json_get_string(pkg_json, "arch", arch, sizeof(arch)))
            strcpy(arch, "x86_64");
        if (!json_get_string(pkg_json, "min_os_version", min_os_version, sizeof(min_os_version)))
            strcpy(min_os_version, "0.0.0");
        size_installed = json_get_number(pkg_json, "size_installed", 0);

        /* Apply arch filter */
        if (arch_filter && strcmp(arch, arch_filter) != 0) {
            free(pkg_json);
            continue;
        }

        /* Write package entry */
        if (pkg_count > 0) fprintf(out, ",\n");

        fprintf(out, "    {\n");
        fprintf(out, "      \"name\": "); json_write_escaped(out, name); fprintf(out, ",\n");
        fprintf(out, "      \"version\": "); json_write_escaped(out, version); fprintf(out, ",\n");
        fprintf(out, "      \"description\": "); json_write_escaped(out, description); fprintf(out, ",\n");
        fprintf(out, "      \"category\": "); json_write_escaped(out, category); fprintf(out, ",\n");
        fprintf(out, "      \"type\": "); json_write_escaped(out, type); fprintf(out, ",\n");
        fprintf(out, "      \"arch\": "); json_write_escaped(out, arch); fprintf(out, ",\n");
        fprintf(out, "      \"depends\": "); json_copy_array(out, json_get_array(pkg_json, "depends")); fprintf(out, ",\n");
        fprintf(out, "      \"provides\": "); json_copy_array(out, json_get_array(pkg_json, "provides")); fprintf(out, ",\n");
        fprintf(out, "      \"size\": %ld,\n", (long)st.st_size);
        fprintf(out, "      \"size_installed\": %ld,\n", size_installed);
        fprintf(out, "      \"md5\": \"%s\",\n", md5);
        fprintf(out, "      \"filename\": "); json_write_escaped(out, ent->d_name); fprintf(out, ",\n");
        fprintf(out, "      \"min_os_version\": "); json_write_escaped(out, min_os_version); fprintf(out, "\n");
        fprintf(out, "    }");

        pkg_count++;
        free(pkg_json);
    }

    /* Close JSON */
    fprintf(out, "\n  ]\n");
    fprintf(out, "}\n");

    fclose(out);
    closedir(dir);

    printf("apkg-index: generated %s (%d packages)\n", output, pkg_count);
    return 0;
}
