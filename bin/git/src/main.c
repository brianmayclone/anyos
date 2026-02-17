/*
 * Mini git CLI for anyOS — wraps libgit2
 * Commands: init, add, status, commit, log, diff, clone, remote, fetch, pull, push
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <git2.h>

/* BearSSL TLS stream registration (defined in bearssl_stream.c) */
extern void bearssl_stream_register(void);

static void die(const char *msg, int err) {
    const git_error *e = git_error_last();
    if (e)
        fprintf(stderr, "fatal: %s: %s\n", msg, e->message);
    else
        fprintf(stderr, "fatal: %s (error %d)\n", msg, err);
    exit(1);
}

/* ---- git init ---- */
static int cmd_init(int argc, char **argv) {
    const char *path = (argc > 0) ? argv[0] : ".";
    git_repository *repo = NULL;
    int err = git_repository_init(&repo, path, 0);
    if (err < 0) die("git_repository_init", err);
    printf("Initialized empty Git repository in %s\n", git_repository_path(repo));
    git_repository_free(repo);
    return 0;
}

/* ---- git add ---- */
static int cmd_add(int argc, char **argv) {
    if (argc < 1) {
        fprintf(stderr, "usage: git add <file>...\n");
        return 1;
    }
    git_repository *repo = NULL;
    int err = git_repository_open(&repo, ".");
    if (err < 0) die("Cannot open repository", err);

    git_index *index = NULL;
    err = git_repository_index(&index, repo);
    if (err < 0) die("Cannot get index", err);

    for (int i = 0; i < argc; i++) {
        if (strcmp(argv[i], ".") == 0 || strcmp(argv[i], "-A") == 0) {
            git_strarray pathspec = { NULL, 0 };
            char *star = "*";
            pathspec.strings = &star;
            pathspec.count = 1;
            err = git_index_add_all(index, &pathspec, 0, NULL, NULL);
            if (err < 0) die("git_index_add_all", err);
        } else {
            err = git_index_add_bypath(index, argv[i]);
            if (err < 0) {
                fprintf(stderr, "warning: could not add '%s'\n", argv[i]);
            }
        }
    }

    err = git_index_write(index);
    if (err < 0) die("git_index_write", err);

    git_index_free(index);
    git_repository_free(repo);
    return 0;
}

/* ---- git status ---- */
static int status_cb(const char *path, unsigned int flags, void *payload) {
    (void)payload;
    const char *istatus = NULL;
    const char *wstatus = NULL;

    if (flags & GIT_STATUS_INDEX_NEW)        istatus = "new file";
    if (flags & GIT_STATUS_INDEX_MODIFIED)   istatus = "modified";
    if (flags & GIT_STATUS_INDEX_DELETED)    istatus = "deleted";
    if (flags & GIT_STATUS_INDEX_RENAMED)    istatus = "renamed";

    if (flags & GIT_STATUS_WT_NEW)           wstatus = "untracked";
    if (flags & GIT_STATUS_WT_MODIFIED)      wstatus = "modified";
    if (flags & GIT_STATUS_WT_DELETED)       wstatus = "deleted";

    if (istatus)
        printf("  staged:   %-12s %s\n", istatus, path);
    if (wstatus)
        printf("  working:  %-12s %s\n", wstatus, path);

    return 0;
}

static int cmd_status(int argc, char **argv) {
    (void)argc; (void)argv;
    git_repository *repo = NULL;
    int err = git_repository_open(&repo, ".");
    if (err < 0) die("Cannot open repository", err);

    git_reference *head = NULL;
    err = git_repository_head(&head, repo);
    if (err == GIT_EUNBORNBRANCH) {
        printf("On branch master (no commits yet)\n\n");
    } else if (err == 0) {
        const char *name = git_reference_shorthand(head);
        printf("On branch %s\n\n", name);
        git_reference_free(head);
    }

    printf("Changes:\n");
    err = git_status_foreach(repo, status_cb, NULL);
    if (err < 0) die("git_status_foreach", err);

    git_repository_free(repo);
    return 0;
}

