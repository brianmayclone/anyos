/*
 * mkappbundle — anyOS .app bundle creator
 *
 * Creates a complete .app bundle directory from individual components.
 * Validates all inputs before creating the bundle:
 *   - Info.conf must contain required keys (id, name, exec, version, category)
 *   - Capabilities must be valid anyOS capability names
 *   - Binary must not be empty; ELF files are auto-converted via anyelf
 *   - Icon must be a valid ICO file (Windows icon format)
 *
 * Written in C99 for TCC compatibility (self-hosting on anyOS).
 *
 * Usage:
 *   mkappbundle -i <Info.conf> -e <binary> [options] -o <Output.app>
 *
 * Required:
 *   -i <path>    Info.conf metadata file
 *   -e <path>    Executable binary (flat binary or ELF — auto-converts)
 *   -o <path>    Output .app directory
 *
 * Optional:
 *   -c <path>           Icon file (must be valid ICO format)
 *   -r <path>           Resource file or directory (repeatable, max 64)
 *   --anyelf-path <p>   Path to anyelf for ELF auto-conversion
 *   -v                  Verbose output
 *   --force             Skip validation warnings (errors still abort)
 *
 * Examples:
 *   mkappbundle -i Info.conf -e Terminal -o Terminal.app
 *   mkappbundle -i Info.conf -e DOOM -c Icon.ico -r doom.wad -o DOOM.app
 *   mkappbundle -i Info.conf -e app -c Icon.ico -r syntax/ -o "anyOS Code.app"
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <errno.h>
#include <stdarg.h>
#include <stdint.h>

#ifdef _WIN32
#include <direct.h>
#include <io.h>
#define unlink _unlink
#else
#include <dirent.h>
#include <unistd.h>
#endif

#ifdef ONE_SOURCE
/* Single-source mode for TCC on anyOS — no additional files */
#endif

/* ── Constants ────────────────────────────────────────────────────────── */

#define MAX_RESOURCES   64
#define MAX_PATH_LEN  1024
#define MAX_LINE_LEN   512
#define MAX_NAME_LEN   256

/* ── Error / warning helpers ──────────────────────────────────────────── */

static int g_warnings = 0;
static int g_force    = 0;
static int g_keep_elf = 0; /* allow ELF binaries as-is (no conversion) */
static const char *g_anyelf_path = NULL; /* explicit path to anyelf, or NULL for PATH */

static void fatal(const char *fmt, ...) {
    va_list ap;
    fprintf(stderr, "mkappbundle: error: ");
    va_start(ap, fmt);
    vfprintf(stderr, fmt, ap);
    va_end(ap);
    fprintf(stderr, "\n");
    exit(1);
}

static void warn(const char *fmt, ...) {
    va_list ap;
    fprintf(stderr, "mkappbundle: warning: ");
    va_start(ap, fmt);
    vfprintf(stderr, fmt, ap);
    va_end(ap);
    fprintf(stderr, "\n");
    g_warnings++;
}

/* ── File utilities ───────────────────────────────────────────────────── */

static int file_exists(const char *path) {
    struct stat st;
    return stat(path, &st) == 0;
}

static int is_directory(const char *path) {
    struct stat st;
    if (stat(path, &st) != 0) return 0;
    return S_ISDIR(st.st_mode);
}

static size_t file_size(const char *path) {
    struct stat st;
    if (stat(path, &st) != 0) return 0;
    return (size_t)st.st_size;
}

static void make_directory(const char *path) {
    struct stat st;
    if (stat(path, &st) == 0 && S_ISDIR(st.st_mode))
        return;
#ifdef _WIN32
    _mkdir(path);
#else
    mkdir(path, 0755);
#endif
}

static void make_directories(const char *path) {
    char tmp[MAX_PATH_LEN];
    size_t len = strlen(path);
    if (len >= MAX_PATH_LEN) fatal("path too long: %s", path);
    memcpy(tmp, path, len + 1);

    for (size_t i = 1; i < len; i++) {
        if (tmp[i] == '/') {
            tmp[i] = '\0';
            make_directory(tmp);
            tmp[i] = '/';
        }
    }
    make_directory(tmp);
}

