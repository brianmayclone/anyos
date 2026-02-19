#include <stdio.h>
#include <unistd.h>
#include <stdlib.h>
#include <string.h>

static const char *tests[] = {
    "fork_test",
    "pipe_test",
    "dup_test",
    "pipe_chain",
    NULL
};

int main(void) {
    int total = 0, passed = 0, failed = 0;

    printf("=== anyOS Test Suite ===\n\n");

    for (int i = 0; tests[i] != NULL; i++) {
        total++;
        printf("--- Running: %s ---\n", tests[i]);

        int pid = fork();
        if (pid < 0) {
            printf("  SKIP: fork() failed\n");
            failed++;
            continue;
        }

        if (pid == 0) {
            /* Child: build path from cwd + test name */
            char cwd[128];
            char path[256];
            if (getcwd(cwd, sizeof(cwd)) == NULL) {
                printf("  ERROR: getcwd() failed\n");
                _exit(127);
            }
            memset(path, 0, sizeof(path));
            strcpy(path, cwd);
            if (path[strlen(path) - 1] != '/')
                strcat(path, "/");
            strcat(path, tests[i]);
            char *argv[] = { (char *)tests[i], NULL };
            execv(path, argv);
            printf("  ERROR: exec('%s') failed\n", path);
            _exit(127);
        }

        /* Parent: wait for child */
        int status = 0;
        waitpid(pid, &status, 0);

        if (status == 0) {
            passed++;
            printf("--- %s: OK (exit %d) ---\n\n", tests[i], status);
        } else if (status == 127) {
            failed++;
            printf("--- %s: EXEC FAILED ---\n\n", tests[i]);
        } else {
            failed++;
            printf("--- %s: FAILED (exit %d) ---\n\n", tests[i], status);
        }
    }

    printf("=== Results: %d/%d passed", passed, total);
    if (failed > 0) printf(", %d FAILED", failed);
    printf(" ===\n");

    return failed > 0 ? 1 : 0;
}