/* ---- git commit ---- */
static int cmd_commit(int argc, char **argv) {
    const char *message = NULL;

    for (int i = 0; i < argc; i++) {
        if (strcmp(argv[i], "-m") == 0 && i + 1 < argc) {
            message = argv[i + 1];
            break;
        }
    }
    if (!message) {
        fprintf(stderr, "usage: git commit -m \"message\"\n");
        return 1;
    }

    git_repository *repo = NULL;
    int err = git_repository_open(&repo, ".");
    if (err < 0) die("Cannot open repository", err);

    git_index *index = NULL;
    err = git_repository_index(&index, repo);
    if (err < 0) die("Cannot get index", err);

    git_oid tree_oid;
    err = git_index_write_tree(&tree_oid, index);
    if (err < 0) die("git_index_write_tree", err);

    git_tree *tree = NULL;
    err = git_tree_lookup(&tree, repo, &tree_oid);
    if (err < 0) die("git_tree_lookup", err);

    git_signature *sig = NULL;
    err = git_signature_default(&sig, repo);
    if (err < 0) {
        err = git_signature_now(&sig, "anyOS User", "user@anyos");
        if (err < 0) die("git_signature_now", err);
    }

    git_commit *parent = NULL;
    git_reference *head_ref = NULL;
    int has_parent = 0;

    err = git_repository_head(&head_ref, repo);
    if (err == 0) {
        const git_oid *head_oid = git_reference_target(head_ref);
        err = git_commit_lookup(&parent, repo, head_oid);
        if (err < 0) die("git_commit_lookup", err);
        has_parent = 1;
        git_reference_free(head_ref);
    }

    git_oid commit_oid;
    const git_commit *parents[] = { parent };
    err = git_commit_create(
        &commit_oid, repo, "HEAD",
        sig, sig,
        NULL, message,
        tree,
        has_parent ? 1 : 0,
        has_parent ? parents : NULL
    );
    if (err < 0) die("git_commit_create", err);

    char oid_str[GIT_OID_SHA1_HEXSIZE + 1];
    git_oid_tostr(oid_str, sizeof(oid_str), &commit_oid);
    printf("[%.7s] %s\n", oid_str, message);

    if (parent) git_commit_free(parent);
    git_signature_free(sig);
    git_tree_free(tree);
    git_index_free(index);
    git_repository_free(repo);
    return 0;
}

/* ---- git log ---- */
static int cmd_log(int argc, char **argv) {
    git_repository *repo = NULL;
    int err = git_repository_open(&repo, ".");
    if (err < 0) die("Cannot open repository", err);

    git_revwalk *walk = NULL;
    err = git_revwalk_new(&walk, repo);
    if (err < 0) die("git_revwalk_new", err);

    git_revwalk_sorting(walk, GIT_SORT_TIME);
    err = git_revwalk_push_head(walk);
    if (err < 0) die("git_revwalk_push_head", err);

    int count = 0;
    int max_count = 20;

    for (int i = 0; i < argc; i++) {
        if (strcmp(argv[i], "-n") == 0 && i + 1 < argc) {
            max_count = atoi(argv[i + 1]);
            break;
        }
    }

    git_oid oid;
    while (git_revwalk_next(&oid, walk) == 0 && count < max_count) {
        git_commit *commit = NULL;
        err = git_commit_lookup(&commit, repo, &oid);
        if (err < 0) continue;

        char oid_str[GIT_OID_SHA1_HEXSIZE + 1];
        git_oid_tostr(oid_str, sizeof(oid_str), &oid);

        const git_signature *author = git_commit_author(commit);
        const char *msg = git_commit_message(commit);

        printf("commit %s\n", oid_str);
        printf("Author: %s <%s>\n", author->name, author->email);
        printf("\n    %s\n\n", msg);

        git_commit_free(commit);
        count++;
    }

    git_revwalk_free(walk);
    git_repository_free(repo);
    return 0;
}

/* ---- git diff ---- */
static int diff_line_cb(const git_diff_delta *delta,
                        const git_diff_hunk *hunk,
                        const git_diff_line *line,
                        void *payload) {
    (void)delta; (void)hunk; (void)payload;

    if (line->origin == GIT_DIFF_LINE_ADDITION)
        putchar('+');
    else if (line->origin == GIT_DIFF_LINE_DELETION)
        putchar('-');
    else if (line->origin == GIT_DIFF_LINE_CONTEXT)
        putchar(' ');

    fwrite(line->content, 1, line->content_len, stdout);
    return 0;
}

