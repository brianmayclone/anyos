CC = i686-elf-gcc
AR = i686-elf-ar
AS = i686-elf-gcc

CFLAGS = -ffreestanding -nostdlib -nostdinc -fno-builtin -fno-stack-protector \
         -Wall -Wextra -O2 -I include -m32
ASFLAGS = -m32

SRCDIR = src
OBJDIR = obj

C_SRCS = $(wildcard $(SRCDIR)/*.c)
S_SRCS = $(wildcard $(SRCDIR)/*.S)
C_OBJS = $(patsubst $(SRCDIR)/%.c, $(OBJDIR)/%.o, $(C_SRCS))
S_OBJS = $(patsubst $(SRCDIR)/%.S, $(OBJDIR)/%.o, $(S_SRCS))

# crt0.o is separate (not in libc.a)
CRT0_OBJ = $(OBJDIR)/crt0.o
LIB_OBJS = $(filter-out $(CRT0_OBJ), $(C_OBJS) $(S_OBJS))

all: $(OBJDIR) libc.a $(CRT0_OBJ)

$(OBJDIR):
	mkdir -p $(OBJDIR)

$(OBJDIR)/%.o: $(SRCDIR)/%.c
	$(CC) $(CFLAGS) -c $< -o $@

$(OBJDIR)/%.o: $(SRCDIR)/%.S
	$(AS) $(ASFLAGS) -c $< -o $@

libc.a: $(LIB_OBJS)
	$(AR) rcs $@ $^

clean:
	rm -rf $(OBJDIR) libc.a

.PHONY: all clean
