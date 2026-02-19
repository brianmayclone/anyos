CC = cc
CFLAGS = -o

TESTS = fork_test pipe_test dup_test pipe_chain testsuite

all: $(TESTS)
	@echo "All tests built."

fork_test: fork_test.c
	$(CC) $(CFLAGS) fork_test fork_test.c

pipe_test: pipe_test.c
	$(CC) $(CFLAGS) pipe_test pipe_test.c

dup_test: dup_test.c
	$(CC) $(CFLAGS) dup_test dup_test.c

pipe_chain: pipe_chain.c
	$(CC) $(CFLAGS) pipe_chain pipe_chain.c

testsuite: testsuite.c
	$(CC) $(CFLAGS) testsuite testsuite.c

clean:
	@echo "Cleaning tests..."

.PHONY: all clean