static int cmd_diff(int argc, char **argv) {
    (void)argc; (void)argv;
    git_repository *repo = NULL;
    int err = git_repository_open(&repo, ".");
    if (err < 0) die("Cannot open repository", err);

    git_diff *diff = NULL;
    git_diff_options opts = GIT_DIFF_OPTIONS_INIT;

    err = git_diff_index_to_workdir(&diff, repo, NULL, &opts);
    if (err < 0) die("git_diff_index_to_workdir", err);

    err = git_diff_print(diff, GIT_DIFF_FORMAT_PATCH, diff_line_cb, NULL);
    if (err < 0) die("git_diff_print", err);

    git_diff_free(diff);
    git_repository_free(repo);
    return 0;
}

/* ---- git clone ---- */
static int cmd_clone(int argc, char **argv) {
    if (argc < 1) {
        fprintf(stderr, "usage: git clone <url> [<directory>]\n");
        return 1;
    }

    const char *url = argv[0];
    const char *path = (argc > 1) ? argv[1] : NULL;

    /* Auto-detect path from URL if not given */
    char pathbuf[256];
    if (!path) {
        const char *slash = strrchr(url, '/');
        if (slash) slash++;
        else slash = url;
        strncpy(pathbuf, slash, sizeof(pathbuf) - 1);
        pathbuf[sizeof(pathbuf) - 1] = '\0';
        /* Strip .git suffix */
        char *dot = strstr(pathbuf, ".git");
        if (dot && dot[4] == '\0') *dot = '\0';
        path = pathbuf;
    }

    printf("Cloning into '%s'...\n", path);

    git_clone_options opts = GIT_CLONE_OPTIONS_INIT;
    git_repository *repo = NULL;
    int err = git_clone(&repo, url, path, &opts);
    if (err < 0) die("git_clone", err);

    printf("done.\n");
    git_repository_free(repo);
    return 0;
}

/* ---- git remote ---- */
static int cmd_remote(int argc, char **argv) {
    git_repository *repo = NULL;
    int err = git_repository_open(&repo, ".");
    if (err < 0) die("Cannot open repository", err);

    if (argc == 0 || (argc == 1 && strcmp(argv[0], "-v") == 0)) {
        /* List remotes */
        int verbose = (argc == 1 && strcmp(argv[0], "-v") == 0);
        git_strarray remotes = {0};
        err = git_remote_list(&remotes, repo);
        if (err < 0) die("git_remote_list", err);

        for (size_t i = 0; i < remotes.count; i++) {
            if (verbose) {
                git_remote *remote = NULL;
                git_remote_lookup(&remote, repo, remotes.strings[i]);
                if (remote) {
                    const char *url = git_remote_url(remote);
                    printf("%s\t%s (fetch)\n", remotes.strings[i], url ? url : "");
                    const char *pushurl = git_remote_pushurl(remote);
                    printf("%s\t%s (push)\n", remotes.strings[i], pushurl ? pushurl : (url ? url : ""));
                    git_remote_free(remote);
                }
            } else {
                printf("%s\n", remotes.strings[i]);
            }
        }
        git_strarray_dispose(&remotes);
    } else if (argc >= 3 && strcmp(argv[0], "add") == 0) {
        /* Add remote */
        git_remote *remote = NULL;
        err = git_remote_create(&remote, repo, argv[1], argv[2]);
        if (err < 0) die("git_remote_create", err);
        git_remote_free(remote);
    } else if (argc >= 2 && strcmp(argv[0], "remove") == 0) {
        err = git_remote_delete(repo, argv[1]);
        if (err < 0) die("git_remote_delete", err);
    } else if (argc >= 3 && strcmp(argv[0], "set-url") == 0) {
        err = git_remote_set_url(repo, argv[1], argv[2]);
        if (err < 0) die("git_remote_set_url", err);
    } else {
        fprintf(stderr, "usage: git remote [-v]\n");
        fprintf(stderr, "       git remote add <name> <url>\n");
        fprintf(stderr, "       git remote remove <name>\n");
        fprintf(stderr, "       git remote set-url <name> <url>\n");
        git_repository_free(repo);
        return 1;
    }

    git_repository_free(repo);
    return 0;
}

/* ---- Transfer progress callback ---- */
static int fetch_progress(const git_indexer_progress *stats, void *payload) {
    (void)payload;
    if (stats->received_objects > 0) {
        printf("\rReceiving objects: %3d%% (%u/%u)",
            (int)(100 * stats->received_objects / stats->total_objects),
            stats->received_objects, stats->total_objects);
        if (stats->received_objects == stats->total_objects)
            printf(", done.\n");
    }
    return 0;
}

