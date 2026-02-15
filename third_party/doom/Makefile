# DOOM for anyOS — cross-compiled with i686-elf-gcc
CC = i686-elf-gcc
LD = i686-elf-gcc
AR = i686-elf-ar

# Paths to anyOS libc (set by CMake or pass on command line)
LIBC_DIR ?= ../libc
LIBC_INC = $(LIBC_DIR)/include
LIBC_A   = $(LIBC_DIR)/libc.a
CRT0     = $(LIBC_DIR)/obj/crt0.o
LINK_LD  = $(LIBC_DIR)/link.ld

SRCDIR = src
OBJDIR = obj

CFLAGS = -std=gnu99 -ffreestanding -nostdlib -nostdinc -fno-builtin -fno-stack-protector \
         -I$(LIBC_INC) -I$(SRCDIR) \
         -O2 -m32 -w \
         -DDOOMGENERIC_RESX=320 -DDOOMGENERIC_RESY=200

LDFLAGS = -nostdlib -static -m32 -T $(LINK_LD)

# All DOOM C sources
SRCS = $(wildcard $(SRCDIR)/*.c)
OBJS = $(patsubst $(SRCDIR)/%.c, $(OBJDIR)/%.o, $(SRCS))

TARGET = doom.elf

all: $(OBJDIR) $(TARGET)

$(OBJDIR):
	mkdir -p $(OBJDIR)

$(OBJDIR)/%.o: $(SRCDIR)/%.c
	$(CC) $(CFLAGS) -c $< -o $@

$(TARGET): $(OBJS) $(LIBC_A) $(CRT0)
	$(LD) $(LDFLAGS) -o $@ $(CRT0) $(OBJS) $(LIBC_A) -lgcc

clean:
	rm -rf $(OBJDIR) $(TARGET)

.PHONY: all clean
