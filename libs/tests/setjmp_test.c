#include <stdio.h>
#include <setjmp.h>

static jmp_buf jbuf;

int main(void) {
    int pass = 1;

    printf("setjmp test: starting\n");

    /* Test 1: setjmp returns 0 on first call */
    printf("  test 1: setjmp() returns 0 on first call\n");
    int val = setjmp(jbuf);
    if (val == 0) {
        printf("    PASS: setjmp returned 0\n");

        /* Test 2: longjmp returns the specified value */
        printf("  test 2: longjmp(jbuf, 42) -> setjmp returns 42\n");
        longjmp(jbuf, 42);
        /* should not reach here */
        printf("    FAIL: longjmp did not jump back\n");
        pass = 0;
    } else if (val == 42) {
        printf("    PASS: setjmp returned 42 after longjmp\n");
    } else {
        printf("    FAIL: setjmp returned %d (expected 42)\n", val);
        pass = 0;
    }

    /* Test 3: longjmp with val=0 should return 1 */
    printf("  test 3: longjmp(jbuf, 0) -> setjmp returns 1\n");
    val = setjmp(jbuf);
    if (val == 0) {
        longjmp(jbuf, 0);
        printf("    FAIL: longjmp did not jump back\n");
        pass = 0;
    } else if (val == 1) {
        printf("    PASS: setjmp returned 1 (val=0 corrected to 1)\n");
    } else {
        printf("    FAIL: setjmp returned %d (expected 1)\n", val);
        pass = 0;
    }

    /* Test 4: Nested function call + longjmp (tests stack unwinding) */
    printf("  test 4: longjmp from nested function call\n");
    val = setjmp(jbuf);
    if (val == 0) {
        /* Call a function that longjmps */
        volatile int x = 99;
        (void)x;
        longjmp(jbuf, 7);
        printf("    FAIL: should not reach here\n");
        pass = 0;
    } else if (val == 7) {
        printf("    PASS: nested longjmp returned 7\n");
    } else {
        printf("    FAIL: setjmp returned %d (expected 7)\n", val);
        pass = 0;
    }

    if (pass) {
        printf("PASS: all setjmp tests passed!\n");
    } else {
        printf("FAIL: some setjmp tests failed!\n");
    }

    return pass ? 0 : 1;
}
