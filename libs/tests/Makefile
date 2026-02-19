CC = cc
CFLAGS = -o

TESTS = fork_test

all: $(TESTS)
	@echo "All tests built."

fork_test: fork_test.c
	$(CC) $(CFLAGS) fork_test fork_test.c

clean:
	@echo "Cleaning tests..."

.PHONY: all clean
