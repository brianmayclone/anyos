#!/usr/bin/env python3
"""Convert a kernel-target ELF binary to KDRV format for anyOS loadable drivers.

Usage:
    python3 elf2kdrv.py input.elf output.kdrv [--exports-symbol DRIVER_EXPORTS]

The ELF must be built with the x86_64-anyos.json kernel target. The tool:
  1. Parses ELF64 PT_LOAD segments to determine code, data, and BSS sizes
  2. Finds the DRIVER_EXPORTS symbol for the exports_offset field
  3. Outputs a KDRV binary: 4096-byte header + code pages + data pages
     (BSS is not stored — the kernel zeros those pages at load time)
"""

import struct
import sys
import os

PAGE_SIZE = 4096
KDRV_MAGIC = b'KDRV'
KDRV_VERSION = 1
KDRV_ABI_VERSION = 1

# ELF constants
EI_CLASS = 4
ELFCLASS64 = 2
PT_LOAD = 1
PF_X = 1
PF_W = 2
SHT_SYMTAB = 2
SHT_STRTAB = 3
STB_GLOBAL = 1


def align_up(val, align):
    return (val + align - 1) & ~(align - 1)


def pages(size):
    return align_up(size, PAGE_SIZE) // PAGE_SIZE


def parse_elf64(data):
    """Parse ELF64 header, program headers, and symbol table."""
    # ELF header
    if data[:4] != b'\x7fELF':
        raise ValueError("Not an ELF file")
    if data[EI_CLASS] != ELFCLASS64:
        raise ValueError("Not a 64-bit ELF")

    (e_type, e_machine, e_version, e_entry, e_phoff, e_shoff,
     e_flags, e_ehsize, e_phentsize, e_phnum,
     e_shentsize, e_shnum, e_shstrndx) = struct.unpack_from('<HHIQQQIHHHHHH', data, 16)

    # Program headers
    segments = []
    for i in range(e_phnum):
        off = e_phoff + i * e_phentsize
        (p_type, p_flags, p_offset, p_vaddr, p_paddr,
         p_filesz, p_memsz, p_align) = struct.unpack_from('<IIQQQQQQ', data, off)
        segments.append({
            'type': p_type, 'flags': p_flags, 'offset': p_offset,
            'vaddr': p_vaddr, 'filesz': p_filesz, 'memsz': p_memsz,
        })

    # Section headers (for symbol table)
    sections = []
    for i in range(e_shnum):
        off = e_shoff + i * e_shentsize
        (sh_name, sh_type, sh_flags, sh_addr, sh_offset,
         sh_size, sh_link, sh_info, sh_addralign, sh_entsize) = struct.unpack_from('<IIQQQQIIqq', data, off)
        sections.append({
            'name_off': sh_name, 'type': sh_type, 'flags': sh_flags,
            'addr': sh_addr, 'offset': sh_offset, 'size': sh_size,
            'link': sh_link, 'entsize': sh_entsize,
        })

    return e_entry, segments, sections


def find_symbol(data, sections, name):
    """Find a symbol by name in the ELF symbol table. Returns its value (address)."""
    for sec in sections:
        if sec['type'] != SHT_SYMTAB:
            continue
        strtab_sec = sections[sec['link']]
        strtab_data = data[strtab_sec['offset']:strtab_sec['offset'] + strtab_sec['size']]

        num_syms = sec['size'] // sec['entsize'] if sec['entsize'] else 0
        for i in range(num_syms):
            off = sec['offset'] + i * sec['entsize']
            (st_name, st_info, st_other, st_shndx,
             st_value, st_size) = struct.unpack_from('<IBBHQQ', data, off)
            if st_name < len(strtab_data):
                end = strtab_data.index(0, st_name) if 0 in strtab_data[st_name:] else len(strtab_data)
                sym_name = strtab_data[st_name:end].decode('ascii', errors='replace')
                if sym_name == name:
                    return st_value
    return None


