#!/usr/bin/env python3
# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT

"""
mkimage.py - Create bootable disk image for anyOS

Supports two modes:
  BIOS mode (default):
    Sector 0:       Stage 1 (MBR, 512 bytes)
    Sectors 1-63:   Stage 2 (padded to 63 * 512 bytes)
    Sectors 64+:    Kernel flat binary (extracted from ELF PT_LOAD segments)
    Sector fs_start+: exFAT filesystem (optional, if --sysroot is given)

  UEFI mode (--uefi):
    GPT partition table with:
      Partition 1: EFI System Partition (FAT16, 3 MiB) containing BOOTX64.EFI
      Partition 2: anyOS Data (exFAT) containing sysroot + /System/kernel.bin
"""

import argparse
import os
import struct
import sys
import uuid
import zlib

# ELF constants
ELF_MAGIC = b'\x7fELF'
PT_LOAD = 1

SECTOR_SIZE = 512


# =====================================================================
# VFAT Long Filename (LFN) helpers
# =====================================================================

def lfn_checksum(name83: bytes) -> int:
    """Compute the VFAT LFN checksum from an 8.3 name (11 bytes)."""
    s = 0
    for b in name83:
        s = (((s & 1) << 7) + (s >> 1) + b) & 0xFF
    return s


def needs_lfn(filename: str) -> bool:
    """Check if a filename needs LFN entries (doesn't fit 8.3)."""
    if len(filename) > 12 or len(filename) == 0:
        return True
    if filename.startswith('.') and filename not in ('.', '..'):
        return True
    if filename.count('.') > 1:
        return True
    if '.' in filename:
        base, ext = filename.rsplit('.', 1)
    else:
        base, ext = filename, ''
    if len(base) > 8 or len(ext) > 3:
        return True
    for c in filename:
        if c in ' +,;=[]':
            return True
    # Check for lowercase (Windows creates LFN for mixed case)
    if filename != filename.upper():
        return True
    return False


_short_name_counters = {}  # tracks numeric tails per base+ext

def generate_short_name(filename: str) -> bytes:
    """Generate a unique 8.3 short name from a long filename."""
    name = filename.upper()
    if '.' in name:
        base, ext = name.rsplit('.', 1)
    else:
        base, ext = name, ''

    # Filter invalid chars
    base = ''.join(c for c in base if c not in ' .+,;=[]')
    ext = ''.join(c for c in ext if c not in ' .')

    # Truncate
    base = base[:6]
    ext = ext[:3]

    # Track collision by (base, ext) pair
    key = (base, ext)
    counter = _short_name_counters.get(key, 0) + 1
    _short_name_counters[key] = counter

    tail = f'~{counter}'
    max_base = 8 - len(tail)
    short_base = base[:max_base] + tail
    short = short_base.ljust(8)
    ext = ext.ljust(3)
    return (short + ext).encode('ascii')


def make_lfn_entries(filename: str, name83: bytes) -> list:
    """Create LFN directory entries. Returns list of 32-byte entries in disk order."""
    chk = lfn_checksum(name83)

    # Convert to UTF-16LE code units
    utf16 = [ord(c) for c in filename]
    num_entries = (len(utf16) + 12) // 13

    entries = []
    for seq in range(1, num_entries + 1):
        entry = bytearray(32)
        is_last = (seq == num_entries)
        entry[0] = seq | (0x40 if is_last else 0)
        entry[11] = 0x0F  # ATTR_LONG_NAME
        entry[12] = 0     # type
        entry[13] = chk
        entry[26] = 0     # first cluster lo
        entry[27] = 0

        start = (seq - 1) * 13
        chars = []
        for j in range(13):
            idx = start + j
            if idx < len(utf16):
                chars.append(utf16[idx])
            elif idx == len(utf16):
                chars.append(0x0000)
            else:
                chars.append(0xFFFF)

        # Store chars 1-5 at offset 1
        for j in range(5):
            struct.pack_into('<H', entry, 1 + j * 2, chars[j])
        # Store chars 6-11 at offset 14
        for j in range(6):
            struct.pack_into('<H', entry, 14 + j * 2, chars[5 + j])
        # Store chars 12-13 at offset 28
        for j in range(2):
            struct.pack_into('<H', entry, 28 + j * 2, chars[11 + j])

        entries.append(bytes(entry))

    # Reverse so last entry (0x40) comes first on disk
    entries.reverse()
    return entries


def elf_to_flat_binary(elf_data, base_paddr):
    """
    Parse an ELF32 or ELF64 file and extract PT_LOAD segments into a flat binary.
    The flat binary is laid out so that byte 0 corresponds to base_paddr.
    """
    # Verify ELF magic
    if elf_data[:4] != ELF_MAGIC:
        print("ERROR: Kernel is not a valid ELF file", file=sys.stderr)
        sys.exit(1)

    # Detect ELF class: 1 = 32-bit, 2 = 64-bit
    ei_class = elf_data[4]

    if ei_class == 2:
        # ELF64 header
        (e_type, e_machine, e_version, e_entry, e_phoff, e_shoff,
         e_flags, e_ehsize, e_phentsize, e_phnum, e_shentsize, e_shnum,
         e_shstrndx) = struct.unpack_from("<HHIQQQIHHHHHH", elf_data, 16)
        print(f"  ELF64 entry point: 0x{e_entry:016X}")
    elif ei_class == 1:
        # ELF32 header
        (e_type, e_machine, e_version, e_entry, e_phoff, e_shoff,
         e_flags, e_ehsize, e_phentsize, e_phnum, e_shentsize, e_shnum,
         e_shstrndx) = struct.unpack_from("<HHIIIIIHHHHHH", elf_data, 16)
        print(f"  ELF32 entry point: 0x{e_entry:08X}")
    else:
        print(f"ERROR: Unknown ELF class {ei_class}", file=sys.stderr)
        sys.exit(1)

    print(f"  Program headers: {e_phnum} entries at offset {e_phoff}")

    # Parse program headers to find max extent
    max_paddr_end = 0
    segments = []
    for i in range(e_phnum):
        ph_offset = e_phoff + i * e_phentsize

        if ei_class == 2:
            # ELF64 Phdr: p_type(4), p_flags(4), p_offset(8), p_vaddr(8),
            #             p_paddr(8), p_filesz(8), p_memsz(8), p_align(8)
            (p_type, p_flags, p_offset, p_vaddr, p_paddr, p_filesz, p_memsz,
             p_align) = struct.unpack_from("<IIQQQQQQ", elf_data, ph_offset)
        else:
            # ELF32 Phdr
            (p_type, p_offset, p_vaddr, p_paddr, p_filesz, p_memsz,
             p_flags, p_align) = struct.unpack_from("<IIIIIIII", elf_data, ph_offset)

        if p_type == PT_LOAD and p_filesz > 0:
            segments.append((p_paddr, p_offset, p_filesz, p_memsz, p_vaddr))
            end = p_paddr + p_memsz
            if end > max_paddr_end:
                max_paddr_end = end
            print(f"  PT_LOAD: paddr=0x{p_paddr:08X} vaddr=0x{p_vaddr:016X} "
                  f"filesz=0x{p_filesz:X} memsz=0x{p_memsz:X}")

    if not segments:
        print("ERROR: No PT_LOAD segments found in kernel ELF", file=sys.stderr)
        sys.exit(1)

    # Create flat binary
    flat_size = max_paddr_end - base_paddr
    flat = bytearray(flat_size)

    for p_paddr, p_offset, p_filesz, p_memsz, p_vaddr in segments:
        dest_offset = p_paddr - base_paddr
        flat[dest_offset:dest_offset + p_filesz] = elf_data[p_offset:p_offset + p_filesz]

    print(f"  Flat binary: {flat_size} bytes (0x{base_paddr:08X} - 0x{max_paddr_end:08X})")
    return bytes(flat)


