# mkappbundle — anyOS .app bundle creator
#
# Build modes:
#   make              — Standard build (gcc/clang on host)
#   make one          — Single-source build (TCC-compatible, for anyOS)
#   make CC=tcc one   — Build with TCC

CC      ?= cc
CFLAGS  ?= -Wall -Wextra -O2 -std=c99
SRCDIR  = src
SRCS    = $(SRCDIR)/mkappbundle.c
OBJS    = $(SRCS:.c=.o)
TARGET  = mkappbundle

# Default: standard build
all: $(TARGET)

$(TARGET): $(OBJS)
	$(CC) -o $@ $(OBJS)

$(SRCDIR)/%.o: $(SRCDIR)/%.c
	$(CC) $(CFLAGS) -c $< -o $@

# Single-source build (for TCC / anyOS self-hosting)
one:
	$(CC) $(CFLAGS) -DONE_SOURCE -o $(TARGET) $(SRCDIR)/mkappbundle.c

clean:
	rm -f $(TARGET) $(SRCDIR)/*.o

test: $(TARGET)
	@echo "=== mkappbundle build successful ==="
	@echo "Run: ./mkappbundle -i Info.conf -e binary -o Output.app"
	@echo "     ./mkappbundle --help"

.PHONY: all one clean test