def main():
    if len(sys.argv) < 3:
        print(f"Usage: {sys.argv[0]} input.elf output.kdrv [--exports-symbol NAME]")
        sys.exit(1)

    input_path = sys.argv[1]
    output_path = sys.argv[2]
    exports_symbol = 'DRIVER_EXPORTS'

    i = 3
    while i < len(sys.argv):
        if sys.argv[i] == '--exports-symbol' and i + 1 < len(sys.argv):
            exports_symbol = sys.argv[i + 1]
            i += 2
        else:
            i += 1

    with open(input_path, 'rb') as f:
        data = f.read()

    entry, segments, sections = parse_elf64(data)

    # Separate PT_LOAD segments into code (RX) and data (RW)
    load_segments = [s for s in segments if s['type'] == PT_LOAD]
    if not load_segments:
        print("ERROR: No PT_LOAD segments found", file=sys.stderr)
        sys.exit(1)

    # Sort by vaddr
    load_segments.sort(key=lambda s: s['vaddr'])

    # Base address = lowest vaddr (page-aligned)
    base_vaddr = load_segments[0]['vaddr'] & ~(PAGE_SIZE - 1)

    # Collect code (executable or read-only) and data (writable) regions
    code_data = bytearray()
    data_data = bytearray()
    code_size = 0
    data_size = 0
    bss_size = 0

    for seg in load_segments:
        seg_offset_from_base = seg['vaddr'] - base_vaddr
        seg_data = data[seg['offset']:seg['offset'] + seg['filesz']]
        seg_bss = seg['memsz'] - seg['filesz']

        if seg['flags'] & PF_W:
            # Data segment (writable)
            # Pad to align within data region
            while len(data_data) < (seg_offset_from_base - align_up(code_size, PAGE_SIZE)):
                data_data.append(0)
            data_data.extend(seg_data)
            data_size = len(data_data)
            bss_size = seg_bss
        else:
            # Code segment (read-only / executable)
            while len(code_data) < seg_offset_from_base:
                code_data.append(0)
            code_data.extend(seg_data)
            code_size = len(code_data)

    code_pages_count = pages(code_size) if code_size > 0 else 0
    data_pages_count = pages(data_size) if data_size > 0 else 0
    bss_pages_count = pages(bss_size) if bss_size > 0 else 0

    # Find exports symbol
    exports_addr = find_symbol(data, sections, exports_symbol)
    if exports_addr is None:
        print(f"WARNING: Symbol '{exports_symbol}' not found — exports_offset set to 0", file=sys.stderr)
        exports_offset = 0
    else:
        # exports_offset is relative to the load base (which starts after the header page)
        # Header is page 0, code starts at page 1
        # So offset from load_base = PAGE_SIZE (header) + (exports_addr - base_vaddr)
        exports_offset = PAGE_SIZE + (exports_addr - base_vaddr)

    # Build KDRV header (4096 bytes)
    header = bytearray(PAGE_SIZE)
    struct.pack_into('<4sIII', header, 0,
                     KDRV_MAGIC, KDRV_VERSION, KDRV_ABI_VERSION, 0)
    struct.pack_into('<QII I', header, 16,
                     exports_offset, code_pages_count, data_pages_count, bss_pages_count)

    # Pad code and data to page boundaries
    code_padded = code_data + bytes(code_pages_count * PAGE_SIZE - len(code_data)) if code_pages_count > 0 else b''
    data_padded = data_data + bytes(data_pages_count * PAGE_SIZE - len(data_data)) if data_pages_count > 0 else b''

    # Write output
    with open(output_path, 'wb') as f:
        f.write(bytes(header))
        f.write(bytes(code_padded))
        f.write(bytes(data_padded))
        # BSS is not written — kernel zeros those pages

    total_size = PAGE_SIZE + len(code_padded) + len(data_padded)
    print(f"elf2kdrv: {input_path} -> {output_path}")
    print(f"  base_vaddr: {base_vaddr:#x}")
    print(f"  code: {code_pages_count} pages ({code_size} bytes)")
    print(f"  data: {data_pages_count} pages ({data_size} bytes)")
    print(f"  bss:  {bss_pages_count} pages ({bss_size} bytes)")
    print(f"  exports_offset: {exports_offset:#x}")
    print(f"  total: {total_size} bytes")


if __name__ == '__main__':
    main()
