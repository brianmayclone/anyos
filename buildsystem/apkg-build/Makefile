# apkg-build — anyOS package archive creator
#
# Build modes:
#   make              — Standard build (gcc/clang on host)
#   make one          — Single-source build (TCC-compatible, for anyOS)
#   make CC=tcc one   — Build with TCC

CC      ?= cc
CFLAGS  ?= -Wall -Wextra -O2 -std=c99
SRCDIR  = src
SRCS    = $(SRCDIR)/apkg_build.c
OBJS    = $(SRCS:.c=.o)
TARGET  = apkg-build

# Default: standard build
all: $(TARGET)

$(TARGET): $(OBJS)
	$(CC) -o $@ $(OBJS)

$(SRCDIR)/%.o: $(SRCDIR)/%.c
	$(CC) $(CFLAGS) -c $< -o $@

# Single-source build (for TCC / anyOS self-hosting)
one:
	$(CC) $(CFLAGS) -DONE_SOURCE -o $(TARGET) $(SRCDIR)/apkg_build.c

clean:
	rm -f $(TARGET) $(SRCDIR)/*.o

.PHONY: all one clean