static int copy_file(const char *src, const char *dst) {
    FILE *in = fopen(src, "rb");
    if (!in) {
        fprintf(stderr, "mkappbundle: cannot open '%s': %s\n", src, strerror(errno));
        return -1;
    }

    FILE *out = fopen(dst, "wb");
    if (!out) {
        fclose(in);
        fprintf(stderr, "mkappbundle: cannot create '%s': %s\n", dst, strerror(errno));
        return -1;
    }

    char buf[8192];
    size_t n;
    while ((n = fread(buf, 1, sizeof(buf), in)) > 0) {
        if (fwrite(buf, 1, n, out) != n) {
            fclose(in); fclose(out);
            fprintf(stderr, "mkappbundle: write error on '%s'\n", dst);
            return -1;
        }
    }

    fclose(in);
    fclose(out);
    return 0;
}

static const char *basename_of(const char *path) {
    const char *p = strrchr(path, '/');
    return p ? p + 1 : path;
}

#ifndef _WIN32
static int copy_directory(const char *src_dir, const char *dst_dir) {
    make_directories(dst_dir);

    DIR *d = opendir(src_dir);
    if (!d) {
        fprintf(stderr, "mkappbundle: cannot open directory '%s': %s\n",
                src_dir, strerror(errno));
        return -1;
    }

    struct dirent *ent;
    while ((ent = readdir(d)) != NULL) {
        if (ent->d_name[0] == '.' &&
            (ent->d_name[1] == '\0' ||
             (ent->d_name[1] == '.' && ent->d_name[2] == '\0')))
            continue;

        char src_path[MAX_PATH_LEN];
        char dst_path[MAX_PATH_LEN];
        snprintf(src_path, sizeof(src_path), "%s/%s", src_dir, ent->d_name);
        snprintf(dst_path, sizeof(dst_path), "%s/%s", dst_dir, ent->d_name);

        if (is_directory(src_path)) {
            if (copy_directory(src_path, dst_path) != 0) {
                closedir(d);
                return -1;
            }
        } else {
            if (copy_file(src_path, dst_path) != 0) {
                closedir(d);
                return -1;
            }
        }
    }

    closedir(d);
    return 0;
}
#endif

/* ── Validation: Info.conf ────────────────────────────────────────────── */

/* Known valid capability names (must match kernel/src/task/capabilities.rs) */
static const char *VALID_CAPS[] = {
    "all", "filesystem", "network", "audio", "display", "device",
    "process", "pipe", "shm", "event", "compositor", "system",
    "dll", "thread", "manage_perms", NULL
};

static int is_valid_capability(const char *cap) {
    for (int i = 0; VALID_CAPS[i]; i++) {
        if (strcmp(cap, VALID_CAPS[i]) == 0) return 1;
    }
    return 0;
}

/* Known valid category names */
static const char *VALID_CATEGORIES[] = {
    "System", "Utilities", "Games", "Development",
    "Graphics", "Multimedia", "Network", "Internet",
    "Productivity", "Media", "Other", NULL
};

static int is_valid_category(const char *cat) {
    for (int i = 0; VALID_CATEGORIES[i]; i++) {
        if (strcmp(cat, VALID_CATEGORIES[i]) == 0) return 1;
    }
    return 0;
}

typedef struct {
    char id[MAX_NAME_LEN];
    char name[MAX_NAME_LEN];
    char exec[MAX_NAME_LEN];
    char version[64];
    char category[64];
    char capabilities[MAX_NAME_LEN];
    char working_dir[64];
} InfoConf;

/*
 * Parse and validate Info.conf.
 * Returns 0 on success, -1 on fatal error.
 * Warnings are printed but don't cause failure.
 */
