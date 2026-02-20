# mkimage — anyOS disk image builder
#
# Build modes:
#   make              — Multi-file build (gcc/clang on host)
#   make one          — Single-source build (TCC-compatible, for anyOS)
#   make CC=tcc one   — Build with TCC

CC      ?= cc
CFLAGS  ?= -Wall -Wextra -O2 -std=c99
SRCDIR  = src
SRCS    = $(SRCDIR)/mkimage.c $(SRCDIR)/elf.c $(SRCDIR)/fat16.c \
          $(SRCDIR)/exfat.c $(SRCDIR)/gpt.c $(SRCDIR)/iso9660.c
HDRS    = $(SRCDIR)/mkimage.h
OBJS    = $(SRCS:.c=.o)
TARGET  = mkimage

# Default: multi-file build
all: $(TARGET)

$(TARGET): $(OBJS)
	$(CC) -o $@ $(OBJS)

$(SRCDIR)/%.o: $(SRCDIR)/%.c $(HDRS)
	$(CC) $(CFLAGS) -c $< -o $@

# Single-source build (for TCC / anyOS self-hosting)
one:
	$(CC) $(CFLAGS) -DONE_SOURCE -o $(TARGET) $(SRCDIR)/mkimage.c

clean:
	rm -f $(TARGET) $(SRCDIR)/*.o

test: $(TARGET)
	@echo "=== mkimage build successful ==="
	@echo "Usage: ./mkimage --stage1 s1.bin --stage2 s2.bin --kernel k.elf --output disk.img"

.PHONY: all one clean test
