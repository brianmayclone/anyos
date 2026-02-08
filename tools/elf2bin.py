#!/usr/bin/env python3
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

    # Parse ELF header (32-bit little-endian)
    e_phoff = struct.unpack_from("<I", data, 28)[0]
    e_phentsize = struct.unpack_from("<H", data, 42)[0]
    e_phnum = struct.unpack_from("<H", data, 44)[0]

    # Collect PT_LOAD segments
    segments = []
    for i in range(e_phnum):
        off = e_phoff + i * e_phentsize
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