/* ---- git fetch ---- */
static int cmd_fetch(int argc, char **argv) {
    const char *remote_name = (argc > 0) ? argv[0] : "origin";

    git_repository *repo = NULL;
    int err = git_repository_open(&repo, ".");
    if (err < 0) die("Cannot open repository", err);

    git_remote *remote = NULL;
    err = git_remote_lookup(&remote, repo, remote_name);
    if (err < 0) die("Remote not found", err);

    git_fetch_options opts = GIT_FETCH_OPTIONS_INIT;
    opts.callbacks.transfer_progress = fetch_progress;

    printf("Fetching %s...\n", remote_name);
    err = git_remote_fetch(remote, NULL, &opts, "fetch");
    if (err < 0) die("git_remote_fetch", err);

    printf("From %s\n", git_remote_url(remote));

    git_remote_free(remote);
    git_repository_free(repo);
    return 0;
}

/* ---- git pull (fetch + merge) ---- */
static int cmd_pull(int argc, char **argv) {
    const char *remote_name = (argc > 0) ? argv[0] : "origin";

    git_repository *repo = NULL;
    int err = git_repository_open(&repo, ".");
    if (err < 0) die("Cannot open repository", err);

    /* Fetch */
    git_remote *remote = NULL;
    err = git_remote_lookup(&remote, repo, remote_name);
    if (err < 0) die("Remote not found", err);

    git_fetch_options fetch_opts = GIT_FETCH_OPTIONS_INIT;
    fetch_opts.callbacks.transfer_progress = fetch_progress;

    printf("Pulling from %s...\n", remote_name);
    err = git_remote_fetch(remote, NULL, &fetch_opts, "pull");
    if (err < 0) die("git_remote_fetch", err);

    /* Find the FETCH_HEAD */
    git_oid fetch_head_oid;
    err = git_reference_name_to_id(&fetch_head_oid, repo, "FETCH_HEAD");
    if (err < 0) {
        /* Try remote tracking branch */
        git_reference *head_ref = NULL;
        err = git_repository_head(&head_ref, repo);
        if (err < 0) die("Cannot determine HEAD", err);

        const char *branch = git_reference_shorthand(head_ref);
        char refname[256];
        snprintf(refname, sizeof(refname), "refs/remotes/%s/%s", remote_name, branch);
        git_reference_free(head_ref);

        err = git_reference_name_to_id(&fetch_head_oid, repo, refname);
        if (err < 0) die("Cannot find remote tracking branch", err);
    }

    /* Fast-forward merge: move HEAD to FETCH_HEAD */
    git_annotated_commit *fetch_commit = NULL;
    err = git_annotated_commit_lookup(&fetch_commit, repo, &fetch_head_oid);
    if (err < 0) die("git_annotated_commit_lookup", err);

    git_merge_analysis_t analysis;
    git_merge_preference_t preference;
    const git_annotated_commit *merge_heads[] = { fetch_commit };
    err = git_merge_analysis(&analysis, &preference, repo, merge_heads, 1);
    if (err < 0) die("git_merge_analysis", err);

    if (analysis & GIT_MERGE_ANALYSIS_UP_TO_DATE) {
        printf("Already up to date.\n");
    } else if (analysis & GIT_MERGE_ANALYSIS_FASTFORWARD) {
        /* Fast-forward */
        git_reference *ref = NULL;
        err = git_repository_head(&ref, repo);
        if (err < 0) die("Cannot get HEAD", err);

        git_reference *new_ref = NULL;
        err = git_reference_set_target(&new_ref, ref, &fetch_head_oid, "pull: fast-forward");
        if (err < 0) die("git_reference_set_target", err);

        /* Checkout the new HEAD */
        git_checkout_options checkout_opts = GIT_CHECKOUT_OPTIONS_INIT;
        checkout_opts.checkout_strategy = GIT_CHECKOUT_FORCE;
        err = git_checkout_head(repo, &checkout_opts);
        if (err < 0) die("git_checkout_head", err);

        char oid_str[GIT_OID_SHA1_HEXSIZE + 1];
        git_oid_tostr(oid_str, sizeof(oid_str), &fetch_head_oid);
        printf("Fast-forward to %.7s\n", oid_str);

        git_reference_free(new_ref);
        git_reference_free(ref);
    } else {
        fprintf(stderr, "error: non-fast-forward merge not supported\n");
        fprintf(stderr, "hint: commit your changes first, or use fast-forward merges\n");
    }

    git_annotated_commit_free(fetch_commit);
    git_remote_free(remote);
    git_repository_free(repo);
    return 0;
}