static int validate_info_conf(const char *path, InfoConf *info) {
    FILE *fp = fopen(path, "r");
    if (!fp) fatal("cannot open Info.conf: %s", strerror(errno));

    memset(info, 0, sizeof(*info));

    char line[MAX_LINE_LEN];
    int lineno = 0;
    int errors = 0;

    while (fgets(line, sizeof(line), fp)) {
        lineno++;

        /* Strip trailing whitespace */
        size_t len = strlen(line);
        while (len > 0 && (line[len-1] == '\n' || line[len-1] == '\r' ||
                           line[len-1] == ' '  || line[len-1] == '\t'))
            line[--len] = '\0';

        if (len == 0) continue;

        char *eq = strchr(line, '=');
        if (!eq) {
            warn("Info.conf:%d: malformed line (no '='): %s", lineno, line);
            errors++;
            continue;
        }

        *eq = '\0';
        const char *key = line;
        const char *val = eq + 1;

        if (strlen(val) == 0) {
            warn("Info.conf:%d: empty value for key '%s'", lineno, key);
            errors++;
            continue;
        }

        if (strcmp(key, "id") == 0) {
            strncpy(info->id, val, sizeof(info->id) - 1);
            /* Validate reverse-DNS format (at least one dot) */
            if (!strchr(val, '.'))
                warn("Info.conf:%d: 'id' should be reverse-DNS (e.g. com.anyos.myapp)", lineno);
        } else if (strcmp(key, "name") == 0) {
            strncpy(info->name, val, sizeof(info->name) - 1);
        } else if (strcmp(key, "exec") == 0) {
            strncpy(info->exec, val, sizeof(info->exec) - 1);
        } else if (strcmp(key, "version") == 0) {
            strncpy(info->version, val, sizeof(info->version) - 1);
        } else if (strcmp(key, "category") == 0) {
            strncpy(info->category, val, sizeof(info->category) - 1);
            if (!is_valid_category(val))
                warn("Info.conf:%d: unknown category '%s' (expected: System, Utilities, Games, Development, Graphics, Multimedia, Network, Other)", lineno, val);
        } else if (strcmp(key, "capabilities") == 0) {
            strncpy(info->capabilities, val, sizeof(info->capabilities) - 1);
            /* Validate each comma-separated capability */
            char tmp[MAX_NAME_LEN];
            strncpy(tmp, val, sizeof(tmp) - 1);
            tmp[sizeof(tmp) - 1] = '\0';
            char *tok = strtok(tmp, ",");
            while (tok) {
                /* Skip leading whitespace */
                while (*tok == ' ') tok++;
                if (!is_valid_capability(tok))
                    warn("Info.conf:%d: unknown capability '%s'", lineno, tok);
                tok = strtok(NULL, ",");
            }
        } else if (strcmp(key, "working_dir") == 0) {
            strncpy(info->working_dir, val, sizeof(info->working_dir) - 1);
            if (strcmp(val, "bundle") != 0)
                warn("Info.conf:%d: unknown working_dir '%s' (expected: 'bundle')", lineno, val);
        } else {
            warn("Info.conf:%d: unknown key '%s'", lineno, key);
        }
    }

    fclose(fp);

    /* Check required fields */
    if (info->id[0] == '\0') {
        fprintf(stderr, "mkappbundle: error: Info.conf missing required key 'id'\n");
        errors++;
    }
    if (info->name[0] == '\0') {
        fprintf(stderr, "mkappbundle: error: Info.conf missing required key 'name'\n");
        errors++;
    }
    if (info->exec[0] == '\0') {
        fprintf(stderr, "mkappbundle: error: Info.conf missing required key 'exec'\n");
        errors++;
    }
    if (info->version[0] == '\0') {
        fprintf(stderr, "mkappbundle: error: Info.conf missing required key 'version'\n");
        errors++;
    }
    if (info->category[0] == '\0') {
        fprintf(stderr, "mkappbundle: error: Info.conf missing required key 'category'\n");
        errors++;
    }

    /* Warn if no capabilities are specified */
    if (info->capabilities[0] == '\0') {
        fprintf(stderr,
            "mkappbundle: notice: Info.conf has no 'capabilities' key.\n"
            "  The app will launch with zero permissions and will not prompt\n"
            "  the user for access. Add capabilities=... to grant permissions.\n"
            "  Available: filesystem, network, audio, display, device, process,\n"
            "             pipe, shm, event, compositor, system, dll, thread,\n"
            "             manage_perms, all\n");
    }

    return errors > 0 ? -1 : 0;
}

/* ── Validation: Binary executable ────────────────────────────────────── */

/*
 * Check if the binary at `path` is an ELF file (starts with \x7fELF).
 * Returns 1 if ELF, 0 otherwise.
 */
static int is_elf_file(const char *path) {
    FILE *fp = fopen(path, "rb");
    if (!fp) return 0;

    uint8_t magic[4];
    size_t n = fread(magic, 1, 4, fp);
    fclose(fp);

    return (n >= 4 && magic[0] == 0x7F && magic[1] == 'E' &&
            magic[2] == 'L' && magic[3] == 'F');
}

