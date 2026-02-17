# Configuration for anyOS cross-compilation (i686-elf, bare metal).

# Build directory.
BUILD = build

# Extension for executable files.
E =

# Extension for object files.
O = .o

# Prefix for library file name.
LP = lib

# Extension for library file name.
L = .a

# No DLL support for bare metal.
DP =
D =

# File deletion tool.
RM = rm -f

# Directory creation tool.
MKDIR = mkdir -p

# C compiler and flags.
CC = i686-elf-gcc
LIBC_INC = $(shell cd $(dir $(lastword $(MAKEFILE_LIST)))/../../../libs/libc/include && pwd)
CFLAGS = -W -Wall -Os -ffreestanding -nostdlib -fno-builtin \
         -isystem $(LIBC_INC) \
         -DBR_USE_UNIX_TIME=0 -DBR_USE_WIN32_TIME=0 \
         -DBR_RDRAND=0 -DBR_POWER8=0 -DBR_AESNI=0 \
         -DBR_SSE2=0 -DBR_PCLMUL=0 -DBR_64=0 \
         -DBR_i386=1 -DBR_LOMUL=1
CCOUT = -c -o

# Static library building tool.
AR = i686-elf-ar
ARFLAGS = -rcs
AROUT =

# No DLL or linking for bare metal static lib only.
LDDLL = true
LDDLLFLAGS =
LDDLLOUT =

LD = true
LDFLAGS =
LDOUT =

# No T0 compiler needed.
MKT0COMP = true
RUNT0COMP = true

# Only build static library.
STATICLIB = yes
DLL = no
TOOLS = no
TESTS = no
