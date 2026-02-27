# apkg-index — anyOS package repository index generator
#
# Build modes:
#   make              — Standard build (gcc/clang on host)
#   make one          — Single-source build (TCC-compatible, for anyOS)
#   make CC=tcc one   — Build with TCC

CC      ?= cc
CFLAGS  ?= -Wall -Wextra -O2 -std=c99
SRCDIR  = src
SRCS    = $(SRCDIR)/apkg_index.c
OBJS    = $(SRCS:.c=.o)
TARGET  = apkg-index

# Default: standard build
all: $(TARGET)

$(TARGET): $(OBJS)
	$(CC) -o $@ $(OBJS)

$(SRCDIR)/%.o: $(SRCDIR)/%.c
	$(CC) $(CFLAGS) -c $< -o $@

# Single-source build (for TCC / anyOS self-hosting)
one:
	$(CC) $(CFLAGS) -DONE_SOURCE -o $(TARGET) $(SRCDIR)/apkg_index.c

clean:
	rm -f $(TARGET) $(SRCDIR)/*.o

.PHONY: all one clean