/*
 * Try to auto-convert an ELF binary to flat binary using anyelf.
 * Uses g_anyelf_path if set, otherwise searches PATH.
 * Writes converted file to `out_path`. Returns 0 on success, -1 on failure.
 */
static int try_anyelf_convert(const char *elf_path, const char *out_path, int verbose) {
    char cmd[MAX_PATH_LEN * 4];
    const char *anyelf = g_anyelf_path ? g_anyelf_path : "anyelf";
#ifdef _WIN32
    snprintf(cmd, sizeof(cmd), "\"%s\" bin \"%s\" \"%s\" > NUL 2>&1",
             anyelf, elf_path, out_path);
#else
    snprintf(cmd, sizeof(cmd), "\"%s\" bin \"%s\" \"%s\" > /dev/null 2>&1",
             anyelf, elf_path, out_path);
#endif

    int ret = system(cmd);
    if (ret == 0) {
        if (verbose)
            printf("  Auto-converted ELF '%s' -> flat binary '%s'\n",
                   elf_path, out_path);
        return 0;
    }
    return -1;
}

/*
 * Validate the binary executable.
 * If it's an ELF file and anyelf is available, auto-converts it.
 * `converted_path` (size MAX_PATH_LEN) receives the converted flat binary
 * path if conversion occurred, or empty string if the input was already flat.
 */
static int validate_binary(const char *path, char *converted_path, int verbose) {
    converted_path[0] = '\0';

    size_t sz = file_size(path);
    if (sz == 0) {
        fprintf(stderr, "mkappbundle: error: binary '%s' is empty\n", path);
        return -1;
    }

    if (is_elf_file(path)) {
        if (g_keep_elf) {
            /* ELF allowed as-is (e.g. for C programs using kernel ELF loader) */
            if (verbose)
                printf("  Binary is ELF — keeping as-is (--keep-elf)\n");
            return 0;
        }

        /* Try auto-conversion with anyelf */
        snprintf(converted_path, MAX_PATH_LEN, "%s.flat.tmp", path);

        if (try_anyelf_convert(path, converted_path, verbose) == 0) {
            printf("mkappbundle: auto-converted ELF to flat binary using anyelf\n");
            return 0;
        }

        /* anyelf not found or failed — show manual instructions */
        converted_path[0] = '\0';
        fprintf(stderr,
            "mkappbundle: error: binary '%s' is an ELF file\n"
            "  .app bundles require flat binaries. Convert with:\n"
            "    anyelf bin %s output.bin\n"
            "  Or ensure 'anyelf' is in your PATH for auto-conversion.\n"
            "  Use --keep-elf to bundle ELF binaries without conversion.\n",
            path, path);
        return -1;
    }

    if (sz < 16) {
        warn("binary '%s' is suspiciously small (%zu bytes)", path, sz);
    }

    return 0;
}

/* ── Validation: ICO icon ─────────────────────────────────────────────── */

static int validate_icon(const char *path) {
    size_t sz = file_size(path);
    if (sz < 6) {
        fprintf(stderr, "mkappbundle: error: icon '%s' is too small (%zu bytes)\n",
                path, sz);
        return -1;
    }

    FILE *fp = fopen(path, "rb");
    if (!fp) fatal("cannot open icon: %s", strerror(errno));

    uint8_t hdr[6];
    size_t n = fread(hdr, 1, 6, fp);
    fclose(fp);

    if (n < 6) {
        fprintf(stderr, "mkappbundle: error: cannot read icon header\n");
        return -1;
    }

    /* ICO format: bytes 0-1 = 0x0000 (reserved), bytes 2-3 = 0x0001 (type=icon) */
    uint16_t reserved = hdr[0] | (hdr[1] << 8);
    uint16_t type     = hdr[2] | (hdr[3] << 8);
    uint16_t count    = hdr[4] | (hdr[5] << 8);

    if (reserved != 0) {
        fprintf(stderr, "mkappbundle: error: icon '%s' has invalid header "
                "(reserved=%u, expected 0)\n", path, reserved);
        return -1;
    }

    if (type != 1) {
        fprintf(stderr, "mkappbundle: error: icon '%s' is not an ICO file "
                "(type=%u, expected 1)\n", path, type);
        return -1;
    }

    if (count == 0) {
        fprintf(stderr, "mkappbundle: error: icon '%s' contains 0 images\n", path);
        return -1;
    }

    return 0;
}

