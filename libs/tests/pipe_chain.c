#include <stdio.h>
#include <unistd.h>
#include <string.h>

/* Simulates: echo "hello world" | cat
   Parent writes to pipe, child reads and prints. */
int main(void) {
    int fds[2];
    printf("pipe_chain: simulating echo | cat...\n");

    if (pipe(fds) < 0) {
        printf("FAIL: pipe() failed\n");
        return 1;
    }

    int pid = fork();
    if (pid < 0) {
        printf("FAIL: fork() failed\n");
        return 1;
    }

    if (pid == 0) {
        /* Child = "cat": redirect stdin from pipe, then read+print */
        close(fds[1]);       /* close write end */
        dup2(fds[0], 0);    /* stdin = pipe read end */
        close(fds[0]);       /* close original read fd */

        char buf[128];
        memset(buf, 0, sizeof(buf));
        int total = 0;
        int n;
        while ((n = read(0, buf + total, sizeof(buf) - total - 1)) > 0) {
            total += n;
        }
        buf[total] = '\0';
        printf("  cat received: \"%s\" (%d bytes)\n", buf, total);

        if (total == 12 && memcmp(buf, "hello world\n", 12) == 0) {
            printf("PASS: pipe_chain succeeded!\n");
        } else {
            printf("FAIL: expected \"hello world\\n\" (12), got %d bytes\n", total);
        }
        _exit(0);
    }

    /* Parent = "echo": write to pipe */
    close(fds[0]);           /* close read end */
    write(fds[1], "hello world\n", 12);
    close(fds[1]);           /* EOF for reader */

    int status = 0;
    waitpid(pid, &status, 0);
    printf("  parent: child exited with code %d\n", status);
    return 0;
}