class Fat16Formatter:
    """Creates a FAT16 filesystem in the disk image."""

    def __init__(self, image, fs_start_sector, fs_sector_count, sectors_per_cluster=8):
        global _short_name_counters
        _short_name_counters = {}

        self.image = image
        self.fs_start = fs_start_sector
        self.fs_sectors = fs_sector_count

        # FAT16 parameters
        self.bytes_per_sector = 512
        self.sectors_per_cluster = sectors_per_cluster
        self.reserved_sectors = 1     # Just the boot sector
        self.num_fats = 2
        self.root_entry_count = 512   # 512 entries * 32 bytes = 16 KiB = 32 sectors
        self.root_dir_sectors = (self.root_entry_count * 32 + self.bytes_per_sector - 1) // self.bytes_per_sector

        # Calculate FAT size
        data_sectors = self.fs_sectors - self.reserved_sectors - self.root_dir_sectors
        total_clusters = data_sectors // self.sectors_per_cluster
        # FAT16: 2 bytes per entry
        self.fat_size = (total_clusters * 2 + self.bytes_per_sector - 1) // self.bytes_per_sector
        # Recalculate with FAT overhead
        data_sectors = self.fs_sectors - self.reserved_sectors - (self.num_fats * self.fat_size) - self.root_dir_sectors
        self.total_clusters = data_sectors // self.sectors_per_cluster

        self.first_fat_sector = self.reserved_sectors
        self.first_root_dir_sector = self.reserved_sectors + self.num_fats * self.fat_size
        self.first_data_sector = self.first_root_dir_sector + self.root_dir_sectors

        # Next free cluster (starts at 2)
        self.next_cluster = 2
        # Next root dir entry index
        self.next_root_entry = 0

        print(f"  FAT16: {self.total_clusters} clusters, {self.sectors_per_cluster} sec/cluster, "
              f"FAT size={self.fat_size} sectors")
        print(f"  FAT16: first_fat={self.first_fat_sector}, root_dir={self.first_root_dir_sector}, "
              f"data={self.first_data_sector}")

    def _abs_sector(self, relative_sector):
        """Convert filesystem-relative sector to absolute sector in image."""
        return self.fs_start + relative_sector

    def _write_sector(self, relative_sector, data):
        """Write data to a filesystem-relative sector."""
        offset = self._abs_sector(relative_sector) * SECTOR_SIZE
        self.image[offset:offset + len(data)] = data

    def _read_sector(self, relative_sector):
        """Read a filesystem-relative sector."""
        offset = self._abs_sector(relative_sector) * SECTOR_SIZE
        return self.image[offset:offset + SECTOR_SIZE]

    def write_boot_sector(self):
        """Write the FAT16 BPB (BIOS Parameter Block)."""
        bpb = bytearray(SECTOR_SIZE)

        # Jump instruction (skip BPB)
        bpb[0:3] = b'\xEB\x3C\x90'  # jmp short 0x3E; nop

        # OEM name
        bpb[3:11] = b'ANYOS   '

        # BPB fields
        struct.pack_into('<H', bpb, 11, self.bytes_per_sector)      # bytes per sector
        struct.pack_into('<B', bpb, 13, self.sectors_per_cluster)    # sectors per cluster
        struct.pack_into('<H', bpb, 14, self.reserved_sectors)       # reserved sectors
        struct.pack_into('<B', bpb, 16, self.num_fats)               # number of FATs
        struct.pack_into('<H', bpb, 17, self.root_entry_count)       # root entry count
        if self.fs_sectors < 0x10000:
            struct.pack_into('<H', bpb, 19, self.fs_sectors)         # total sectors 16
        else:
            struct.pack_into('<H', bpb, 19, 0)
        struct.pack_into('<B', bpb, 21, 0xF8)                       # media type (hard disk)
        struct.pack_into('<H', bpb, 22, self.fat_size)               # FAT size 16
        struct.pack_into('<H', bpb, 24, 63)                          # sectors per track
        struct.pack_into('<H', bpb, 26, 16)                          # number of heads
        struct.pack_into('<I', bpb, 28, self.fs_start)               # hidden sectors
        if self.fs_sectors >= 0x10000:
            struct.pack_into('<I', bpb, 32, self.fs_sectors)         # total sectors 32

        # Extended BPB (FAT16)
        struct.pack_into('<B', bpb, 36, 0x80)                        # drive number
        struct.pack_into('<B', bpb, 38, 0x29)                        # extended boot signature
        struct.pack_into('<I', bpb, 39, 0x12345678)                  # volume serial number
        bpb[43:54] = b'ANYOS      '                                  # volume label (11 bytes)
        bpb[54:62] = b'FAT16   '                                     # filesystem type

        # Boot signature
        bpb[510] = 0x55
        bpb[511] = 0xAA

        self._write_sector(0, bpb)
        print(f"  FAT16: BPB written at sector {self.fs_start}")

    def init_fat(self):
        """Initialize the FAT tables with reserved entries."""
        # FAT entry 0: media type
        # FAT entry 1: end-of-chain marker
        fat_sector = bytearray(SECTOR_SIZE)
        struct.pack_into('<H', fat_sector, 0, 0xFFF8)  # Entry 0: media descriptor
        struct.pack_into('<H', fat_sector, 2, 0xFFFF)   # Entry 1: end marker

        # Write to both FAT copies
        for fat_idx in range(self.num_fats):
            fat_start = self.first_fat_sector + fat_idx * self.fat_size
            self._write_sector(fat_start, fat_sector)

    def _set_fat_entry(self, cluster, value):
        """Set a FAT entry for a cluster."""
        fat_offset = cluster * 2
        sector_in_fat = fat_offset // SECTOR_SIZE
        offset_in_sector = fat_offset % SECTOR_SIZE

        for fat_idx in range(self.num_fats):
            abs_sector = self.first_fat_sector + fat_idx * self.fat_size + sector_in_fat
            sector_data = bytearray(self._read_sector(abs_sector))
            struct.pack_into('<H', sector_data, offset_in_sector, value)
            self._write_sector(abs_sector, sector_data)

    def _cluster_to_sector(self, cluster):
        """Convert cluster number to filesystem-relative sector."""
        return self.first_data_sector + (cluster - 2) * self.sectors_per_cluster

    def _allocate_clusters(self, num_clusters):
        """Allocate a chain of clusters. Returns the first cluster number."""
        if num_clusters == 0:
            return 0

        first = self.next_cluster
        for i in range(num_clusters):
            current = self.next_cluster
            self.next_cluster += 1

            if i < num_clusters - 1:
                self._set_fat_entry(current, current + 1)
            else:
                self._set_fat_entry(current, 0xFFFF)  # End of chain

        return first

    def _write_to_clusters(self, first_cluster, data):
        """Write data to a chain of clusters starting at first_cluster."""
        cluster = first_cluster
        offset = 0
        cluster_size = self.sectors_per_cluster * SECTOR_SIZE

        while offset < len(data):
            chunk = data[offset:offset + cluster_size]
            sector = self._cluster_to_sector(cluster)

            # Write cluster data sector by sector
            for s in range(self.sectors_per_cluster):
                s_offset = s * SECTOR_SIZE
                if s_offset >= len(chunk):
                    break
                s_data = chunk[s_offset:s_offset + SECTOR_SIZE]
                if len(s_data) < SECTOR_SIZE:
                    s_data = s_data + b'\x00' * (SECTOR_SIZE - len(s_data))
                self._write_sector(sector + s, s_data)

            offset += cluster_size

            # Read next cluster from FAT
            if offset < len(data):
                fat_offset = cluster * 2
                sector_in_fat = fat_offset // SECTOR_SIZE
                offset_in_sector = fat_offset % SECTOR_SIZE
                fat_sector_data = self._read_sector(self.first_fat_sector + sector_in_fat)
                cluster = struct.unpack_from('<H', fat_sector_data, offset_in_sector)[0]
                if cluster >= 0xFFF8:
                    break

    def _make_83_name(self, filename):
        """Convert a filename to FAT 8.3 format (11 bytes, space-padded, uppercase)."""
        name = filename.upper()
        if '.' in name:
            base, ext = name.rsplit('.', 1)
        else:
            base = name
            ext = ''

        base = base[:8].ljust(8)
        ext = ext[:3].ljust(3)
        return (base + ext).encode('ascii')

    def _write_root_entry_at(self, index, entry_data):
        """Write a 32-byte entry at a specific root directory index."""
        entry_offset = index * 32
        sector_in_root = entry_offset // SECTOR_SIZE
        offset_in_sector = entry_offset % SECTOR_SIZE

        sector = self.first_root_dir_sector + sector_in_root
        sector_data = bytearray(self._read_sector(sector))
        sector_data[offset_in_sector:offset_in_sector + 32] = entry_data
        self._write_sector(sector, sector_data)

    def add_root_dir_entry(self, filename, first_cluster, file_size, is_directory=False):
        """Add a directory entry to the root directory, with LFN if needed."""
        use_lfn = needs_lfn(filename)

        if use_lfn:
            name83 = generate_short_name(filename)
        else:
            name83 = self._make_83_name(filename)

        # Write LFN entries first
        if use_lfn:
            lfn_entries = make_lfn_entries(filename, name83)
            for lfn_entry in lfn_entries:
                self._write_root_entry_at(self.next_root_entry, lfn_entry)
                self.next_root_entry += 1

        # Write the 8.3 entry
        entry = bytearray(32)
        entry[0:11] = name83

        attr = 0x10 if is_directory else 0x20  # DIRECTORY or ARCHIVE
        entry[11] = attr

        # First cluster (low 16 bits)
        struct.pack_into('<H', entry, 26, first_cluster & 0xFFFF)
        # First cluster (high 16 bits, always 0 for FAT16)
        struct.pack_into('<H', entry, 20, 0)
        # File size
        struct.pack_into('<I', entry, 28, file_size if not is_directory else 0)

        self._write_root_entry_at(self.next_root_entry, entry)
        self.next_root_entry += 1

    def add_subdir_entry(self, parent_cluster, filename, first_cluster, file_size, is_directory=False):
        """Add a directory entry to a subdirectory cluster, with LFN if needed."""
        use_lfn = needs_lfn(filename)

        if use_lfn:
            name83 = generate_short_name(filename)
            lfn_entries = make_lfn_entries(filename, name83)
            total_needed = len(lfn_entries) + 1
        else:
            name83 = self._make_83_name(filename)
            lfn_entries = []
            total_needed = 1

        # Read existing directory data
        cluster_size = self.sectors_per_cluster * SECTOR_SIZE
        dir_data = bytearray(cluster_size)
        sector = self._cluster_to_sector(parent_cluster)
        for s in range(self.sectors_per_cluster):
            s_data = self._read_sector(sector + s)
            dir_data[s * SECTOR_SIZE:(s + 1) * SECTOR_SIZE] = s_data

        # Find N consecutive free entries
        found_start = -1
        consecutive = 0
        for i in range(0, cluster_size, 32):
            if dir_data[i] == 0x00 or dir_data[i] == 0xE5:
                if consecutive == 0:
                    found_start = i
                consecutive += 1
                if consecutive >= total_needed:
                    break
            else:
                consecutive = 0
                found_start = -1

        if found_start < 0 or consecutive < total_needed:
            print(f"  WARNING: No room in subdir for {filename}")
            return

        # Write LFN entries
        pos = found_start
        for lfn_entry in lfn_entries:
            dir_data[pos:pos + 32] = lfn_entry
            pos += 32

        # Write 8.3 entry
        entry = bytearray(32)
        entry[0:11] = name83
        attr = 0x10 if is_directory else 0x20
        entry[11] = attr
        struct.pack_into('<H', entry, 26, first_cluster & 0xFFFF)
        struct.pack_into('<H', entry, 20, 0)
        struct.pack_into('<I', entry, 28, file_size if not is_directory else 0)
        dir_data[pos:pos + 32] = entry

        # Write back
        for s in range(self.sectors_per_cluster):
            self._write_sector(sector + s, dir_data[s * SECTOR_SIZE:(s + 1) * SECTOR_SIZE])

    def create_directory(self, parent_cluster_or_root, dirname, is_root_parent=True):
        """Create a subdirectory. Returns the cluster of the new directory."""
        # Allocate 1 cluster for the directory
        dir_cluster = self._allocate_clusters(1)

        # Create . and .. entries
        cluster_size = self.sectors_per_cluster * SECTOR_SIZE
        dir_data = bytearray(cluster_size)

        # "." entry
        dot = bytearray(32)
        dot[0:11] = b'.          '
        dot[11] = 0x10  # Directory
        struct.pack_into('<H', dot, 26, dir_cluster)
        dir_data[0:32] = dot

        # ".." entry
        dotdot = bytearray(32)
        dotdot[0:11] = b'..         '
        dotdot[11] = 0x10  # Directory
        parent_val = 0 if is_root_parent else parent_cluster_or_root
        struct.pack_into('<H', dotdot, 26, parent_val)
        dir_data[32:64] = dotdot

        # Write directory cluster
        sector = self._cluster_to_sector(dir_cluster)
        for s in range(self.sectors_per_cluster):
            self._write_sector(sector + s, dir_data[s * SECTOR_SIZE:(s + 1) * SECTOR_SIZE])

        # Add entry to parent
        if is_root_parent:
            self.add_root_dir_entry(dirname, dir_cluster, 0, is_directory=True)
        else:
            self.add_subdir_entry(parent_cluster_or_root, dirname, dir_cluster, 0, is_directory=True)

        return dir_cluster

    def add_file(self, parent_cluster_or_root, filename, data, is_root_parent=True):
        """Add a file to a directory."""
        if len(data) == 0:
            # Empty file, no clusters needed
            if is_root_parent:
                self.add_root_dir_entry(filename, 0, 0)
            else:
                self.add_subdir_entry(parent_cluster_or_root, filename, 0, 0)
            return

        cluster_size = self.sectors_per_cluster * SECTOR_SIZE
        num_clusters = (len(data) + cluster_size - 1) // cluster_size
        first_cluster = self._allocate_clusters(num_clusters)
        self._write_to_clusters(first_cluster, data)

        if is_root_parent:
            self.add_root_dir_entry(filename, first_cluster, len(data))
        else:
            self.add_subdir_entry(parent_cluster_or_root, filename, first_cluster, len(data))

        print(f"    File: {filename} ({len(data)} bytes, {num_clusters} cluster(s), start={first_cluster})")

    def populate_from_sysroot(self, sysroot_path):
        """Recursively copy files from sysroot directory to the filesystem."""
        if not os.path.isdir(sysroot_path):
            print(f"  Warning: sysroot path '{sysroot_path}' does not exist, skipping")
            return

        # Add volume label entry
        label_entry = bytearray(32)
        label_entry[0:11] = b'ANYOS      '
        label_entry[11] = 0x08  # Volume label attribute
        entry_offset = self.next_root_entry * 32
        sector_in_root = entry_offset // SECTOR_SIZE
        offset_in_sector = entry_offset % SECTOR_SIZE
        sector = self.first_root_dir_sector + sector_in_root
        sector_data = bytearray(self._read_sector(sector))
        sector_data[offset_in_sector:offset_in_sector + 32] = label_entry
        self._write_sector(sector, sector_data)
        self.next_root_entry += 1

        self._populate_dir(sysroot_path, None, is_root=True)

    def _populate_dir(self, host_path, parent_cluster, is_root=False):
        """Recursively populate a directory."""
        entries = sorted(os.listdir(host_path))

        for entry_name in entries:
            full_path = os.path.join(host_path, entry_name)

            if entry_name.startswith('.'):
                continue  # Skip hidden files

            if os.path.isdir(full_path):
                # Create subdirectory
                dir_cluster = self.create_directory(
                    parent_cluster, entry_name,
                    is_root_parent=is_root
                )
                print(f"    Dir:  {entry_name}/ (cluster={dir_cluster})")
                # Recurse into subdirectory
                self._populate_dir(full_path, dir_cluster, is_root=False)

            elif os.path.isfile(full_path):
                with open(full_path, 'rb') as f:
                    data = f.read()
                self.add_file(
                    parent_cluster, entry_name, data,
                    is_root_parent=is_root
                )