/* ---- git push ---- */
static int cmd_push(int argc, char **argv) {
    const char *remote_name = (argc > 0) ? argv[0] : "origin";
    const char *refspec = NULL;

    /* Parse args: git push [remote] [refspec] */
    if (argc > 1) refspec = argv[1];

    git_repository *repo = NULL;
    int err = git_repository_open(&repo, ".");
    if (err < 0) die("Cannot open repository", err);

    git_remote *remote = NULL;
    err = git_remote_lookup(&remote, repo, remote_name);
    if (err < 0) die("Remote not found", err);

    /* Build refspec if not given */
    char refspec_buf[256];
    git_strarray refspecs = {0};
    if (refspec) {
        char *rs = (char *)refspec;
        refspecs.strings = &rs;
        refspecs.count = 1;
    } else {
        /* Push current branch */
        git_reference *head = NULL;
        err = git_repository_head(&head, repo);
        if (err < 0) die("Cannot determine HEAD", err);
        const char *name = git_reference_name(head);
        snprintf(refspec_buf, sizeof(refspec_buf), "%s:%s", name, name);
        git_reference_free(head);
        char *rs = refspec_buf;
        refspecs.strings = &rs;
        refspecs.count = 1;
    }

    git_push_options opts = GIT_PUSH_OPTIONS_INIT;

    printf("Pushing to %s...\n", git_remote_url(remote));
    err = git_remote_push(remote, &refspecs, &opts);
    if (err < 0) die("git_remote_push", err);

    printf("done.\n");

    git_remote_free(remote);
    git_repository_free(repo);
    return 0;
}

/* ---- main ---- */
static void usage(void) {
    fprintf(stderr, "usage: git <command> [<args>]\n\n");
    fprintf(stderr, "Commands:\n");
    fprintf(stderr, "  init       Create an empty repository\n");
    fprintf(stderr, "  clone      Clone a repository\n");
    fprintf(stderr, "  add        Add file contents to the index\n");
    fprintf(stderr, "  status     Show the working tree status\n");
    fprintf(stderr, "  commit     Record changes to the repository\n");
    fprintf(stderr, "  log        Show commit logs\n");
    fprintf(stderr, "  diff       Show changes in working tree\n");
    fprintf(stderr, "  remote     Manage remote repositories\n");
    fprintf(stderr, "  fetch      Download objects from a remote\n");
    fprintf(stderr, "  pull       Fetch and merge from a remote\n");
    fprintf(stderr, "  push       Update remote refs\n");
}

int main(int argc, char **argv) {
    if (argc < 2) {
        usage();
        return 1;
    }

    git_libgit2_init();
    bearssl_stream_register();

    const char *cmd = argv[1];
    int ret;

    if (strcmp(cmd, "init") == 0)
        ret = cmd_init(argc - 2, argv + 2);
    else if (strcmp(cmd, "clone") == 0)
        ret = cmd_clone(argc - 2, argv + 2);
    else if (strcmp(cmd, "add") == 0)
        ret = cmd_add(argc - 2, argv + 2);
    else if (strcmp(cmd, "status") == 0)
        ret = cmd_status(argc - 2, argv + 2);
    else if (strcmp(cmd, "commit") == 0)
        ret = cmd_commit(argc - 2, argv + 2);
    else if (strcmp(cmd, "log") == 0)
        ret = cmd_log(argc - 2, argv + 2);
    else if (strcmp(cmd, "diff") == 0)
        ret = cmd_diff(argc - 2, argv + 2);
    else if (strcmp(cmd, "remote") == 0)
        ret = cmd_remote(argc - 2, argv + 2);
    else if (strcmp(cmd, "fetch") == 0)
        ret = cmd_fetch(argc - 2, argv + 2);
    else if (strcmp(cmd, "pull") == 0)
        ret = cmd_pull(argc - 2, argv + 2);
    else if (strcmp(cmd, "push") == 0)
        ret = cmd_push(argc - 2, argv + 2);
    else {
        fprintf(stderr, "git: '%s' is not a git command\n", cmd);
        usage();
        ret = 1;
    }

    git_libgit2_shutdown();
    return ret;
}
