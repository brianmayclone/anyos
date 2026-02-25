CC = clang
AR = ar
AS = clang

CFLAGS = --target=x86_64-unknown-none-elf -ffreestanding -nostdlib -nostdinc \
         -fno-builtin -fno-stack-protector -Wall -Wextra -O2 -I include
ASFLAGS = --target=x86_64-unknown-none-elf -c

SRCDIR = src
OBJDIR = obj

C_SRCS = $(wildcard $(SRCDIR)/*.c)
S_SRCS = $(wildcard $(SRCDIR)/*.S)
C_OBJS = $(patsubst $(SRCDIR)/%.c, $(OBJDIR)/%.o, $(C_SRCS))
S_OBJS = $(patsubst $(SRCDIR)/%.S, $(OBJDIR)/%.o, $(S_SRCS))

# crt0.o is separate (not in libc64.a)
CRT0_OBJ = $(OBJDIR)/crt0.o
CRTI_OBJ = $(OBJDIR)/crti.o
CRTN_OBJ = $(OBJDIR)/crtn.o
LIB_OBJS = $(filter-out $(CRT0_OBJ) $(CRTI_OBJ) $(CRTN_OBJ), $(C_OBJS) $(S_OBJS))

all: $(OBJDIR) libc64.a $(CRT0_OBJ) $(CRTI_OBJ) $(CRTN_OBJ)

$(OBJDIR):
	cmake -E make_directory $(OBJDIR)

$(OBJDIR)/%.o: $(SRCDIR)/%.c
	$(CC) $(CFLAGS) -c $< -o $@

$(OBJDIR)/%.o: $(SRCDIR)/%.S
	$(AS) $(ASFLAGS) $< -o $@

libc64.a: $(LIB_OBJS)
	$(AR) rcs $@ $^

clean:
	cmake -E rm -rf $(OBJDIR) libc64.a

.PHONY: all clean