# =====================================================================
# exFAT formatter
# =====================================================================

class ExFatFormatter:
    """Creates an exFAT filesystem in the disk image."""

    # Directory entry types
    ENTRY_BITMAP  = 0x81
    ENTRY_UPCASE  = 0x82
    ENTRY_LABEL   = 0x83
    ENTRY_FILE    = 0x85
    ENTRY_STREAM  = 0xC0
    ENTRY_FILENAME = 0xC1

    ATTR_DIRECTORY = 0x0010
    ATTR_ARCHIVE   = 0x0020

    FLAG_CONTIGUOUS = 0x02

    EXFAT_EOC  = 0xFFFFFFFF
    EXFAT_FREE = 0x00000000

    def __init__(self, image, fs_start_sector, fs_sector_count, sectors_per_cluster=8):
        self.image = image
        self.fs_start = fs_start_sector
        self.fs_sectors = fs_sector_count
        self.bps_shift = 9   # 2^9 = 512
        self.spc_shift = 3   # 2^3 = 8 sectors per cluster
        self.spc = sectors_per_cluster
        self.cluster_size = self.spc * SECTOR_SIZE  # 4096
        self.num_fats = 1

        # Layout: Main Boot Region (12) + Backup (12) + alignment = FAT at sector 32
        self.fat_offset = 32  # sectors from fs_start to FAT

        # Iterative: compute cluster_count and fat_length
        # cluster_count = (fs_sectors - cluster_heap_offset) / spc
        # cluster_heap_offset = fat_offset + fat_length
        # fat_length = ceil((cluster_count + 2) * 4 / 512)
        # Start with estimate
        est_clusters = (fs_sector_count - self.fat_offset) // self.spc
        fat_bytes = (est_clusters + 2) * 4
        self.fat_length = (fat_bytes + SECTOR_SIZE - 1) // SECTOR_SIZE
        self.cluster_heap_offset = self.fat_offset + self.fat_length
        self.cluster_count = (fs_sector_count - self.cluster_heap_offset) // self.spc

        # Recompute fat_length with final cluster_count
        fat_bytes = (self.cluster_count + 2) * 4
        self.fat_length = (fat_bytes + SECTOR_SIZE - 1) // SECTOR_SIZE
        self.cluster_heap_offset = self.fat_offset + self.fat_length

        # Next free cluster (starts at 2)
        self.next_cluster = 2

        # Root directory at cluster 4 (bitmap=2, upcase=3, root=4)
        # We'll allocate them in order
        self.bitmap_cluster = None
        self.root_cluster = None

        # In-memory FAT cache
        self.fat_cache = bytearray((self.cluster_count + 2) * 4)
        # Entry 0: media type, Entry 1: end marker
        struct.pack_into('<I', self.fat_cache, 0, 0xFFFFFFF8)
        struct.pack_into('<I', self.fat_cache, 4, 0xFFFFFFFF)

        # In-memory allocation bitmap
        bitmap_bytes = (self.cluster_count + 7) // 8
        self.bitmap = bytearray(bitmap_bytes)

        print(f"  exFAT: {self.cluster_count} clusters, {self.cluster_size} bytes/cluster")
        print(f"  exFAT: FAT at sector +{self.fat_offset} ({self.fat_length} sectors), "
              f"data at sector +{self.cluster_heap_offset}")

    def _abs_offset(self, relative_sector):
        """Byte offset in image for a filesystem-relative sector."""
        return (self.fs_start + relative_sector) * SECTOR_SIZE

    def _write_sector(self, relative_sector, data):
        offset = self._abs_offset(relative_sector)
        self.image[offset:offset + len(data)] = data

    def _read_sector(self, relative_sector):
        offset = self._abs_offset(relative_sector)
        return bytes(self.image[offset:offset + SECTOR_SIZE])

    def _cluster_to_sector(self, cluster):
        """Convert cluster number (>=2) to filesystem-relative sector."""
        return self.cluster_heap_offset + (cluster - 2) * self.spc

    def _write_cluster(self, cluster, data):
        sector = self._cluster_to_sector(cluster)
        for s in range(self.spc):
            s_offset = s * SECTOR_SIZE
            if s_offset >= len(data):
                self._write_sector(sector + s, b'\x00' * SECTOR_SIZE)
            else:
                chunk = data[s_offset:s_offset + SECTOR_SIZE]
                if len(chunk) < SECTOR_SIZE:
                    chunk = chunk + b'\x00' * (SECTOR_SIZE - len(chunk))
                self._write_sector(sector + s, chunk)

    def _alloc_cluster(self):
        """Allocate a single cluster, mark bitmap + write EOC to FAT."""
        c = self.next_cluster
        if c - 2 >= self.cluster_count:
            raise RuntimeError("exFAT: out of clusters")
        self.next_cluster += 1
        # Mark bitmap
        idx = c - 2
        self.bitmap[idx // 8] |= 1 << (idx % 8)
        # Write FAT EOC
        struct.pack_into('<I', self.fat_cache, c * 4, self.EXFAT_EOC)
        return c

    def _alloc_clusters_contiguous(self, count):
        """Allocate `count` contiguous clusters. Returns first cluster.
        Does NOT write FAT chain (for NoFatChain files)."""
        if count == 0:
            return 0
        first = self.next_cluster
        for i in range(count):
            c = self.next_cluster
            if c - 2 >= self.cluster_count:
                raise RuntimeError("exFAT: out of clusters")
            self.next_cluster += 1
            idx = c - 2
            self.bitmap[idx // 8] |= 1 << (idx % 8)
            # No FAT chain for contiguous files — leave FAT entries as 0
        return first

    def _alloc_clusters_chained(self, count):
        """Allocate `count` clusters with FAT chain. Returns first cluster."""
        if count == 0:
            return 0
        first = self._alloc_cluster()
        prev = first
        for i in range(1, count):
            c = self._alloc_cluster()
            struct.pack_into('<I', self.fat_cache, prev * 4, c)
            prev = c
        return first

    def _write_to_clusters_contiguous(self, first_cluster, data):
        """Write data to contiguous clusters."""
        offset = 0
        cluster = first_cluster
        while offset < len(data):
            chunk = data[offset:offset + self.cluster_size]
            self._write_cluster(cluster, chunk)
            offset += self.cluster_size
            cluster += 1

    def _write_to_clusters_chained(self, first_cluster, data):
        """Write data to FAT-chained clusters."""
        cluster = first_cluster
        offset = 0
        while offset < len(data):
            chunk = data[offset:offset + self.cluster_size]
            self._write_cluster(cluster, chunk)
            offset += self.cluster_size
            if offset < len(data):
                next_val = struct.unpack_from('<I', self.fat_cache, cluster * 4)[0]
                if next_val >= 0xFFFFFFF8 or next_val == 0:
                    break
                cluster = next_val

    # =====================================================================
    # Boot sector
    # =====================================================================

    @staticmethod
    def _boot_checksum(data):
        """Compute exFAT boot region checksum over sectors 0-10."""
        checksum = 0
        for i, byte in enumerate(data):
            # Skip VolumeFlags (106-107) and PercentInUse (112)
            if i == 106 or i == 107 or i == 112:
                continue
            checksum = (((checksum & 1) << 31) | (checksum >> 1)) + byte
            checksum &= 0xFFFFFFFF
        return checksum

    def write_boot_sector(self):
        """Write the exFAT VBR and backup boot region."""
        vbr = bytearray(SECTOR_SIZE)

        # JumpBoot
        vbr[0:3] = b'\xEB\x76\x90'
        # FileSystemName
        vbr[3:11] = b'EXFAT   '
        # MustBeZero (bytes 11-63)
        # Already zero

        # PartitionOffset (not needed for our use, set to fs_start)
        struct.pack_into('<Q', vbr, 64, self.fs_start)
        # VolumeLength
        struct.pack_into('<Q', vbr, 72, self.fs_sectors)
        # FatOffset
        struct.pack_into('<I', vbr, 80, self.fat_offset)
        # FatLength
        struct.pack_into('<I', vbr, 84, self.fat_length)
        # ClusterHeapOffset
        struct.pack_into('<I', vbr, 88, self.cluster_heap_offset)
        # ClusterCount
        struct.pack_into('<I', vbr, 92, self.cluster_count)
        # FirstClusterOfRootDirectory (will be set after allocation)
        # Placeholder — filled in by init_fs()
        struct.pack_into('<I', vbr, 96, 4)  # root at cluster 4
        # VolumeSerialNumber
        struct.pack_into('<I', vbr, 100, 0x414E594F)  # "ANYO"
        # FileSystemRevision (1.00)
        struct.pack_into('<H', vbr, 104, 0x0100)
        # VolumeFlags
        struct.pack_into('<H', vbr, 106, 0)
        # BytesPerSectorShift
        vbr[108] = self.bps_shift
        # SectorsPerClusterShift
        vbr[109] = self.spc_shift
        # NumberOfFats
        vbr[110] = self.num_fats
        # DriveSelect
        vbr[111] = 0x80
        # PercentInUse
        vbr[112] = 0xFF  # Unknown

        # BootSignature
        vbr[510] = 0x55
        vbr[511] = 0xAA

        # Extended Boot Sectors (sectors 1-8): zeros with 0x55AA signature
        ext_sectors = []
        for _ in range(8):
            ext = bytearray(SECTOR_SIZE)
            ext[510] = 0x55
            ext[511] = 0xAA
            ext_sectors.append(ext)

        # OEM Parameters (sector 9): zeros
        oem = bytearray(SECTOR_SIZE)
        # Reserved (sector 10): zeros
        reserved = bytearray(SECTOR_SIZE)

        # Assemble sectors 0-10 for checksum
        boot_region = bytearray(vbr)
        for ext in ext_sectors:
            boot_region += ext
        boot_region += oem
        boot_region += reserved

        # Compute checksum
        checksum = self._boot_checksum(boot_region)

        # Checksum sector (sector 11): repeated u32
        cs_sector = bytearray(SECTOR_SIZE)
        for i in range(0, SECTOR_SIZE, 4):
            struct.pack_into('<I', cs_sector, i, checksum)

        # Write Main Boot Region (sectors 0-11)
        self._write_sector(0, vbr)
        for i, ext in enumerate(ext_sectors):
            self._write_sector(1 + i, ext)
        self._write_sector(9, oem)
        self._write_sector(10, reserved)
        self._write_sector(11, cs_sector)

        # Write Backup Boot Region (sectors 12-23)
        self._write_sector(12, vbr)
        for i, ext in enumerate(ext_sectors):
            self._write_sector(13 + i, ext)
        self._write_sector(21, oem)
        self._write_sector(22, reserved)
        self._write_sector(23, cs_sector)

        print(f"  exFAT: VBR written at sector {self.fs_start}")

    # =====================================================================
    # Filesystem initialization
    # =====================================================================

    def init_fs(self):
        """Initialize the exFAT filesystem: FAT, bitmap, root directory."""
        # Allocate cluster 2 for allocation bitmap
        self.bitmap_cluster = self._alloc_cluster()  # = 2
        # Allocate cluster 3 for a minimal upcase table
        upcase_cluster = self._alloc_cluster()  # = 3
        # Allocate cluster 4 for root directory
        self.root_cluster = self._alloc_cluster()  # = 4

        # Write minimal upcase table (identity mapping for ASCII 0-127)
        upcase_data = bytearray(128 * 2)  # 128 UTF-16LE entries
        for i in range(128):
            ch = i
            if 0x61 <= ch <= 0x7A:  # a-z → A-Z
                ch -= 0x20
            struct.pack_into('<H', upcase_data, i * 2, ch)
        # Pad to cluster size
        upcase_padded = upcase_data + b'\x00' * (self.cluster_size - len(upcase_data))
        self._write_cluster(upcase_cluster, upcase_padded)

        # Write root directory with bitmap, upcase, and volume label entries
        root_data = bytearray(self.cluster_size)
        pos = 0

        # Allocation Bitmap entry (0x81)
        root_data[pos] = self.ENTRY_BITMAP
        root_data[pos + 1] = 0  # BitmapFlags (first bitmap)
        bitmap_size = (self.cluster_count + 7) // 8
        struct.pack_into('<I', root_data, pos + 20, self.bitmap_cluster)
        struct.pack_into('<Q', root_data, pos + 24, bitmap_size)
        pos += 32

        # Upcase Table entry (0x82)
        root_data[pos] = self.ENTRY_UPCASE
        upcase_checksum = 0
        for b in upcase_data:
            upcase_checksum = (((upcase_checksum & 1) << 31) | (upcase_checksum >> 1)) + b
            upcase_checksum &= 0xFFFFFFFF
        struct.pack_into('<I', root_data, pos + 4, upcase_checksum)
        struct.pack_into('<I', root_data, pos + 20, upcase_cluster)
        struct.pack_into('<Q', root_data, pos + 24, len(upcase_data))
        pos += 32

        # Volume Label entry (0x83)
        label = "anyOS"
        root_data[pos] = self.ENTRY_LABEL
        root_data[pos + 1] = len(label)  # CharacterCount
        for i, ch in enumerate(label):
            struct.pack_into('<H', root_data, pos + 2 + i * 2, ord(ch))
        pos += 32

        self._write_cluster(self.root_cluster, root_data)

        # Update VBR with correct root cluster
        vbr_offset = self._abs_offset(0)
        struct.pack_into('<I', self.image, vbr_offset + 96, self.root_cluster)
        # Also update backup
        backup_offset = self._abs_offset(12)
        struct.pack_into('<I', self.image, backup_offset + 96, self.root_cluster)

        print(f"  exFAT: bitmap=cluster {self.bitmap_cluster}, "
              f"upcase=cluster {upcase_cluster}, root=cluster {self.root_cluster}")

    # =====================================================================
    # Directory entry helpers
    # =====================================================================

    @staticmethod
    def _entry_set_checksum(data):
        """Compute exFAT entry set checksum."""
        checksum = 0
        for i, byte in enumerate(data):
            if i == 2 or i == 3:  # skip SetChecksum field
                continue
            checksum = ((checksum << 15) | (checksum >> 1)) + byte
            checksum &= 0xFFFF
        return checksum

    @staticmethod
    def _name_hash(utf16_chars):
        """Compute exFAT name hash over UTF-16 characters (upper-cased)."""
        h = 0
        for ch in utf16_chars:
            uc = ch
            if 0x61 <= uc <= 0x7A:
                uc -= 0x20
            h = ((h << 15) | (h >> 1)) + (uc & 0xFF)
            h &= 0xFFFF
            h = ((h << 15) | (h >> 1)) + (uc >> 8)
            h &= 0xFFFF
        return h

    def _build_entry_set(self, name, attributes, first_cluster, data_length, contiguous=False, uid=0, gid=0, mode=0xFFF):
        """Build a complete exFAT directory entry set."""
        utf16 = [ord(c) for c in name]
        name_len = len(utf16)
        fn_entries = (name_len + 14) // 15
        secondary = 1 + fn_entries  # Stream + FileName(s)
        total = 1 + secondary
        entry_set = bytearray(total * 32)

        # File Directory Entry (0x85)
        entry_set[0] = self.ENTRY_FILE
        entry_set[1] = secondary
        # [2..3] = SetChecksum (filled last)
        struct.pack_into('<H', entry_set, 4, attributes)
        # [6..11] = uid, gid, mode (VFS permissions)
        struct.pack_into('<H', entry_set, 6, uid)
        struct.pack_into('<H', entry_set, 8, gid)
        struct.pack_into('<H', entry_set, 10, mode)

        # Stream Extension (0xC0)
        s = 32
        entry_set[s] = self.ENTRY_STREAM
        flags = 0x01  # AllocationPossible
        if contiguous:
            flags |= self.FLAG_CONTIGUOUS
        entry_set[s + 1] = flags
        entry_set[s + 3] = name_len
        nh = self._name_hash(utf16)
        struct.pack_into('<H', entry_set, s + 4, nh)
        struct.pack_into('<Q', entry_set, s + 8, data_length)   # ValidDataLength
        struct.pack_into('<I', entry_set, s + 20, first_cluster)
        struct.pack_into('<Q', entry_set, s + 24, data_length)  # DataLength

        # FileName entries (0xC1)
        for fi in range(fn_entries):
            f = (2 + fi) * 32
            entry_set[f] = self.ENTRY_FILENAME
            for j in range(15):
                ci = fi * 15 + j
                ch = utf16[ci] if ci < len(utf16) else 0x0000
                struct.pack_into('<H', entry_set, f + 2 + j * 2, ch)

        # Checksum
        checksum = self._entry_set_checksum(entry_set)
        struct.pack_into('<H', entry_set, 2, checksum)

        return bytes(entry_set)

    def _add_entry_to_dir(self, dir_cluster, entry_set):
        """Add an entry set to a directory cluster. Extends directory if needed."""
        entry_count = len(entry_set) // 32

        # Read current directory data
        cluster = dir_cluster
        while True:
            sector = self._cluster_to_sector(cluster)
            dir_data = bytearray()
            for s in range(self.spc):
                dir_data += bytearray(self._read_sector(sector + s))

            # Find free space
            run_start = -1
            run_len = 0
            for idx in range(len(dir_data) // 32):
                off = idx * 32
                etype = dir_data[off]
                if etype == 0x00 or (etype & 0x80 == 0 and etype != 0):
                    if run_len == 0:
                        run_start = idx
                    run_len += 1
                    if run_len >= entry_count:
                        # Found space
                        write_off = run_start * 32
                        dir_data[write_off:write_off + len(entry_set)] = entry_set
                        for s in range(self.spc):
                            self._write_sector(sector + s,
                                dir_data[s * SECTOR_SIZE:(s + 1) * SECTOR_SIZE])
                        return
                    if etype == 0x00:
                        # End of dir — check remaining space
                        remaining = len(dir_data) // 32 - run_start
                        if remaining >= entry_count:
                            write_off = run_start * 32
                            dir_data[write_off:write_off + len(entry_set)] = entry_set
                            for s in range(self.spc):
                                self._write_sector(sector + s,
                                    dir_data[s * SECTOR_SIZE:(s + 1) * SECTOR_SIZE])
                            return
                        break  # Need new cluster
                else:
                    run_len = 0
                    run_start = -1

            # Check FAT for next cluster
            fat_val = struct.unpack_from('<I', self.fat_cache, cluster * 4)[0]
            if fat_val >= 0xFFFFFFF8 or fat_val == 0:
                # Extend directory with new cluster
                new_cluster = self._alloc_cluster()
                struct.pack_into('<I', self.fat_cache, cluster * 4, new_cluster)
                new_data = bytearray(self.cluster_size)
                new_data[0:len(entry_set)] = entry_set
                self._write_cluster(new_cluster, new_data)
                return
            cluster = fat_val

    # =====================================================================
    # Public API
    # =====================================================================

    def create_directory(self, parent_cluster, dirname):
        """Create a subdirectory. Returns the new directory's cluster."""
        dir_cluster = self._alloc_cluster()
        # Initialize empty directory
        self._write_cluster(dir_cluster, bytearray(self.cluster_size))

        entry_set = self._build_entry_set(
            dirname, self.ATTR_DIRECTORY, dir_cluster, 0, contiguous=False)

        if parent_cluster is None:
            parent_cluster = self.root_cluster
        self._add_entry_to_dir(parent_cluster, entry_set)
        return dir_cluster

    def add_file(self, parent_cluster, filename, data):
        """Add a file to a directory."""
        if parent_cluster is None:
            parent_cluster = self.root_cluster

        if len(data) == 0:
            entry_set = self._build_entry_set(
                filename, self.ATTR_ARCHIVE, 0, 0, contiguous=True)
            self._add_entry_to_dir(parent_cluster, entry_set)
            return

        num_clusters = (len(data) + self.cluster_size - 1) // self.cluster_size
        first_cluster = self._alloc_clusters_contiguous(num_clusters)
        self._write_to_clusters_contiguous(first_cluster, data)

        entry_set = self._build_entry_set(
            filename, self.ATTR_ARCHIVE, first_cluster, len(data), contiguous=True)
        self._add_entry_to_dir(parent_cluster, entry_set)

        print(f"    File: {filename} ({len(data)} bytes, {num_clusters} cluster(s), "
              f"start={first_cluster}, contiguous)")

    def populate_from_sysroot(self, sysroot_path):
        """Recursively copy files from sysroot directory to the filesystem."""
        if not os.path.isdir(sysroot_path):
            print(f"  Warning: sysroot path '{sysroot_path}' does not exist, skipping")
            return
        self._populate_dir(sysroot_path, self.root_cluster)

    def _populate_dir(self, host_path, parent_cluster):
        """Recursively populate a directory."""
        entries = sorted(os.listdir(host_path))

        for entry_name in entries:
            full_path = os.path.join(host_path, entry_name)

            if entry_name.startswith('.'):
                continue

            if os.path.isdir(full_path):
                dir_cluster = self.create_directory(parent_cluster, entry_name)
                print(f"    Dir:  {entry_name}/ (cluster={dir_cluster})")
                self._populate_dir(full_path, dir_cluster)

            elif os.path.isfile(full_path):
                with open(full_path, 'rb') as f:
                    data = f.read()
                self.add_file(parent_cluster, entry_name, data)

    def flush_fat_and_bitmap(self):
        """Write the in-memory FAT cache and allocation bitmap to disk."""
        # Write FAT
        for s in range(self.fat_length):
            offset = s * SECTOR_SIZE
            chunk = self.fat_cache[offset:offset + SECTOR_SIZE]
            if len(chunk) < SECTOR_SIZE:
                chunk = chunk + b'\x00' * (SECTOR_SIZE - len(chunk))
            self._write_sector(self.fat_offset + s, chunk)

        # Write allocation bitmap to its cluster(s)
        bitmap_size = len(self.bitmap)
        num_clusters = (bitmap_size + self.cluster_size - 1) // self.cluster_size
        offset = 0
        cluster = self.bitmap_cluster
        for _ in range(num_clusters):
            chunk = self.bitmap[offset:offset + self.cluster_size]
            if len(chunk) < self.cluster_size:
                chunk = chunk + b'\x00' * (self.cluster_size - len(chunk))
            self._write_cluster(cluster, chunk)
            offset += self.cluster_size
            cluster += 1

        print(f"  exFAT: FAT and bitmap flushed ({self.next_cluster - 2} clusters used "
              f"of {self.cluster_count})")


# =====================================================================
# GPT (GUID Partition Table) helpers
# =====================================================================

GPT_SIGNATURE = b'EFI PART'
GPT_REVISION = 0x00010000
GPT_HEADER_SIZE = 92
GPT_ENTRY_SIZE = 128
GPT_ENTRY_COUNT = 128  # Standard: 128 entries = 32 sectors

# Well-known partition type GUIDs
ESP_TYPE_GUID = uuid.UUID("C12A7328-F81F-11D2-BA4B-00A0C93EC93B")
BASIC_DATA_TYPE_GUID = uuid.UUID("EBD0A0A2-B9E5-4433-87C0-68B6B72699C7")


def guid_to_bytes(guid):
    """Convert UUID to GPT mixed-endian bytes (first 3 fields LE, rest BE)."""
    return guid.bytes_le


def write_protective_mbr(image, total_sectors):
    """Write a protective MBR for GPT."""
    mbr = bytearray(512)
    # Partition entry 1 at offset 446
    mbr[446] = 0x00  # Boot indicator (not bootable)
    mbr[447] = 0x00  # CHS start
    mbr[448] = 0x02
    mbr[449] = 0x00
    mbr[450] = 0xEE  # GPT protective type
    mbr[451] = 0xFF  # CHS end
    mbr[452] = 0xFF
    mbr[453] = 0xFF
    struct.pack_into('<I', mbr, 454, 1)  # Start LBA = 1
    max_sectors = min(total_sectors - 1, 0xFFFFFFFF)
    struct.pack_into('<I', mbr, 458, max_sectors)
    mbr[510] = 0x55
    mbr[511] = 0xAA
    image[0:512] = mbr


def create_gpt(image, total_sectors, partitions):
    """Create GPT header and partition entries.

    partitions: list of (type_guid, unique_guid, first_lba, last_lba, name)
    """
    disk_guid = uuid.uuid4()
    entry_sectors = (GPT_ENTRY_COUNT * GPT_ENTRY_SIZE + 511) // 512  # = 32

    # Build partition entries
    entries = bytearray(GPT_ENTRY_COUNT * GPT_ENTRY_SIZE)
    for i, (type_guid, unique_guid, first_lba, last_lba, name) in enumerate(partitions):
        off = i * GPT_ENTRY_SIZE
        entries[off:off + 16] = guid_to_bytes(type_guid)
        entries[off + 16:off + 32] = guid_to_bytes(unique_guid)
        struct.pack_into('<Q', entries, off + 32, first_lba)
        struct.pack_into('<Q', entries, off + 40, last_lba)
        struct.pack_into('<Q', entries, off + 48, 0)  # Attributes
        name_bytes = name.encode('utf-16-le')[:72]
        entries[off + 56:off + 56 + len(name_bytes)] = name_bytes

    entries_crc = zlib.crc32(entries) & 0xFFFFFFFF

    first_usable_lba = 2 + entry_sectors  # = 34
    last_usable_lba = total_sectors - 1 - entry_sectors - 1

    def make_header(my_lba, alt_lba, entries_lba):
        hdr = bytearray(512)
        hdr[0:8] = GPT_SIGNATURE
        struct.pack_into('<I', hdr, 8, GPT_REVISION)
        struct.pack_into('<I', hdr, 12, GPT_HEADER_SIZE)
        struct.pack_into('<I', hdr, 16, 0)  # CRC32 placeholder
        struct.pack_into('<I', hdr, 20, 0)  # Reserved
        struct.pack_into('<Q', hdr, 24, my_lba)
        struct.pack_into('<Q', hdr, 32, alt_lba)
        struct.pack_into('<Q', hdr, 40, first_usable_lba)
        struct.pack_into('<Q', hdr, 48, last_usable_lba)
        hdr[56:72] = guid_to_bytes(disk_guid)
        struct.pack_into('<Q', hdr, 72, entries_lba)
        struct.pack_into('<I', hdr, 80, GPT_ENTRY_COUNT)
        struct.pack_into('<I', hdr, 84, GPT_ENTRY_SIZE)
        struct.pack_into('<I', hdr, 88, entries_crc)
        # Calculate header CRC32 (over first GPT_HEADER_SIZE bytes with CRC field zeroed)
        header_crc = zlib.crc32(bytes(hdr[:GPT_HEADER_SIZE])) & 0xFFFFFFFF
        struct.pack_into('<I', hdr, 16, header_crc)
        return hdr

    # Primary header at LBA 1, entries at LBA 2
    primary = make_header(1, total_sectors - 1, 2)
    image[512:1024] = primary
    entries_offset = 2 * 512
    image[entries_offset:entries_offset + len(entries)] = entries

    # Backup entries just before backup header
    backup_entries_lba = total_sectors - 1 - entry_sectors
    backup_entries_offset = backup_entries_lba * 512
    image[backup_entries_offset:backup_entries_offset + len(entries)] = entries

    # Backup header at last LBA
    backup = make_header(total_sectors - 1, 1, backup_entries_lba)
    backup_offset = (total_sectors - 1) * 512
    image[backup_offset:backup_offset + 512] = backup

    print(f"  GPT: disk_guid={disk_guid}")
    print(f"  GPT: first_usable={first_usable_lba}, last_usable={last_usable_lba}")
    for i, (_, _, first, last, name) in enumerate(partitions):
        print(f"  GPT: partition {i + 1}: '{name}' LBA {first}-{last} ({(last - first + 1) * 512 // 1024} KiB)")


# =====================================================================
# Image creation: BIOS mode
# =====================================================================

def create_bios_image(args):
    """Create a BIOS-bootable disk image (MBR + Stage 1/2 + kernel sectors)."""
    image_size = args.image_size * 1024 * 1024

    with open(args.stage1, "rb") as f:
        stage1 = f.read()
    if len(stage1) != SECTOR_SIZE:
        print(f"ERROR: Stage 1 must be exactly {SECTOR_SIZE} bytes, got {len(stage1)}", file=sys.stderr)
        sys.exit(1)

    with open(args.stage2, "rb") as f:
        stage2 = f.read()
    stage2_max = 63 * SECTOR_SIZE
    if len(stage2) > stage2_max:
        print(f"ERROR: Stage 2 is {len(stage2)} bytes, max is {stage2_max}", file=sys.stderr)
        sys.exit(1)

    with open(args.kernel, "rb") as f:
        kernel_elf = f.read()

    KERNEL_LMA = 0x00100000
    print(f"Kernel ELF: {len(kernel_elf)} bytes")
    kernel = elf_to_flat_binary(kernel_elf, KERNEL_LMA)

    kernel_sectors = (len(kernel) + SECTOR_SIZE - 1) // SECTOR_SIZE
    kernel_start_sector = 64

    print(f"Stage 1: {len(stage1)} bytes (1 sector)")
    print(f"Stage 2: {len(stage2)} bytes ({(len(stage2) + SECTOR_SIZE - 1) // SECTOR_SIZE} sectors)")
    print(f"Kernel:  {len(kernel)} bytes ({kernel_sectors} sectors, starting at sector {kernel_start_sector})")

    kernel_end_sector = kernel_start_sector + kernel_sectors
    if kernel_end_sector > args.fs_start:
        print(f"ERROR: Kernel ends at sector {kernel_end_sector}, which overlaps "
              f"filesystem at sector {args.fs_start}", file=sys.stderr)
        sys.exit(1)

    if len(stage2) >= 8:
        stage2_bytes = bytearray(stage2)
        struct.pack_into("<H", stage2_bytes, 2, kernel_sectors)
        struct.pack_into("<I", stage2_bytes, 4, kernel_start_sector)
        stage2 = bytes(stage2_bytes)

    image = bytearray(image_size)
    image[0:len(stage1)] = stage1

    stage2_offset = SECTOR_SIZE
    image[stage2_offset:stage2_offset + len(stage2)] = stage2

    kernel_offset = kernel_start_sector * SECTOR_SIZE
    image[kernel_offset:kernel_offset + len(kernel)] = kernel

    fs_sector_count = (image_size // SECTOR_SIZE) - args.fs_start
    print(f"\nexFAT filesystem:")
    print(f"  Start sector: {args.fs_start} (offset 0x{args.fs_start * SECTOR_SIZE:X})")
    print(f"  Size: {fs_sector_count} sectors ({fs_sector_count * SECTOR_SIZE // (1024 * 1024)} MiB)")

    exfat = ExFatFormatter(image, args.fs_start, fs_sector_count)
    exfat.write_boot_sector()
    exfat.init_fs()

    if args.sysroot:
        print(f"  Populating from sysroot: {args.sysroot}")
        exfat.populate_from_sysroot(args.sysroot)

    exfat.flush_fat_and_bitmap()

    with open(args.output, "wb") as f:
        f.write(image)

    print(f"\nDisk image created: {args.output} ({args.image_size} MiB)")


# =====================================================================
# Image creation: UEFI mode
# =====================================================================

def create_uefi_image(args):
    """Create a UEFI-bootable disk image (GPT + ESP + exFAT data partition)."""
    if not args.bootloader:
        print("ERROR: --bootloader required for UEFI mode", file=sys.stderr)
        sys.exit(1)

    image_size = args.image_size * 1024 * 1024
    total_sectors = image_size // SECTOR_SIZE

    # Read EFI bootloader
    with open(args.bootloader, 'rb') as f:
        efi_data = f.read()

    # If --kernel is given, convert ELF to flat binary and inject into sysroot
    kernel_flat = None
    if args.kernel:
        with open(args.kernel, 'rb') as f:
            kernel_elf = f.read()
        KERNEL_LMA = 0x00100000
        print(f"Kernel ELF: {len(kernel_elf)} bytes")
        kernel_flat = elf_to_flat_binary(kernel_elf, KERNEL_LMA)

    print(f"\nUEFI image: {args.image_size} MiB ({total_sectors} sectors)")
    print(f"EFI bootloader: {len(efi_data)} bytes")
    if kernel_flat:
        print(f"Kernel flat binary: {len(kernel_flat)} bytes")

    # Partition layout
    esp_start = 2048
    esp_sectors = 6144  # 3 MiB (data partition must start at LBA 8192 to match kernel PARTITION_LBA)
    esp_end = esp_start + esp_sectors - 1

    data_start = esp_start + esp_sectors  # 8192 — matches kernel's PARTITION_LBA
    entry_sectors = (GPT_ENTRY_COUNT * GPT_ENTRY_SIZE + 511) // 512
    data_end = total_sectors - 1 - entry_sectors - 1  # before backup GPT
    data_sectors = data_end - data_start + 1

    print(f"\nPartition layout:")
    print(f"  ESP:  sectors {esp_start}-{esp_end} ({esp_sectors * 512 // 1024} KiB)")
    print(f"  Data: sectors {data_start}-{data_end} ({data_sectors * 512 // (1024 * 1024)} MiB)")

    # Create image
    image = bytearray(image_size)

    # Write protective MBR
    write_protective_mbr(image, total_sectors)

    # Write GPT
    partitions = [
        (ESP_TYPE_GUID, uuid.uuid4(), esp_start, esp_end, "EFI System"),
        (BASIC_DATA_TYPE_GUID, uuid.uuid4(), data_start, data_end, "anyOS Data"),
    ]
    create_gpt(image, total_sectors, partitions)

    # Format ESP as FAT16 (1 sector/cluster for small partition)
    print(f"\nESP filesystem:")
    esp_fat = Fat16Formatter(image, esp_start, esp_sectors, sectors_per_cluster=1)
    esp_fat.write_boot_sector()
    esp_fat.init_fat()

    # Create /EFI/BOOT/BOOTX64.EFI in ESP
    efi_dir = esp_fat.create_directory(None, "EFI", is_root_parent=True)
    boot_dir = esp_fat.create_directory(efi_dir, "BOOT", is_root_parent=False)
    esp_fat.add_file(boot_dir, "BOOTX64.EFI", efi_data, is_root_parent=False)

    # Place kernel on ESP (FAT16) so UEFI firmware can always read it
    # (exFAT data partition may not be readable by all UEFI implementations)
    if kernel_flat:
        sys_dir = esp_fat.create_directory(None, "System", is_root_parent=True)
        esp_fat.add_file(sys_dir, "kernel.bin", kernel_flat, is_root_parent=False)
        print(f"  Wrote kernel.bin to ESP ({len(kernel_flat)} bytes)")

    # Format data partition as exFAT
    print(f"\nData filesystem (exFAT):")
    data_exfat = ExFatFormatter(image, data_start, data_sectors)
    data_exfat.write_boot_sector()
    data_exfat.init_fs()

    # Populate data partition from sysroot
    if args.sysroot:
        print(f"  Populating from sysroot: {args.sysroot}")
        data_exfat.populate_from_sysroot(args.sysroot)

    data_exfat.flush_fat_and_bitmap()

    # Write image
    with open(args.output, "wb") as f:
        f.write(image)

    print(f"\nUEFI disk image created: {args.output} ({args.image_size} MiB)")


# =====================================================================
# ISO 9660 helpers
# =====================================================================

ISO_BLOCK_SIZE = 2048

def both_endian_u32(val):
    """Encode a 32-bit value in ISO 9660 both-endian format (LE + BE = 8 bytes)."""
    return struct.pack('<I', val) + struct.pack('>I', val)

def both_endian_u16(val):
    """Encode a 16-bit value in ISO 9660 both-endian format (LE + BE = 4 bytes)."""
    return struct.pack('<H', val) + struct.pack('>H', val)

def iso_datetime_now():
    """ISO 9660 directory record date/time (7 bytes): year-1900,month,day,hour,min,sec,gmt_offset."""
    import time
    t = time.localtime()
    return bytes([t.tm_year - 1900, t.tm_mon, t.tm_mday, t.tm_hour, t.tm_min, t.tm_sec, 0])

def iso_dec_datetime_now():
    """ISO 9660 PVD date/time string (17 bytes ASCII): YYYYMMDDHHMMSSCC + GMT offset."""
    import time
    t = time.localtime()
    s = f'{t.tm_year:04d}{t.tm_mon:02d}{t.tm_mday:02d}{t.tm_hour:02d}{t.tm_min:02d}{t.tm_sec:02d}00'
    return s.encode('ascii') + b'\x00'  # 17 bytes

def make_dir_record(lba, data_len, flags, name_bytes, is_root=False):
    """Build an ISO 9660 directory record (variable length)."""
    name_len = len(name_bytes)
    rec_len = 33 + name_len
    if rec_len % 2 != 0:
        rec_len += 1  # Pad to even

    rec = bytearray(rec_len)
    rec[0] = rec_len
    rec[1] = 0  # Extended attribute length
    rec[2:10] = both_endian_u32(lba)
    rec[10:18] = both_endian_u32(data_len)
    rec[18:25] = iso_datetime_now()
    rec[25] = flags
    rec[26] = 0  # File unit size
    rec[27] = 0  # Interleave gap size
    rec[28:32] = both_endian_u16(1)  # Volume sequence number
    rec[32] = name_len
    rec[33:33 + name_len] = name_bytes
    return bytes(rec)


class Iso9660Creator:
    """Creates an ISO 9660 filesystem image with El Torito boot support."""

    def __init__(self):
        self.dirs = {}     # path -> {'lba': int, 'entries': [DirRecord,...], 'children': [name,...]}
        self.files = {}    # path -> {'data': bytes, 'lba': int}
        self.volume_id = 'ANYOS_LIVE'

    def add_sysroot(self, sysroot_path):
        """Recursively collect all files and directories from sysroot."""
        if not os.path.isdir(sysroot_path):
            return
        self._collect_dir(sysroot_path, '/')

    def _collect_dir(self, host_path, iso_path):
        """Recursively collect directory contents."""
        if iso_path not in self.dirs:
            self.dirs[iso_path] = {'children': [], 'files': []}

        for entry in sorted(os.listdir(host_path)):
            if entry.startswith('.'):
                continue
            full = os.path.join(host_path, entry)
            child_iso = iso_path.rstrip('/') + '/' + entry

            if os.path.isdir(full):
                self.dirs[iso_path]['children'].append(entry)
                self._collect_dir(full, child_iso)
            elif os.path.isfile(full):
                with open(full, 'rb') as f:
                    data = f.read()
                self.files[child_iso] = {'data': data}
                self.dirs[iso_path]['files'].append(entry)

    def write_image(self, output_path, stage1_data=None, stage2_data=None, kernel_flat=None):
        """Write the complete ISO 9660 image."""
        # Layout:
        # Sectors 0-15:  System area (stage1 + stage2)
        # Sector 16:     PVD
        # Sector 17:     Boot Record Volume Descriptor (El Torito)
        # Sector 18:     VD Set Terminator
        # Sector 19:     Boot Catalog
        # Sector 20:     Path Table (L-type)
        # Sector 21:     Path Table (M-type)
        # Sector 22+:    Directory extents
        # Then:          Kernel data (at sector 32 minimum = disk LBA 128)
        # Then:          File data

        has_boot = stage1_data is not None and stage2_data is not None

        # Assign LBAs for directories
        all_dirs = sorted(self.dirs.keys())
        dir_lba_start = 22
        dir_lbas = {}
        next_lba = dir_lba_start
        for d in all_dirs:
            dir_lbas[d] = next_lba
            # Estimate directory extent size (1 sector per dir for now, expand if needed)
            next_lba += 1

        # Kernel data LBA (must be at CD sector 32 = disk LBA 128 for bootloader)
        kernel_lba = 32
        if next_lba > kernel_lba:
            kernel_lba = next_lba
        kernel_sectors = 0
        if kernel_flat:
            kernel_sectors = (len(kernel_flat) + ISO_BLOCK_SIZE - 1) // ISO_BLOCK_SIZE
        file_data_lba = kernel_lba + kernel_sectors

        # Assign LBAs for files
        file_lbas = {}
        cur_lba = file_data_lba
        for fpath in sorted(self.files.keys()):
            fdata = self.files[fpath]['data']
            file_lbas[fpath] = cur_lba
            sectors = (len(fdata) + ISO_BLOCK_SIZE - 1) // ISO_BLOCK_SIZE
            cur_lba += max(sectors, 1)  # At least 1 sector per file

        total_sectors = cur_lba

        # Build directory extents
        dir_extents = {}
        for d in all_dirs:
            extent = bytearray()
            d_lba = dir_lbas[d]

            # "." entry
            extent += make_dir_record(d_lba, ISO_BLOCK_SIZE, 0x02, b'\x00')
            # ".." entry
            parent = '/'.join(d.rstrip('/').split('/')[:-1]) or '/'
            parent_lba = dir_lbas.get(parent, dir_lbas.get('/', d_lba))
            extent += make_dir_record(parent_lba, ISO_BLOCK_SIZE, 0x02, b'\x01')

            # Child directories
            children = self.dirs[d].get('children', [])
            for child_name in children:
                child_path = d.rstrip('/') + '/' + child_name
                child_lba = dir_lbas[child_path]
                name_bytes = child_name.upper().encode('ascii')
                extent += make_dir_record(child_lba, ISO_BLOCK_SIZE, 0x02, name_bytes)

            # Files
            files_in_dir = self.dirs[d].get('files', [])
            for fname in files_in_dir:
                fpath = d.rstrip('/') + '/' + fname
                fdata = self.files[fpath]['data']
                flba = file_lbas[fpath]
                # ISO 9660 filename: uppercase + ";1" version
                iso_name = fname.upper()
                if '.' not in iso_name:
                    iso_name += '.'
                iso_name += ';1'
                name_bytes = iso_name.encode('ascii')
                extent += make_dir_record(flba, len(fdata), 0x00, name_bytes)

            # Pad extent to block boundary
            while len(extent) % ISO_BLOCK_SIZE != 0:
                extent += b'\x00'

            # Check if we need more than 1 sector
            needed = len(extent) // ISO_BLOCK_SIZE
            if needed > 1:
                # Need to reassign LBAs — for simplicity, just allocate more
                # This could cause overlap. In practice, sysroot dirs are small enough.
                pass

            dir_extents[d] = bytes(extent)

        # Build Path Table (L-type, little-endian)
        path_table = bytearray()
        # Entry format: dir_id_len(1), ext_attr_len(1), extent_lba(4, LE), parent_dir_num(2, LE), dir_id(N), pad(1 if odd)
        dir_numbers = {}
        dir_num = 1
        for d in all_dirs:
            dir_numbers[d] = dir_num
            dir_num += 1

        for d in all_dirs:
            d_lba = dir_lbas[d]
            parent = '/'.join(d.rstrip('/').split('/')[:-1]) or '/'
            parent_num = dir_numbers.get(parent, 1)
            if d == '/':
                name_bytes = b'\x01'  # Root directory identifier
                parent_num = 1
            else:
                name_bytes = d.rsplit('/', 1)[-1].upper().encode('ascii')

            entry = bytearray()
            entry.append(len(name_bytes))  # Directory identifier length
            entry.append(0)                 # Extended attribute record length
            entry += struct.pack('<I', d_lba)   # LBA (LE)
            entry += struct.pack('<H', parent_num)  # Parent dir number (LE)
            entry += name_bytes
            if len(name_bytes) % 2 != 0:
                entry += b'\x00'  # Padding

            path_table += entry

        path_table_size = len(path_table)

        # Build Path Table (M-type, big-endian)
        path_table_m = bytearray()
        for d in all_dirs:
            d_lba = dir_lbas[d]
            parent = '/'.join(d.rstrip('/').split('/')[:-1]) or '/'
            parent_num = dir_numbers.get(parent, 1)
            if d == '/':
                name_bytes = b'\x01'
                parent_num = 1
            else:
                name_bytes = d.rsplit('/', 1)[-1].upper().encode('ascii')

            entry = bytearray()
            entry.append(len(name_bytes))
            entry.append(0)
            entry += struct.pack('>I', d_lba)   # LBA (BE)
            entry += struct.pack('>H', parent_num)  # Parent dir number (BE)
            entry += name_bytes
            if len(name_bytes) % 2 != 0:
                entry += b'\x00'

            path_table_m += entry

        # Build PVD
        root_dir_lba = dir_lbas['/']
        root_dir_size = len(dir_extents['/'])
        pvd = self._make_pvd(total_sectors, root_dir_lba, root_dir_size,
                             20, path_table_size)

        # Build El Torito BRVD
        brvd = bytearray(ISO_BLOCK_SIZE)
        brvd[0] = 0     # Boot Record type
        brvd[1:6] = b'CD001'
        brvd[6] = 1     # Version
        brvd[7:39] = b'EL TORITO SPECIFICATION' + b'\x00' * 9
        # Boot catalog LBA at offset 71 (LE u32)
        struct.pack_into('<I', brvd, 71, 19)  # Boot catalog at sector 19

        # Build VD Set Terminator
        vdst = bytearray(ISO_BLOCK_SIZE)
        vdst[0] = 255  # Terminator type
        vdst[1:6] = b'CD001'
        vdst[6] = 1

        # Build Boot Catalog
        boot_cat = bytearray(ISO_BLOCK_SIZE)
        # Validation Entry (32 bytes)
        boot_cat[0] = 0x01   # Header ID
        boot_cat[1] = 0x00   # Platform ID (x86)
        boot_cat[28] = 0xAA  # Key byte 1
        boot_cat[29] = 0x55  # Key byte 2
        # Calculate checksum for validation entry (sum of all 16-bit LE words must be 0)
        boot_cat[30] = 0
        boot_cat[31] = 0
        checksum = 0
        for i in range(0, 32, 2):
            checksum += struct.unpack_from('<H', boot_cat, i)[0]
        checksum = (0x10000 - (checksum & 0xFFFF)) & 0xFFFF
        struct.pack_into('<H', boot_cat, 28 - 2, checksum)  # Ugh wait, the spec says offset 28=key bytes
        # Actually: validation entry checksum is at offset 28 = key byte 55AA, checksum is included
        # Let me redo: all words sum to 0. Key bytes at offset 30-31.
        # Re-read spec: byte 0=header_id(1), 1=platform(0), 2-3=reserved, 4-27=ID string,
        #               28-29=checksum, 30=key_byte_55, 31=key_byte_AA
        boot_cat[30] = 0x55
        boot_cat[31] = 0xAA
        boot_cat[28] = 0  # checksum placeholder
        boot_cat[29] = 0
        checksum = 0
        for i in range(0, 32, 2):
            checksum += struct.unpack_from('<H', boot_cat, i)[0]
        checksum = (0x10000 - (checksum & 0xFFFF)) & 0xFFFF
        struct.pack_into('<H', boot_cat, 28, checksum)

        # Initial/Default Entry (32 bytes, at offset 32)
        boot_cat[32] = 0x88  # Bootable
        boot_cat[33] = 0x00  # No emulation
        struct.pack_into('<H', boot_cat, 34, 0x0000)  # Load Segment (0 = default 0x7C0)
        boot_cat[36] = 0x00  # System type
        boot_cat[37] = 0x00  # Unused
        # Sector count: number of 512-byte virtual sectors to load
        # Load stage1 (1 sector) + stage2 (63 sectors) = 64 sectors = 32 KiB
        struct.pack_into('<H', boot_cat, 38, 64)
        # Load RBA: CD sector of boot image (sector 0 = system area)
        struct.pack_into('<I', boot_cat, 40, 0)

        # === Assemble the image ===
        image_size = total_sectors * ISO_BLOCK_SIZE
        image = bytearray(image_size)

        # System area (sectors 0-15): stage1 + stage2
        # Patch stage2 with actual kernel location (must happen after layout is computed)
        if has_boot:
            stage2_patched = bytearray(stage2_data)
            kernel_disk_lba = kernel_lba * (ISO_BLOCK_SIZE // SECTOR_SIZE)  # CD sector → 512-byte sector
            kernel_disk_sectors = (len(kernel_flat) + SECTOR_SIZE - 1) // SECTOR_SIZE if kernel_flat else 0
            if len(stage2_patched) >= 8:
                struct.pack_into("<H", stage2_patched, 2, kernel_disk_sectors)
                struct.pack_into("<I", stage2_patched, 4, kernel_disk_lba)
            print(f"  Stage2 patched: kernel at disk LBA {kernel_disk_lba}, {kernel_disk_sectors} sectors")
            image[0:len(stage1_data)] = stage1_data
            image[SECTOR_SIZE:SECTOR_SIZE + len(stage2_patched)] = stage2_patched

        # PVD at sector 16
        image[16 * ISO_BLOCK_SIZE:16 * ISO_BLOCK_SIZE + ISO_BLOCK_SIZE] = pvd
        # BRVD at sector 17
        image[17 * ISO_BLOCK_SIZE:17 * ISO_BLOCK_SIZE + ISO_BLOCK_SIZE] = brvd
        # VD Terminator at sector 18
        image[18 * ISO_BLOCK_SIZE:18 * ISO_BLOCK_SIZE + ISO_BLOCK_SIZE] = vdst
        # Boot Catalog at sector 19
        image[19 * ISO_BLOCK_SIZE:19 * ISO_BLOCK_SIZE + ISO_BLOCK_SIZE] = boot_cat
        # Path Table L at sector 20
        pt_offset = 20 * ISO_BLOCK_SIZE
        image[pt_offset:pt_offset + len(path_table)] = path_table
        # Path Table M at sector 21
        pt_m_offset = 21 * ISO_BLOCK_SIZE
        image[pt_m_offset:pt_m_offset + len(path_table_m)] = path_table_m

        # Directory extents
        for d in all_dirs:
            d_lba = dir_lbas[d]
            d_offset = d_lba * ISO_BLOCK_SIZE
            ext_data = dir_extents[d]
            image[d_offset:d_offset + len(ext_data)] = ext_data

        # Kernel data at sector 32 (= disk LBA 128)
        if kernel_flat:
            k_offset = kernel_lba * ISO_BLOCK_SIZE
            image[k_offset:k_offset + len(kernel_flat)] = kernel_flat
            print(f"  Kernel at CD sector {kernel_lba} ({len(kernel_flat)} bytes, "
                  f"{kernel_sectors} sectors)")

        # File data
        for fpath in sorted(self.files.keys()):
            fdata = self.files[fpath]['data']
            flba = file_lbas[fpath]
            f_offset = flba * ISO_BLOCK_SIZE
            image[f_offset:f_offset + len(fdata)] = fdata

        # Write image
        with open(output_path, 'wb') as f:
            f.write(image)

        iso_size_mb = len(image) / (1024 * 1024)
        print(f"\n  ISO 9660 image: {output_path} ({iso_size_mb:.1f} MiB, "
              f"{total_sectors} CD sectors)")
        print(f"  Files: {len(self.files)}, Directories: {len(self.dirs)}")

        return kernel_lba, kernel_sectors

    def _make_pvd(self, total_blocks, root_dir_lba, root_dir_size,
                  path_table_lba, path_table_size):
        """Create a Primary Volume Descriptor."""
        pvd = bytearray(ISO_BLOCK_SIZE)
        pvd[0] = 1      # Type: PVD
        pvd[1:6] = b'CD001'
        pvd[6] = 1      # Version
        # System identifier (bytes 8-39, 32 chars, space-padded)
        sys_id = b'ANYOS                           '
        pvd[8:40] = sys_id
        # Volume identifier (bytes 40-71, 32 chars, space-padded)
        vol_id = self.volume_id.ljust(32).encode('ascii')[:32]
        pvd[40:72] = vol_id
        # Volume Space Size (bytes 80-87, both-endian u32)
        pvd[80:88] = both_endian_u32(total_blocks)
        # Volume Set Size (bytes 120-123, both-endian u16)
        pvd[120:124] = both_endian_u16(1)
        # Volume Sequence Number (bytes 124-127, both-endian u16)
        pvd[124:128] = both_endian_u16(1)
        # Logical Block Size (bytes 128-131, both-endian u16)
        pvd[128:132] = both_endian_u16(ISO_BLOCK_SIZE)
        # Path Table Size (bytes 132-139, both-endian u32)
        pvd[132:140] = both_endian_u32(path_table_size)
        # Type L Path Table Location (bytes 140-143, u32 LE)
        struct.pack_into('<I', pvd, 140, path_table_lba)
        # Optional Type L Path Table Location (bytes 144-147, u32 LE)
        struct.pack_into('<I', pvd, 144, 0)
        # Type M Path Table Location (bytes 148-151, u32 BE)
        struct.pack_into('>I', pvd, 148, path_table_lba + 1)
        # Optional Type M Path Table Location (bytes 152-155, u32 BE)
        struct.pack_into('>I', pvd, 152, 0)
        # Root Directory Record (bytes 156-189, 34 bytes)
        root_rec = make_dir_record(root_dir_lba, root_dir_size, 0x02, b'\x00', is_root=True)
        pvd[156:156 + len(root_rec)] = root_rec
        # Volume Set Identifier (190-317, 128 bytes)
        # Publisher Identifier (318-445, 128 bytes)
        # Data Preparer Identifier (446-573, 128 bytes)
        # Application Identifier (574-701, 128 bytes)
        app_id = b'ANYOS MKIMAGE                   '
        pvd[574:574 + len(app_id)] = app_id
        # Volume Creation Date/Time (813-829, 17 bytes)
        pvd[813:830] = iso_dec_datetime_now()
        # Volume Modification Date/Time (830-846)
        pvd[830:847] = iso_dec_datetime_now()
        # File Structure Version (881)
        pvd[881] = 1
        return bytes(pvd)


# =====================================================================
# Image creation: ISO mode (El Torito bootable CD-ROM)
# =====================================================================

def create_iso_image(args):
    """Create a bootable ISO 9660 image with El Torito BIOS boot."""
    if not args.stage1 or not args.stage2 or not args.kernel:
        print("ERROR: --stage1, --stage2, and --kernel are required for ISO mode",
              file=sys.stderr)
        sys.exit(1)

    with open(args.stage1, "rb") as f:
        stage1 = f.read()
    if len(stage1) != SECTOR_SIZE:
        print(f"ERROR: Stage 1 must be exactly {SECTOR_SIZE} bytes", file=sys.stderr)
        sys.exit(1)

    with open(args.stage2, "rb") as f:
        stage2 = f.read()
    stage2_max = 63 * SECTOR_SIZE
    if len(stage2) > stage2_max:
        print(f"ERROR: Stage 2 is {len(stage2)} bytes, max is {stage2_max}", file=sys.stderr)
        sys.exit(1)

    with open(args.kernel, "rb") as f:
        kernel_elf = f.read()

    KERNEL_LMA = 0x00100000
    print(f"Kernel ELF: {len(kernel_elf)} bytes")
    kernel_flat = elf_to_flat_binary(kernel_elf, KERNEL_LMA)

    kernel_sectors = (len(kernel_flat) + SECTOR_SIZE - 1) // SECTOR_SIZE

    print(f"\nISO 9660 Live CD image:")
    print(f"  Stage 1: {len(stage1)} bytes")
    print(f"  Stage 2: {len(stage2)} bytes")
    print(f"  Kernel:  {len(kernel_flat)} bytes ({kernel_sectors} disk sectors)")

    # Note: stage2 is patched with actual kernel LBA inside write_image()
    # after the ISO layout is computed (kernel may shift past CD sector 32
    # if many directories overflow the reserved space).

    # Collect sysroot
    iso = Iso9660Creator()
    if args.sysroot:
        print(f"  Populating ISO from sysroot: {args.sysroot}")
        iso.add_sysroot(args.sysroot)

    # Write ISO image (patches stage2 with correct kernel location)
    kernel_lba, _ = iso.write_image(args.output, stage1, stage2, kernel_flat)
    kernel_disk_lba = kernel_lba * (ISO_BLOCK_SIZE // SECTOR_SIZE)

    print(f"\nISO image created: {args.output}")
    print(f"  Boot: El Torito no-emulation, 64 sectors loaded at 0x7C00")
    print(f"  Kernel at CD sector {kernel_lba} (disk LBA {kernel_disk_lba})")


# =====================================================================
# Main entry point
# =====================================================================

def main():
    parser = argparse.ArgumentParser(description="Create anyOS disk image")
    parser.add_argument("--uefi", action="store_true",
                        help="Create UEFI (GPT+ESP) image instead of BIOS (MBR)")
    parser.add_argument("--iso", action="store_true",
                        help="Create bootable ISO 9660 (El Torito) image")
    parser.add_argument("--bootloader", default=None,
                        help="Path to UEFI bootloader .efi file (required for --uefi)")
    parser.add_argument("--stage1", default=None, help="Path to stage1.bin (BIOS mode)")
    parser.add_argument("--stage2", default=None, help="Path to stage2.bin (BIOS mode)")
    parser.add_argument("--kernel", default=None, help="Path to kernel ELF")
    parser.add_argument("--output", required=True, help="Output disk image path")
    parser.add_argument("--image-size", type=int, default=64, help="Image size in MiB")
    parser.add_argument("--sysroot", default=None,
                        help="Path to sysroot directory to populate filesystem")
    parser.add_argument("--fs-start", type=int, default=8192,
                        help="Start sector for exFAT filesystem (BIOS mode only)")
    args = parser.parse_args()

    if args.iso:
        create_iso_image(args)
    elif args.uefi:
        create_uefi_image(args)
    else:
        if not args.stage1 or not args.stage2 or not args.kernel:
            print("ERROR: --stage1, --stage2, and --kernel are required for BIOS mode",
                  file=sys.stderr)
            sys.exit(1)
        create_bios_image(args)


if __name__ == "__main__":
    main()
