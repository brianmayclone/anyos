#!/usr/bin/env python3
# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT

"""Convert an ELF executable to a flat binary image or DLIB v3 dynamic library.

Reads PT_LOAD segments from the ELF and produces either:
  - A contiguous flat binary starting at the lowest virtual address (default)
  - A DLIB v3 file with header + shared RO pages + per-process .data template (--dlib)
"""
import struct
import sys

PAGE_SIZE = 4096
PF_W = 0x2  # ELF segment flag: writable


def align_up(value: int, alignment: int) -> int:
    return (value + alignment - 1) & ~(alignment - 1)


def parse_elf_segments(data: bytes):
    """Parse ELF PT_LOAD segments. Returns (segments, ei_class).

    Each segment is (p_vaddr, p_offset, p_filesz, p_memsz, p_flags).
    """
    if data[:4] != b"\x7fELF":
        print("Error: not an ELF file", file=sys.stderr)
        sys.exit(1)

    ei_class = data[4]

    if ei_class == 2:
        e_phoff = struct.unpack_from("<Q", data, 32)[0]
        e_phentsize = struct.unpack_from("<H", data, 54)[0]
        e_phnum = struct.unpack_from("<H", data, 56)[0]
    else:
        e_phoff = struct.unpack_from("<I", data, 28)[0]
        e_phentsize = struct.unpack_from("<H", data, 42)[0]
        e_phnum = struct.unpack_from("<H", data, 44)[0]

    segments = []
    for i in range(e_phnum):
        off = e_phoff + i * e_phentsize
        if ei_class == 2:
            # ELF64 Phdr: p_type(4), p_flags(4), p_offset(8), p_vaddr(8),
            #             p_paddr(8), p_filesz(8), p_memsz(8), p_align(8)
            p_type = struct.unpack_from("<I", data, off)[0]
            p_flags = struct.unpack_from("<I", data, off + 4)[0]
            p_offset, p_vaddr, p_paddr, p_filesz, p_memsz = struct.unpack_from(
                "<QQQQQ", data, off + 8
            )
        else:
            # ELF32 Phdr: p_type(4), p_offset(4), p_vaddr(4), p_paddr(4),
            #             p_filesz(4), p_memsz(4), p_flags(4), p_align(4)
            p_type, p_offset, p_vaddr, p_paddr, p_filesz, p_memsz, p_flags = (
                struct.unpack_from("<IIIIIII", data, off)
            )
        if p_type == 1:  # PT_LOAD
            segments.append((p_vaddr, p_offset, p_filesz, p_memsz, p_flags))

    if not segments:
        print("Error: no PT_LOAD segments found", file=sys.stderr)
        sys.exit(1)

    return segments, ei_class


def elf2bin(elf_path: str, bin_path: str) -> None:
    with open(elf_path, "rb") as f:
        data = f.read()

    segments, _ = parse_elf_segments(data)

    base = min(s[0] for s in segments)
    end = max(s[0] + s[3] for s in segments)
    size = end - base

    flat = bytearray(size)
    for vaddr, offset, filesz, memsz, flags in segments:
        dest = vaddr - base
        flat[dest : dest + filesz] = data[offset : offset + filesz]

    with open(bin_path, "wb") as f:
        f.write(flat)

    print(f"  {elf_path} -> {bin_path} ({size} bytes, base={base:#010x})")


def elf2dlib(elf_path: str, dlib_path: str) -> None:
    """Convert ELF to DLIB v3 format.

    DLIB v3 layout:
      [0x000 .. 0xFFF]  4096-byte header (magic, version, section sizes)
      [0x1000 .. ]       RO content (.rodata + .text), page-aligned
      [ ... ]            .data template content, page-aligned
      (no .bss on disk — zeroed on demand by kernel)
    """
    with open(elf_path, "rb") as f:
        data = f.read()

    segments, _ = parse_elf_segments(data)

    # Separate RO and RW segments by ELF flags
    ro_segs = [(v, o, fs, ms) for v, o, fs, ms, fl in segments if not (fl & PF_W)]
    rw_segs = [(v, o, fs, ms) for v, o, fs, ms, fl in segments if (fl & PF_W)]

    if not ro_segs:
        print("Error: DLIB has no read-only segments (.rodata/.text)", file=sys.stderr)
        sys.exit(1)

    base = min(s[0] for s in ro_segs)

    if rw_segs:
        # RW region starts at the first writable segment (should be page-aligned by link.ld)
        rw_start = min(s[0] for s in rw_segs)
        rw_file_end = max(s[0] + s[2] for s in rw_segs)   # vaddr + filesz
        rw_mem_end = max(s[0] + s[3] for s in rw_segs)     # vaddr + memsz

        ro_size = rw_start - base
        data_file_size = rw_file_end - rw_start
        total_rw_memsz = rw_mem_end - rw_start

        # Page-align sizes
        ro_size = align_up(ro_size, PAGE_SIZE)
        data_size = align_up(data_file_size, PAGE_SIZE)
        total_rw_size = align_up(total_rw_memsz, PAGE_SIZE)
        bss_size = total_rw_size - data_size
    else:
        # No writable segments — pure RO library (backward compatible)
        ro_end = max(s[0] + s[3] for s in ro_segs)
        ro_size = align_up(ro_end - base, PAGE_SIZE)
        data_size = 0
        bss_size = 0

    ro_pages = ro_size // PAGE_SIZE
    data_pages = data_size // PAGE_SIZE
    bss_pages = bss_size // PAGE_SIZE
    total_pages = ro_pages + data_pages + bss_pages

    # Build flat content: RO pages + .data template pages (no BSS on disk)
    content_size = ro_size + data_size
    flat = bytearray(content_size)
    for vaddr, offset, filesz, memsz, flags in segments:
        dest = vaddr - base
        # Only write file content that fits within our content buffer
        copy_end = min(dest + filesz, content_size)
        if dest < content_size and copy_end > dest:
            src_len = copy_end - dest
            flat[dest : copy_end] = data[offset : offset + src_len]

    # Build 4096-byte DLIB v3 header
    header = bytearray(PAGE_SIZE)
    # magic (4) + version (4) + header_size (4) + flags (4)
    struct.pack_into("<4sIII", header, 0x00, b"DLIB", 3, PAGE_SIZE, 0)
    # base_vaddr (8)
    struct.pack_into("<Q", header, 0x10, base)
    # ro_pages (4) + data_pages (4) + bss_pages (4) + total_pages (4)
    struct.pack_into("<IIII", header, 0x18, ro_pages, data_pages, bss_pages, total_pages)

    with open(dlib_path, "wb") as f:
        f.write(header)
        f.write(flat)

    file_size = PAGE_SIZE + content_size
    print(
        f"  {elf_path} -> {dlib_path} (DLIB v3: {ro_pages} RO + {data_pages} data "
        f"+ {bss_pages} BSS pages, {file_size} bytes, base={base:#010x})"
    )


if __name__ == "__main__":
    if len(sys.argv) >= 2 and sys.argv[1] == "--dlib":
        if len(sys.argv) != 4:
            print(f"Usage: {sys.argv[0]} --dlib <input.elf> <output.dlib>", file=sys.stderr)
            sys.exit(1)
        elf2dlib(sys.argv[2], sys.argv[3])
    else:
        if len(sys.argv) != 3:
            print(f"Usage: {sys.argv[0]} [--dlib] <input.elf> <output.bin>", file=sys.stderr)
            sys.exit(1)
        elf2bin(sys.argv[1], sys.argv[2])
