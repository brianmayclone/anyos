#include <stdio.h>
#include <unistd.h>
#include <stdlib.h>
#include <signal.h>

static volatile int got_signal = 0;
static volatile int signal_num = 0;

void handler(int sig) {
    got_signal = 1;
    signal_num = sig;
}

int main(void) {
    int pass = 1;

    printf("signal test: starting (pid=%d)\n", getpid());

    /* Test 1: Install SIGUSR1 handler and raise it via kill(getpid(), SIGUSR1) */
    printf("  test 1: signal(SIGUSR1, handler) + kill(self, SIGUSR1)\n");
    got_signal = 0;
    signal_num = 0;
    signal(SIGUSR1, handler);
    kill(getpid(), SIGUSR1);

    /* Signal delivery happens on syscall return, so any syscall should trigger it.
       The kill() itself is a syscall, and signal delivery happens after it returns.
       But just in case, do a getpid() to ensure delivery. */
    getpid();

    if (got_signal && signal_num == SIGUSR1) {
        printf("    PASS: handler called with sig=%d\n", signal_num);
    } else {
        printf("    FAIL: got_signal=%d, signal_num=%d (expected %d)\n",
               got_signal, signal_num, SIGUSR1);
        pass = 0;
    }

    /* Test 2: SIG_IGN should ignore the signal */
    printf("  test 2: signal(SIGUSR1, SIG_IGN) + kill(self, SIGUSR1)\n");
    got_signal = 0;
    signal(SIGUSR1, SIG_IGN);
    kill(getpid(), SIGUSR1);
    getpid();  /* trigger delivery */

    if (!got_signal) {
        printf("    PASS: signal was ignored\n");
    } else {
        printf("    FAIL: handler was called despite SIG_IGN\n");
        pass = 0;
    }

    /* Test 3: SIGCHLD on child exit */
    printf("  test 3: fork + child exit -> parent SIGCHLD\n");
    got_signal = 0;
    signal_num = 0;
    signal(SIGCHLD, handler);

    int pid = fork();
    if (pid < 0) {
        printf("    SKIP: fork() failed\n");
    } else if (pid == 0) {
        /* child: exit immediately */
        _exit(0);
    } else {
        /* parent: wait for child, then check SIGCHLD */
        int status = 0;
        waitpid(pid, &status, 0);

        /* SIGCHLD may be delivered during waitpid or on next syscall */
        getpid();

        if (got_signal && signal_num == SIGCHLD) {
            printf("    PASS: SIGCHLD received (sig=%d)\n", signal_num);
        } else {
            printf("    FAIL: got_signal=%d, signal_num=%d (expected %d)\n",
                   got_signal, signal_num, SIGCHLD);
            pass = 0;
        }
    }

    if (pass) {
        printf("PASS: all signal tests passed!\n");
    } else {
        printf("FAIL: some signal tests failed!\n");
    }

    return pass ? 0 : 1;
}
