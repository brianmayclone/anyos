#!/usr/bin/env python3
# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT

"""Convert an ELF executable to a flat binary image.

Reads PT_LOAD segments from the ELF and produces a contiguous binary
starting at the lowest virtual address.
"""
import struct
import sys


def elf2bin(elf_path: str, bin_path: str) -> None:
    with open(elf_path, "rb") as f:
        data = f.read()

    # Verify ELF magic
    if data[:4] != b"\x7fELF":
        print(f"Error: {elf_path} is not an ELF file", file=sys.stderr)
        sys.exit(1)

    # Detect ELF class: 1 = 32-bit, 2 = 64-bit
    ei_class = data[4]

    if ei_class == 2:
        # ELF64: e_phoff at offset 32 (8 bytes), e_phentsize at 54, e_phnum at 56
        e_phoff = struct.unpack_from("<Q", data, 32)[0]
        e_phentsize = struct.unpack_from("<H", data, 54)[0]
        e_phnum = struct.unpack_from("<H", data, 56)[0]
    else:
        # ELF32: e_phoff at offset 28 (4 bytes), e_phentsize at 42, e_phnum at 44
        e_phoff = struct.unpack_from("<I", data, 28)[0]
        e_phentsize = struct.unpack_from("<H", data, 42)[0]
        e_phnum = struct.unpack_from("<H", data, 44)[0]

    # Collect PT_LOAD segments
    segments = []
    for i in range(e_phnum):
        off = e_phoff + i * e_phentsize
        if ei_class == 2:
            # ELF64 Phdr: p_type(4), p_flags(4), p_offset(8), p_vaddr(8),
            #             p_paddr(8), p_filesz(8), p_memsz(8), p_align(8)
            p_type = struct.unpack_from("<I", data, off)[0]
            p_offset, p_vaddr, p_paddr, p_filesz, p_memsz = struct.unpack_from(
                "<QQQQQ", data, off + 8
            )
        else:
            # ELF32 Phdr: p_type(4), p_offset(4), p_vaddr(4), p_paddr(4),
            #             p_filesz(4), p_memsz(4), p_flags(4), p_align(4)
            p_type, p_offset, p_vaddr, p_paddr, p_filesz, p_memsz = struct.unpack_from(
                "<IIIIII", data, off
            )
        if p_type == 1:  # PT_LOAD
            segments.append((p_vaddr, p_offset, p_filesz, p_memsz))

    if not segments:
        print("Error: no PT_LOAD segments found", file=sys.stderr)
        sys.exit(1)

    # Calculate range
    base = min(s[0] for s in segments)
    end = max(s[0] + s[3] for s in segments)
    size = end - base

    # Build flat binary (memsz includes BSS zeroes)
    flat = bytearray(size)
    for vaddr, offset, filesz, memsz in segments:
        dest = vaddr - base
        flat[dest : dest + filesz] = data[offset : offset + filesz]

    with open(bin_path, "wb") as f:
        f.write(flat)

    print(f"  {elf_path} -> {bin_path} ({size} bytes, base={base:#010x})")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        print(f"Usage: {sys.argv[0]} <input.elf> <output.bin>", file=sys.stderr)
        sys.exit(1)
    elf2bin(sys.argv[1], sys.argv[2])
