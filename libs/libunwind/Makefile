# libunwind — Minimal DWARF .eh_frame stack unwinder for anyOS (x86_64)

CC       = clang
AS       = clang
AR       = ar
CFLAGS   = --target=x86_64-unknown-none-elf -ffreestanding -nostdlib -nostdinc \
           -fno-builtin -fno-stack-protector -fno-exceptions -O2 \
           -I include -I ../libc64/include -Wall -Wextra -Wno-unused-parameter $(EXTRA_CFLAGS)
ASFLAGS  = --target=x86_64-unknown-none-elf -c

SRCDIR = src
OBJDIR = obj

C_SRCS = $(wildcard $(SRCDIR)/*.c)
S_SRCS = $(wildcard $(SRCDIR)/*.S)
C_OBJS = $(patsubst $(SRCDIR)/%.c, $(OBJDIR)/%.o, $(C_SRCS))
S_OBJS = $(patsubst $(SRCDIR)/%.S, $(OBJDIR)/%.o, $(S_SRCS))
OBJECTS = $(C_OBJS) $(S_OBJS)

TARGET = libunwind.a

.PHONY: all clean

all: $(TARGET)

$(TARGET): $(OBJECTS)
	$(AR) rcs $@ $^

$(OBJDIR)/%.o: $(SRCDIR)/%.c | $(OBJDIR)
	$(CC) $(CFLAGS) -c $< -o $@

$(OBJDIR)/%.o: $(SRCDIR)/%.S | $(OBJDIR)
	$(AS) $(ASFLAGS) $< -o $@

$(OBJDIR):
	mkdir -p $(OBJDIR)

clean:
	rm -rf $(OBJDIR) $(TARGET)