/* ── Usage ────────────────────────────────────────────────────────────── */

static void usage(void) {
    fprintf(stderr,
        "mkappbundle — anyOS .app bundle creator\n"
        "\n"
        "Usage:\n"
        "  mkappbundle -i <Info.conf> -e <binary> [options] -o <Output.app>\n"
        "\n"
        "Required:\n"
        "  -i <path>    Info.conf metadata file\n"
        "  -e <path>    Executable (flat binary or ELF — auto-converts)\n"
        "  -o <path>    Output .app directory\n"
        "\n"
        "Optional:\n"
        "  -c <path>           Icon file (validated as ICO format)\n"
        "  -r <path>           Resource file or directory (repeatable, max %d)\n"
        "  --anyelf-path <p>   Path to anyelf binary (for ELF auto-conversion)\n"
        "  --keep-elf          Bundle ELF binaries as-is (no conversion)\n"
        "  -v                  Verbose output\n"
        "  --force             Continue despite warnings\n"
        "\n"
        "Validation:\n"
        "  - Info.conf: required keys (id, name, exec, version, category)\n"
        "  - Info.conf: valid capability names, valid category, reverse-DNS id\n"
        "  - Binary:    must not be empty; ELF auto-converted if anyelf in PATH\n"
        "  - Icon:      must be valid Windows ICO format\n"
        "\n"
        "Examples:\n"
        "  mkappbundle -i Info.conf -e Terminal -o Terminal.app\n"
        "  mkappbundle -i Info.conf -e DOOM -c Icon.ico -r doom.wad -o DOOM.app\n",
        MAX_RESOURCES
    );
    exit(1);
}

/* ── Main ─────────────────────────────────────────────────────────────── */

