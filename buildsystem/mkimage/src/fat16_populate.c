/*
 * fat16_populate.c - recursive host-directory importer for FAT16.
 */

#include "mkimage.h"

#include <dirent.h>
#include <sys/stat.h>

static int should_skip(const char *name)
{
    return strcmp(name, ".DS_Store") == 0 ||
           strcmp(name, ".git") == 0 ||
           strcmp(name, ".gitignore") == 0 ||
           strcmp(name, ".gitkeep") == 0;
}

static void fat16_populate_dir(Fat16 *fs, const char *host_path,
                               uint32_t parent_cluster, int is_root)
{
    DIR *d;
    struct dirent *ent;
    char **names = NULL;
    int name_count = 0;
    int name_cap = 0;
    int i;

    d = opendir(host_path);
    if (!d) {
        fprintf(stderr, "  WARNING: Cannot open directory %s\n", host_path);
        return;
    }

    while ((ent = readdir(d)) != NULL) {
        const char *n = ent->d_name;
        if (n[0] == '.' && (n[1] == '\0' || (n[1] == '.' && n[2] == '\0')))
            continue;
        if (should_skip(n))
            continue;

        if (name_count >= name_cap) {
            int new_cap = name_cap == 0 ? 64 : name_cap * 2;
            char **tmp = (char **)realloc(names,
                                          (size_t)new_cap * sizeof(char *));
            if (!tmp)
                fatal("fat16_populate_dir: realloc failed");
            names = tmp;
            name_cap = new_cap;
        }
        names[name_count++] = strdup(n);
    }
    closedir(d);

    for (i = 0; i < name_count - 1; ++i) {
        int j;
        for (j = i + 1; j < name_count; ++j) {
            if (strcmp(names[i], names[j]) > 0) {
                char *tmp = names[i];
                names[i] = names[j];
                names[j] = tmp;
            }
        }
    }

    for (i = 0; i < name_count; ++i) {
        const char *entry_name = names[i];
        char full_path[4096];
        struct stat st;

        snprintf(full_path, sizeof(full_path), "%s/%s", host_path, entry_name);
        if (stat(full_path, &st) != 0)
            goto next;

        if (S_ISDIR(st.st_mode)) {
            uint32_t dir_cluster = fat16_create_dir(fs, parent_cluster,
                                                    entry_name, is_root);
            printf("    Dir:  %s/ (cluster=%u)\n", entry_name, dir_cluster);
            fat16_populate_dir(fs, full_path, dir_cluster, 0);
        } else if (S_ISREG(st.st_mode)) {
            size_t file_size;
            uint8_t *file_data = read_file(full_path, &file_size);
            if (file_data) {
                fat16_add_file(fs, parent_cluster, entry_name,
                               file_data, file_size, is_root);
                free(file_data);
            }
        }

next:
        free(names[i]);
    }

    free(names);
}

void fat16_populate_sysroot(Fat16 *fs, const char *sysroot_path)
{
    struct stat st;

    if (stat(sysroot_path, &st) != 0 || !S_ISDIR(st.st_mode)) {
        printf("  Warning: sysroot path '%s' does not exist, skipping\n",
               sysroot_path);
        return;
    }

    fat16_add_volume_label(fs, "ANYOS");
    fat16_populate_dir(fs, sysroot_path, 0, 1);
}
