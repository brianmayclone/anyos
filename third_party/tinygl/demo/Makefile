# GLCube demo for anyOS — TinyGL spinning cube
CC ?= i686-elf-gcc
LD = $(CC)

LIBC_DIR ?= ../../../libs/libc
TINYGL_DIR ?= ..

LIBC_INC = $(LIBC_DIR)/include
LIBC_A   = $(LIBC_DIR)/libc.a
CRT0     = $(LIBC_DIR)/obj/crt0.o
LINK_LD  = $(LIBC_DIR)/link.ld
TINYGL_A = $(TINYGL_DIR)/libtinygl.a

CFLAGS = -std=c99 -ffreestanding -nostdlib -nostdinc -fno-builtin -fno-stack-protector \
         -I$(LIBC_INC) -I$(TINYGL_DIR)/include -I$(TINYGL_DIR)/src \
         -O2 -m32 -w -DNDEBUG

LDFLAGS = -nostdlib -static -m32 -T $(LINK_LD)

OBJDIR = obj
TARGET = glcube.elf

all: $(OBJDIR) $(TARGET)

$(OBJDIR):
	mkdir -p $(OBJDIR)

$(OBJDIR)/glcube.o: glcube.c
	$(CC) $(CFLAGS) -c $< -o $@

$(TARGET): $(OBJDIR)/glcube.o $(TINYGL_A) $(LIBC_A) $(CRT0)
	$(LD) $(LDFLAGS) -o $@ $(CRT0) $(OBJDIR)/glcube.o $(TINYGL_A) $(LIBC_A) -lgcc

clean:
	rm -rf $(OBJDIR) $(TARGET)

.PHONY: all clean