int main(int argc, char **argv) {
    const char *info_path   = NULL;
    const char *exec_path   = NULL;
    const char *icon_path   = NULL;
    const char *output_path = NULL;
    const char *resources[MAX_RESOURCES];
    int         num_resources = 0;
    int         verbose       = 0;

    if (argc < 2) usage();

    /* Parse arguments */
    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], "-i") == 0 && i + 1 < argc) {
            info_path = argv[++i];
        } else if (strcmp(argv[i], "-e") == 0 && i + 1 < argc) {
            exec_path = argv[++i];
        } else if (strcmp(argv[i], "-c") == 0 && i + 1 < argc) {
            icon_path = argv[++i];
        } else if (strcmp(argv[i], "-o") == 0 && i + 1 < argc) {
            output_path = argv[++i];
        } else if (strcmp(argv[i], "-r") == 0 && i + 1 < argc) {
            if (num_resources >= MAX_RESOURCES)
                fatal("too many resources (max %d)", MAX_RESOURCES);
            resources[num_resources++] = argv[++i];
        } else if (strcmp(argv[i], "-v") == 0) {
            verbose = 1;
        } else if (strcmp(argv[i], "--force") == 0) {
            g_force = 1;
        } else if (strcmp(argv[i], "--keep-elf") == 0) {
            g_keep_elf = 1;
        } else if (strcmp(argv[i], "--anyelf-path") == 0 && i + 1 < argc) {
            g_anyelf_path = argv[++i];
        } else if (strcmp(argv[i], "-h") == 0 || strcmp(argv[i], "--help") == 0) {
            usage();
        } else {
            fprintf(stderr, "mkappbundle: unknown option '%s'\n\n", argv[i]);
            usage();
        }
    }

    /* Check required arguments */
    if (!info_path)   fatal("missing -i <Info.conf>");
    if (!exec_path)   fatal("missing -e <binary>");
    if (!output_path) fatal("missing -o <Output.app>");

    /* ── Phase 1: Validate all inputs ─────────────────────────────────── */

    if (verbose) printf("Validating inputs...\n");

    /* Check existence first */
    if (!file_exists(info_path))
        fatal("Info.conf not found: %s", info_path);
    if (!file_exists(exec_path))
        fatal("executable not found: %s", exec_path);
    if (icon_path && !file_exists(icon_path))
        fatal("icon not found: %s", icon_path);
    for (int i = 0; i < num_resources; i++) {
        if (!file_exists(resources[i]))
            fatal("resource not found: %s", resources[i]);
    }

    /* Validate Info.conf */
    InfoConf info;
    if (validate_info_conf(info_path, &info) != 0)
        fatal("Info.conf validation failed");

    /* Validate binary (may auto-convert ELF → flat via anyelf) */
    char converted_path[MAX_PATH_LEN];
    if (validate_binary(exec_path, converted_path, verbose) != 0)
        fatal("binary validation failed");

    /* Use converted path if auto-conversion happened */
    const char *actual_exec_path = converted_path[0] ? converted_path : exec_path;

    /* Validate icon */
    if (icon_path) {
        if (validate_icon(icon_path) != 0)
            fatal("icon validation failed");
    }

    /* Check for warnings */
    if (g_warnings > 0 && !g_force) {
        fprintf(stderr, "mkappbundle: %d warning(s). Use --force to continue anyway.\n",
                g_warnings);
        exit(1);
    }

    /* Verify exec name consistency: the binary should match what Info.conf expects */
    if (verbose) {
        printf("  Info.conf: id=%s, name=%s, exec=%s\n",
               info.id, info.name, info.exec);
        printf("  Binary:    %s (%zu bytes)%s\n", actual_exec_path,
               file_size(actual_exec_path),
               converted_path[0] ? " (auto-converted from ELF)" : "");
        if (icon_path)
            printf("  Icon:      %s (%zu bytes)\n", icon_path, file_size(icon_path));
        printf("  Resources: %d\n", num_resources);
    }

    /* ── Phase 2: Create the bundle ───────────────────────────────────── */

    if (verbose) printf("Creating bundle: %s\n", output_path);

    make_directories(output_path);

    /* Copy Info.conf */
    {
        char dst[MAX_PATH_LEN];
        snprintf(dst, sizeof(dst), "%s/Info.conf", output_path);
        if (copy_file(info_path, dst) != 0)
            fatal("failed to copy Info.conf");
        if (verbose) printf("  + Info.conf\n");
    }

    /* Copy executable (named as specified by Info.conf's exec field) */
    {
        char dst[MAX_PATH_LEN];
        snprintf(dst, sizeof(dst), "%s/%s", output_path, info.exec);
        if (copy_file(actual_exec_path, dst) != 0)
            fatal("failed to copy executable");
#ifndef _WIN32
        chmod(dst, 0755);
#endif
        if (verbose) printf("  + %s (executable, %zu bytes)\n",
                            info.exec, file_size(actual_exec_path));
    }

    /* Clean up temporary converted file if any */
    if (converted_path[0]) {
        unlink(converted_path);
    }

    /* Copy icon */
    if (icon_path) {
        char dst[MAX_PATH_LEN];
        snprintf(dst, sizeof(dst), "%s/Icon.ico", output_path);
        if (copy_file(icon_path, dst) != 0)
            fatal("failed to copy icon");
        if (verbose) printf("  + Icon.ico (%zu bytes)\n", file_size(icon_path));
    }

    /* Copy resources */
    for (int i = 0; i < num_resources; i++) {
        const char *res = resources[i];

        if (is_directory(res)) {
            /* Strip trailing slashes for proper basename */
            char clean[MAX_PATH_LEN];
            strncpy(clean, res, sizeof(clean) - 1);
            clean[sizeof(clean) - 1] = '\0';
            size_t clen = strlen(clean);
            while (clen > 1 && clean[clen - 1] == '/')
                clean[--clen] = '\0';
            const char *dirname = basename_of(clean);

            char dst_dir[MAX_PATH_LEN];
            snprintf(dst_dir, sizeof(dst_dir), "%s/%s", output_path, dirname);

#ifndef _WIN32
            if (copy_directory(clean, dst_dir) != 0)
                fatal("failed to copy resource directory: %s", res);
#else
            fatal("directory copy not supported on this platform");
#endif
            if (verbose) printf("  + %s/ (directory)\n", dirname);
        } else {
            const char *fname = basename_of(res);
            char dst[MAX_PATH_LEN];
            snprintf(dst, sizeof(dst), "%s/%s", output_path, fname);
            if (copy_file(res, dst) != 0)
                fatal("failed to copy resource: %s", res);
            if (verbose) printf("  + %s (%zu bytes)\n", fname, file_size(res));
        }
    }

    /* ── Done ─────────────────────────────────────────────────────────── */

    printf("mkappbundle: created %s/ (%s v%s)\n",
           output_path, info.name, info.version);
    return 0;
}
