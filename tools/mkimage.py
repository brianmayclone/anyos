#!/usr/bin/env python3
"""
mkimage.py - Create bootable disk image for anyOS

Disk layout:
  Sector 0:       Stage 1 (MBR, 512 bytes)
  Sectors 1-63:   Stage 2 (padded to 63 * 512 bytes)
  Sectors 64+:    Kernel flat binary (extracted from ELF PT_LOAD segments)
  Sector fs_start+: FAT16 filesystem (optional, if --sysroot is given)
  Total:          64 MiB image
"""

import argparse
import os
import struct
import sys

# ELF constants
ELF_MAGIC = b'\x7fELF'
PT_LOAD = 1

SECTOR_SIZE = 512


def elf_to_flat_binary(elf_data, base_paddr):
    """
    Parse an ELF32 file and extract PT_LOAD segments into a flat binary.
    The flat binary is laid out so that byte 0 corresponds to base_paddr.
    """
    # Verify ELF magic
    if elf_data[:4] != ELF_MAGIC:
        print("ERROR: Kernel is not a valid ELF file", file=sys.stderr)
        sys.exit(1)

    # Parse ELF32 header
    (e_type, e_machine, e_version, e_entry, e_phoff, e_shoff,
     e_flags, e_ehsize, e_phentsize, e_phnum, e_shentsize, e_shnum,
     e_shstrndx) = struct.unpack_from("<HHIIIIIHHHHHH", elf_data, 16)

    ei_class = elf_data[4]
    if ei_class != 1:
        print("ERROR: Kernel ELF is not 32-bit (ELF32)", file=sys.stderr)
        sys.exit(1)

    print(f"  ELF entry point: 0x{e_entry:08X}")
    print(f"  Program headers: {e_phnum} entries at offset {e_phoff}")

    # Parse program headers to find max extent
    max_paddr_end = 0
    segments = []
    for i in range(e_phnum):
        ph_offset = e_phoff + i * e_phentsize
        (p_type, p_offset, p_vaddr, p_paddr, p_filesz, p_memsz,
         p_flags, p_align) = struct.unpack_from("<IIIIIIII", elf_data, ph_offset)

        if p_type == PT_LOAD and p_filesz > 0:
            segments.append((p_paddr, p_offset, p_filesz, p_memsz, p_vaddr))
            end = p_paddr + p_memsz
            if end > max_paddr_end:
                max_paddr_end = end
            print(f"  PT_LOAD: paddr=0x{p_paddr:08X} vaddr=0x{p_vaddr:08X} "
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

    def __init__(self, image, fs_start_sector, fs_sector_count):
        self.image = image
        self.fs_start = fs_start_sector
        self.fs_sectors = fs_sector_count

        # FAT16 parameters
        self.bytes_per_sector = 512
        self.sectors_per_cluster = 8  # 4 KiB clusters
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

    def add_root_dir_entry(self, filename, first_cluster, file_size, is_directory=False):
        """Add a directory entry to the root directory."""
        entry = bytearray(32)
        entry[0:11] = self._make_83_name(filename)

        attr = 0x10 if is_directory else 0x20  # DIRECTORY or ARCHIVE
        entry[11] = attr

        # First cluster (low 16 bits)
        struct.pack_into('<H', entry, 26, first_cluster & 0xFFFF)
        # First cluster (high 16 bits, always 0 for FAT16)
        struct.pack_into('<H', entry, 20, 0)
        # File size
        struct.pack_into('<I', entry, 28, file_size if not is_directory else 0)

        # Write to root directory
        entry_offset = self.next_root_entry * 32
        sector_in_root = entry_offset // SECTOR_SIZE
        offset_in_sector = entry_offset % SECTOR_SIZE

        sector = self.first_root_dir_sector + sector_in_root
        sector_data = bytearray(self._read_sector(sector))
        sector_data[offset_in_sector:offset_in_sector + 32] = entry
        self._write_sector(sector, sector_data)

        self.next_root_entry += 1

    def add_subdir_entry(self, parent_cluster, filename, first_cluster, file_size, is_directory=False):
        """Add a directory entry to a subdirectory cluster."""
        # Read existing directory data
        cluster_size = self.sectors_per_cluster * SECTOR_SIZE
        dir_data = bytearray(cluster_size)
        sector = self._cluster_to_sector(parent_cluster)
        for s in range(self.sectors_per_cluster):
            s_data = self._read_sector(sector + s)
            dir_data[s * SECTOR_SIZE:(s + 1) * SECTOR_SIZE] = s_data

        # Find first empty entry
        for i in range(0, cluster_size, 32):
            if dir_data[i] == 0x00 or dir_data[i] == 0xE5:
                entry = bytearray(32)
                entry[0:11] = self._make_83_name(filename)
                attr = 0x10 if is_directory else 0x20
                entry[11] = attr
                struct.pack_into('<H', entry, 26, first_cluster & 0xFFFF)
                struct.pack_into('<H', entry, 20, 0)
                struct.pack_into('<I', entry, 28, file_size if not is_directory else 0)
                dir_data[i:i + 32] = entry
                break

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


def main():
    parser = argparse.ArgumentParser(description="Create anyOS disk image")
    parser.add_argument("--stage1", required=True, help="Path to stage1.bin")
    parser.add_argument("--stage2", required=True, help="Path to stage2.bin")
    parser.add_argument("--kernel", required=True, help="Path to kernel ELF")
    parser.add_argument("--output", required=True, help="Output disk image path")
    parser.add_argument("--image-size", type=int, default=64, help="Image size in MiB")
    parser.add_argument("--sysroot", default=None, help="Path to sysroot directory to populate filesystem")
    parser.add_argument("--fs-start", type=int, default=2048, help="Start sector for FAT16 filesystem")
    args = parser.parse_args()

    image_size = args.image_size * 1024 * 1024  # Convert to bytes

    # Read input files
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

    # Convert ELF to flat binary (segments placed at their physical addresses)
    KERNEL_LMA = 0x00100000  # Must match link.ld KERNEL_LMA
    print(f"Kernel ELF: {len(kernel_elf)} bytes")
    kernel = elf_to_flat_binary(kernel_elf, KERNEL_LMA)

    kernel_sectors = (len(kernel) + SECTOR_SIZE - 1) // SECTOR_SIZE
    kernel_start_sector = 64

    print(f"Stage 1: {len(stage1)} bytes (1 sector)")
    print(f"Stage 2: {len(stage2)} bytes ({(len(stage2) + SECTOR_SIZE - 1) // SECTOR_SIZE} sectors)")
    print(f"Kernel:  {len(kernel)} bytes ({kernel_sectors} sectors, starting at sector {kernel_start_sector})")

    # Check kernel doesn't overlap filesystem
    kernel_end_sector = kernel_start_sector + kernel_sectors
    if kernel_end_sector > args.fs_start:
        print(f"ERROR: Kernel ends at sector {kernel_end_sector}, which overlaps "
              f"filesystem at sector {args.fs_start}", file=sys.stderr)
        sys.exit(1)

    # Patch stage2 with kernel location info
    if len(stage2) >= 8:
        stage2_bytes = bytearray(stage2)
        struct.pack_into("<H", stage2_bytes, 2, kernel_sectors)
        struct.pack_into("<I", stage2_bytes, 4, kernel_start_sector)
        stage2 = bytes(stage2_bytes)

    # Create image
    image = bytearray(image_size)

    # Write Stage 1 at sector 0
    image[0:len(stage1)] = stage1

    # Write Stage 2 at sector 1
    stage2_offset = SECTOR_SIZE
    image[stage2_offset:stage2_offset + len(stage2)] = stage2

    # Write kernel flat binary at sector 64
    kernel_offset = kernel_start_sector * SECTOR_SIZE
    image[kernel_offset:kernel_offset + len(kernel)] = kernel

    # Create FAT16 filesystem
    fs_sector_count = (image_size // SECTOR_SIZE) - args.fs_start
    print(f"\nFAT16 filesystem:")
    print(f"  Start sector: {args.fs_start} (offset 0x{args.fs_start * SECTOR_SIZE:X})")
    print(f"  Size: {fs_sector_count} sectors ({fs_sector_count * SECTOR_SIZE // (1024*1024)} MiB)")

    fat = Fat16Formatter(image, args.fs_start, fs_sector_count)
    fat.write_boot_sector()
    fat.init_fat()

    # Populate filesystem from sysroot
    if args.sysroot:
        print(f"  Populating from sysroot: {args.sysroot}")
        fat.populate_from_sysroot(args.sysroot)

    # Write image
    with open(args.output, "wb") as f:
        f.write(image)

    print(f"\nDisk image created: {args.output} ({args.image_size} MiB)")


if __name__ == "__main__":
    main()
