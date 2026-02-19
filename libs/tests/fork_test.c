#include <stdio.h>
#include <unistd.h>
#include <stdlib.h>

int main(void) {
    printf("fork test: starting (pid=%d)\n", getpid());

    int pid = fork();
    if (pid < 0) {
        printf("fork FAILED!\n");
        return 1;
    }

    if (pid == 0) {
        printf("  child: pid=%d, fork returned 0\n", getpid());
        printf("  child: exiting with code 42\n");
        _exit(42);
    }

    printf("  parent: fork returned child pid=%d\n", pid);
    printf("  parent: waiting for child...\n");

    int status = 0;
    waitpid(pid, &status, 0);

    printf("  parent: child exited with code %d\n", status);

    if (status == 42) {
        printf("PASS: fork test succeeded!\n");
    } else {
        printf("FAIL: expected exit code 42, got %d\n", status);
    }

    return 0;
}
