# TinyGL for anyOS — cross-compiled with i686-elf-gcc
# Software OpenGL 1.x rasterizer by Fabrice Bellard / C-Chads

CC ?= i686-elf-gcc
AR ?= i686-elf-ar

LIBC_DIR ?= ../../libs/libc
LIBC_INC = $(LIBC_DIR)/include

SRCDIR = src
OBJDIR = obj

CFLAGS = -std=c99 -ffreestanding -nostdlib -nostdinc -fno-builtin -fno-stack-protector \
         -I$(LIBC_INC) -Iinclude -I$(SRCDIR) \
         -O2 -m32 -w -DNDEBUG

SRCS = $(SRCDIR)/api.c \
       $(SRCDIR)/arrays.c \
       $(SRCDIR)/clear.c \
       $(SRCDIR)/clip.c \
       $(SRCDIR)/get.c \
       $(SRCDIR)/image_util.c \
       $(SRCDIR)/init.c \
       $(SRCDIR)/light.c \
       $(SRCDIR)/list.c \
       $(SRCDIR)/matrix.c \
       $(SRCDIR)/memory.c \
       $(SRCDIR)/misc.c \
       $(SRCDIR)/msghandling.c \
       $(SRCDIR)/select.c \
       $(SRCDIR)/specbuf.c \
       $(SRCDIR)/texture.c \
       $(SRCDIR)/vertex.c \
       $(SRCDIR)/zbuffer.c \
       $(SRCDIR)/zline.c \
       $(SRCDIR)/zmath.c \
       $(SRCDIR)/zpostprocess.c \
       $(SRCDIR)/zraster.c \
       $(SRCDIR)/ztext.c \
       $(SRCDIR)/ztriangle.c \
       $(SRCDIR)/accum.c

OBJS = $(patsubst $(SRCDIR)/%.c,$(OBJDIR)/%.o,$(SRCS))

TARGET = libtinygl.a

all: $(OBJDIR) $(TARGET)

$(OBJDIR):
	mkdir -p $(OBJDIR)

$(OBJDIR)/%.o: $(SRCDIR)/%.c
	$(CC) $(CFLAGS) -c $< -o $@

$(TARGET): $(OBJS)
	$(AR) rcs $@ $^

clean:
	rm -rf $(OBJDIR) $(TARGET)

.PHONY: all clean
