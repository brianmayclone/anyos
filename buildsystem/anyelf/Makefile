# anyelf — anyOS ELF conversion tool
#
# Build modes:
#   make              — Multi-file build (gcc/clang on host)
#   make one          — Single-source build (TCC-compatible, for anyOS)
#   make CC=tcc one   — Build with TCC

CC      ?= cc
CFLAGS  ?= -Wall -Wextra -O2 -std=c99
SRCDIR  = src
SRCS    = $(SRCDIR)/anyelf.c $(SRCDIR)/convert.c
HDRS    = $(SRCDIR)/anyelf.h
OBJS    = $(SRCS:.c=.o)
TARGET  = anyelf

# Default: multi-file build
all: $(TARGET)

$(TARGET): $(OBJS)
	$(CC) -o $@ $(OBJS)

$(SRCDIR)/%.o: $(SRCDIR)/%.c $(HDRS)
	$(CC) $(CFLAGS) -c $< -o $@

# Single-source build (for TCC / anyOS self-hosting)
one:
	$(CC) $(CFLAGS) -DONE_SOURCE -o $(TARGET) $(SRCDIR)/anyelf.c

clean:
	rm -f $(TARGET) $(SRCDIR)/*.o

test: $(TARGET)
	@echo "=== anyelf build successful ==="
	@echo "Run: ./anyelf bin input.elf output.bin"
	@echo "     ./anyelf dlib input.elf output.dlib"
	@echo "     ./anyelf kdrv input.elf output.kdrv"

.PHONY: all one clean test
