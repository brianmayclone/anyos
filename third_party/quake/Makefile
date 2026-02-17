# Quake for anyOS — cross-compiled with i686-elf-gcc
CC = i686-elf-gcc
LD = i686-elf-gcc

# Paths to anyOS libc (set by CMake or pass on command line)
LIBC_DIR ?= ../../programs/libc
LIBC_INC = $(LIBC_DIR)/include
LIBC_A   = $(LIBC_DIR)/libc.a
CRT0     = $(LIBC_DIR)/obj/crt0.o
LINK_LD  = $(LIBC_DIR)/link.ld

SRCDIR = WinQuake
OBJDIR = obj

# -U__i386__ forces id386=0 in quakedef.h — use pure C renderer (no ASM)
CFLAGS = -std=gnu99 -ffreestanding -nostdlib -nostdinc -fno-builtin -fno-stack-protector \
         -I$(LIBC_INC) -I$(SRCDIR) \
         -O2 -m32 -w -Wno-error \
         -Wno-implicit-function-declaration -Wno-implicit-int -Wno-int-conversion \
         -Wno-incompatible-pointer-types -Wno-return-mismatch \
         -fcommon -U__i386__

LDFLAGS = -nostdlib -static -m32 -T $(LINK_LD)

# Core engine
SRCS = \
	$(SRCDIR)/chase.c \
	$(SRCDIR)/cl_demo.c \
	$(SRCDIR)/cl_input.c \
	$(SRCDIR)/cl_main.c \
	$(SRCDIR)/cl_parse.c \
	$(SRCDIR)/cl_tent.c \
	$(SRCDIR)/cmd.c \
	$(SRCDIR)/common.c \
	$(SRCDIR)/console.c \
	$(SRCDIR)/crc.c \
	$(SRCDIR)/cvar.c \
	$(SRCDIR)/d_edge.c \
	$(SRCDIR)/d_fill.c \
	$(SRCDIR)/d_init.c \
	$(SRCDIR)/d_modech.c \
	$(SRCDIR)/d_part.c \
	$(SRCDIR)/d_polyse.c \
	$(SRCDIR)/d_scan.c \
	$(SRCDIR)/d_sky.c \
	$(SRCDIR)/d_sprite.c \
	$(SRCDIR)/d_surf.c \
	$(SRCDIR)/d_vars.c \
	$(SRCDIR)/d_zpoint.c \
	$(SRCDIR)/draw.c \
	$(SRCDIR)/host.c \
	$(SRCDIR)/host_cmd.c \
	$(SRCDIR)/keys.c \
	$(SRCDIR)/mathlib.c \
	$(SRCDIR)/menu.c \
	$(SRCDIR)/model.c \
	$(SRCDIR)/nonintel.c \
	$(SRCDIR)/pr_cmds.c \
	$(SRCDIR)/pr_edict.c \
	$(SRCDIR)/pr_exec.c \
	$(SRCDIR)/r_aclip.c \
	$(SRCDIR)/r_alias.c \
	$(SRCDIR)/r_bsp.c \
	$(SRCDIR)/r_draw.c \
	$(SRCDIR)/r_edge.c \
	$(SRCDIR)/r_efrag.c \
	$(SRCDIR)/r_light.c \
	$(SRCDIR)/r_main.c \
	$(SRCDIR)/r_misc.c \
	$(SRCDIR)/r_part.c \
	$(SRCDIR)/r_sky.c \
	$(SRCDIR)/r_sprite.c \
	$(SRCDIR)/r_surf.c \
	$(SRCDIR)/r_vars.c \
	$(SRCDIR)/sbar.c \
	$(SRCDIR)/screen.c \
	$(SRCDIR)/sv_main.c \
	$(SRCDIR)/sv_move.c \
	$(SRCDIR)/sv_phys.c \
	$(SRCDIR)/sv_user.c \
	$(SRCDIR)/view.c \
	$(SRCDIR)/wad.c \
	$(SRCDIR)/world.c \
	$(SRCDIR)/zone.c \
	$(SRCDIR)/net_loop.c \
	$(SRCDIR)/net_main.c \
	$(SRCDIR)/net_none.c \
	$(SRCDIR)/net_vcr.c \
	$(SRCDIR)/sys_anyos.c \
	$(SRCDIR)/vid_anyos.c \
	$(SRCDIR)/cd_null.c \
	$(SRCDIR)/snd_null.c

OBJS = $(patsubst $(SRCDIR)/%.c, $(OBJDIR)/%.o, $(SRCS))

TARGET = quake.elf

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
