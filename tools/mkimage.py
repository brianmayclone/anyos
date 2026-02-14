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
    Sector fs_start+: FAT16 filesystem (optional, if --sysroot is given)

  UEFI mode (--uefi):
    GPT partition table with:
      Partition 1: EFI System Partition (FAT16, 3 MiB) containing BOOTX64.EFI
      Partition 2: anyOS Data (FAT16) containing sysroot + /system/kernel.bin
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
    print(f"\nFAT16 filesystem:")
    print(f"  Start sector: {args.fs_start} (offset 0x{args.fs_start * SECTOR_SIZE:X})")
    print(f"  Size: {fs_sector_count} sectors ({fs_sector_count * SECTOR_SIZE // (1024 * 1024)} MiB)")

    fat = Fat16Formatter(image, args.fs_start, fs_sector_count)
    fat.write_boot_sector()
    fat.init_fat()

    if args.sysroot:
        print(f"  Populating from sysroot: {args.sysroot}")
        fat.populate_from_sysroot(args.sysroot)

    with open(args.output, "wb") as f:
        f.write(image)

    print(f"\nDisk image created: {args.output} ({args.image_size} MiB)")


# =====================================================================
# Image creation: UEFI mode
# =====================================================================

def create_uefi_image(args):
    """Create a UEFI-bootable disk image (GPT + ESP + FAT16 data partition)."""
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
    esp_sectors = 6144  # 3 MiB (data partition must start at LBA 8192 to match kernel FAT16_PARTITION_LBA)
    esp_end = esp_start + esp_sectors - 1

    data_start = esp_start + esp_sectors  # 8192 — matches kernel's FAT16_PARTITION_LBA
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

    # Format data partition as FAT16
    print(f"\nData filesystem:")
    data_fat = Fat16Formatter(image, data_start, data_sectors)
    data_fat.write_boot_sector()
    data_fat.init_fat()

    # If kernel flat binary available, write it to sysroot temporarily
    kernel_tmp_path = None
    if kernel_flat and args.sysroot:
        system_dir = os.path.join(args.sysroot, "system")
        os.makedirs(system_dir, exist_ok=True)
        kernel_tmp_path = os.path.join(system_dir, "kernel.bin")
        with open(kernel_tmp_path, 'wb') as f:
            f.write(kernel_flat)
        print(f"  Wrote kernel.bin to sysroot ({len(kernel_flat)} bytes)")

    # Populate data partition from sysroot
    if args.sysroot:
        print(f"  Populating from sysroot: {args.sysroot}")
        data_fat.populate_from_sysroot(args.sysroot)

    # Clean up temporary kernel.bin from sysroot
    if kernel_tmp_path and os.path.exists(kernel_tmp_path):
        os.remove(kernel_tmp_path)

    # Write image
    with open(args.output, "wb") as f:
        f.write(image)

    print(f"\nUEFI disk image created: {args.output} ({args.image_size} MiB)")


# =====================================================================
# Main entry point
# =====================================================================

def main():
    parser = argparse.ArgumentParser(description="Create anyOS disk image")
    parser.add_argument("--uefi", action="store_true",
                        help="Create UEFI (GPT+ESP) image instead of BIOS (MBR)")
    parser.add_argument("--bootloader", default=None,
                        help="Path to UEFI bootloader .efi file (required for --uefi)")
    parser.add_argument("--stage1", default=None, help="Path to stage1.bin (BIOS mode)")
    parser.add_argument("--stage2", default=None, help="Path to stage2.bin (BIOS mode)")
    parser.add_argument("--kernel", default=None, help="Path to kernel ELF")
    parser.add_argument("--output", required=True, help="Output disk image path")
    parser.add_argument("--image-size", type=int, default=64, help="Image size in MiB")
    parser.add_argument("--sysroot", default=None,
                        help="Path to sysroot directory to populate filesystem")
    parser.add_argument("--fs-start", type=int, default=2048,
                        help="Start sector for FAT16 filesystem (BIOS mode only)")
    args = parser.parse_args()

    if args.uefi:
        create_uefi_image(args)
    else:
        if not args.stage1 or not args.stage2 or not args.kernel:
            print("ERROR: --stage1, --stage2, and --kernel are required for BIOS mode",
                  file=sys.stderr)
            sys.exit(1)
        create_bios_image(args)


if __name__ == "__main__":
    main()
