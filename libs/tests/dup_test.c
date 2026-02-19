#include <stdio.h>
#include <unistd.h>
#include <fcntl.h>
#include <string.h>

int main(void) {
    int fds[2];
    printf("dup_test: testing dup/dup2...\n");

    /* Test 1: dup() */
    if (pipe(fds) < 0) {
        printf("FAIL: pipe() failed\n");
        return 1;
    }
    int dup_rd = dup(fds[0]);
    if (dup_rd < 0) {
        printf("FAIL: dup() returned -1\n");
        return 1;
    }
    printf("  dup(%d) = %d\n", fds[0], dup_rd);

    /* Write through original, read through dup */
    write(fds[1], "abc", 3);
    char buf[16];
    memset(buf, 0, sizeof(buf));
    int n = read(dup_rd, buf, sizeof(buf));
    printf("  read via dup'd fd: %d bytes \"%s\"\n", n, buf);
    if (n != 3 || memcmp(buf, "abc", 3) != 0) {
        printf("FAIL: dup read mismatch\n");
        return 1;
    }
    close(dup_rd);
    close(fds[0]);
    close(fds[1]);

    /* Test 2: dup2() — redirect stdout to pipe */
    if (pipe(fds) < 0) {
        printf("FAIL: pipe() #2 failed\n");
        return 1;
    }
    printf("  dup2(%d, 1) — redirecting stdout to pipe...\n", fds[1]);

    int saved_stdout = dup(1);
    dup2(fds[1], 1);
    close(fds[1]);

    /* This printf goes to the pipe, not to serial */
    printf("redirected!");

    /* Restore stdout */
    dup2(saved_stdout, 1);
    close(saved_stdout);

    /* Read what printf wrote into the pipe */
    memset(buf, 0, sizeof(buf));
    n = read(fds[0], buf, sizeof(buf));
    close(fds[0]);

    printf("  captured from redirected stdout: %d bytes \"%s\"\n", n, buf);
    if (n > 0 && memcmp(buf, "redirected!", 11) == 0) {
        printf("PASS: dup_test succeeded!\n");
    } else {
        printf("FAIL: expected \"redirected!\", got \"%s\"\n", buf);
    }

    return 0;
}
