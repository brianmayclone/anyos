# anyld — anyOS ELF64 Shared Object Linker
#
# Build modes:
#   make              — Multi-file build (gcc/clang on host)
#   make one          — Single-source build (TCC-compatible, for anyOS)
#   make CC=tcc one   — Build with TCC

CC      ?= cc
CFLAGS  ?= -Wall -Wextra -O2 -std=c99
SRCDIR  = src
SRCS    = $(SRCDIR)/anyld.c $(SRCDIR)/input.c $(SRCDIR)/link.c \
          $(SRCDIR)/output.c $(SRCDIR)/defs.c
HDRS    = $(SRCDIR)/elf64.h $(SRCDIR)/anyld.h
OBJS    = $(SRCS:.c=.o)
TARGET  = anyld

# Default: multi-file build
all: $(TARGET)

$(TARGET): $(OBJS)
	$(CC) -o $@ $(OBJS)

$(SRCDIR)/%.o: $(SRCDIR)/%.c $(HDRS)
	$(CC) $(CFLAGS) -c $< -o $@

# Single-source build (for TCC / anyOS self-hosting)
one:
	$(CC) $(CFLAGS) -DONE_SOURCE -o $(TARGET) $(SRCDIR)/anyld.c

clean:
	rm -f $(TARGET) $(SRCDIR)/*.o

# Quick test: link a trivial .o into a .so
test: $(TARGET)
	@echo "=== anyld build successful ==="
	@echo "Run: ./anyld -o test.so -b 0x04000000 -e exports.def input.a"

.PHONY: all one clean test
