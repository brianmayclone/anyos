#include <stdio.h>
#include <unistd.h>
#include <stdlib.h>
#include <string.h>

int main(void) {
    int fds[2];
    printf("pipe_test: creating pipe...\n");

    if (pipe(fds) < 0) {
        printf("FAIL: pipe() returned -1\n");
        return 1;
    }
    printf("  pipe created: read_fd=%d, write_fd=%d\n", fds[0], fds[1]);

    int pid = fork();
    if (pid < 0) {
        printf("FAIL: fork() returned -1\n");
        return 1;
    }

    if (pid == 0) {
        /* Child: close write end, read from pipe */
        close(fds[1]);
        char buf[64];
        memset(buf, 0, sizeof(buf));
        int n = read(fds[0], buf, sizeof(buf) - 1);
        close(fds[0]);
        printf("  child: read %d bytes: \"%s\"\n", n, buf);
        if (n == 5 && memcmp(buf, "hello", 5) == 0) {
            printf("PASS: pipe_test succeeded!\n");
        } else {
            printf("FAIL: expected \"hello\" (5 bytes), got \"%s\" (%d bytes)\n", buf, n);
        }
        _exit(0);
    }

    /* Parent: close read end, write to pipe */
    close(fds[0]);
    write(fds[1], "hello", 5);
    close(fds[1]);
    printf("  parent: wrote \"hello\" to pipe, waiting for child...\n");

    int status = 0;
    waitpid(pid, &status, 0);
    printf("  parent: child exited with code %d\n", status);
    return 0;
}
