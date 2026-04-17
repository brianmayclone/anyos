#!/usr/bin/env python3
# Copyright (c) 2026 Mike Strathmann
# SPDX-License-Identifier: MIT

"""
anyos_append_corefs.py — append a CoreFS system partition to an existing
anyOS disk image.

The existing image is left strictly unchanged up to its current end-of-
file: we never touch the boot sectors, the kernel region, or the
exFAT filesystem that mkimage already wrote.  The script only:

  1. **Extends** the image by `--size` bytes (os.truncate).
  2. **Writes** a new MBR partition entry (slot 1) pointing at the
     appended region with partition type 0xCF (CoreFS).
  3. **Invokes** `mkfs-corefs-host` on that region so the fresh space
     contains a valid CoreFS superblock / bitmap / inode table /
     journal from the very first boot.

This is intentionally a post-processing step rather than a mkimage
feature — the production mkimage builder is a C program
(`buildsystem/mkimage/src/mkimage.c`) and `tools/__mkimage.py` is the
legacy Python version.  Appending the CoreFS partition from outside
keeps both mkimage variants untouched.

Usage (from CMake or the command line):

    anyos_append_corefs.py \\
        --image <path-to-anyos.img> \\
        --size 128M \\
        --mkfs-corefs-host <path-to-mkfs-corefs-host binary> \\
        [--label system] [--slot 1]

Exits with a non-zero status on any error; prints a concise progress
line per step so the surrounding build log stays readable.
"""

import argparse
import os
import struct
import subprocess
import sys
from pathlib import Path

SECTOR_SIZE = 512

# Byte positions inside sector 0 (MBR).
MBR_PARTITION_TABLE_OFFSET = 0x1BE
MBR_PARTITION_ENTRY_SIZE = 16
MBR_PARTITION_SLOTS = 4

# Partition type bytes recognised by the anyOS kernel scanner
# (see kernel/src/fs/partition.rs).
MBR_TYPE_EXFAT = 0x07
MBR_TYPE_COREFS = 0xCF


# ---------------------------------------------------------------------------
# Size parsing (accepts k/K/m/M/g/G suffixes, matching mkfs-corefs-host).
# ---------------------------------------------------------------------------

def parse_size(value: str) -> int:
    """Parse a size spec like `128M`, `4G`, `8192000`. Returns bytes."""
    v = value.strip()
    if not v:
        raise argparse.ArgumentTypeError("empty size")
    mult = 1
    if v[-1] in "kKmMgGtT":
        unit = v[-1].lower()
        v = v[:-1]
        mult = {"k": 1024, "m": 1024 ** 2, "g": 1024 ** 3, "t": 1024 ** 4}[unit]
    try:
        n = int(v)
    except ValueError as exc:
        raise argparse.ArgumentTypeError(f"not a number: {value}") from exc
    if n <= 0:
        raise argparse.ArgumentTypeError(f"size must be > 0: {value}")
    return n * mult


# ---------------------------------------------------------------------------
# MBR partition-entry helpers
# ---------------------------------------------------------------------------

def _chs_encode(lba: int) -> bytes:
    """Encode an LBA into the 3-byte MBR CHS field.  Real CHS is
    obsolete; we clamp at the classic `(1023, 254, 63)` maximum like
    every modern partitioner does."""
    cylinder = min(lba // (255 * 63), 1023)
    head = (lba // 63) % 255
    sector = (lba % 63) + 1
    return bytes(
        (
            head & 0xFF,
            (sector & 0x3F) | ((cylinder >> 2) & 0xC0),
            cylinder & 0xFF,
        )
    )


def build_mbr_partition_entry(
    *,
    bootable: bool,
    ptype: int,
    start_lba: int,
    size_sectors: int,
) -> bytes:
    """Produce the 16-byte partition descriptor for one MBR slot."""
    if start_lba < 0 or size_sectors <= 0:
        raise ValueError(
            f"invalid partition extent: start_lba={start_lba}, size={size_sectors}"
        )
    if start_lba + size_sectors > 0xFFFFFFFF:
        raise ValueError(
            f"partition ends at LBA {start_lba + size_sectors}, beyond MBR's 32-bit limit"
        )

    entry = bytearray(MBR_PARTITION_ENTRY_SIZE)
    entry[0] = 0x80 if bootable else 0x00
    entry[1:4] = _chs_encode(start_lba)
    entry[4] = ptype & 0xFF
    entry[5:8] = _chs_encode(start_lba + size_sectors - 1)
    struct.pack_into("<I", entry, 8, start_lba)
    struct.pack_into("<I", entry, 12, size_sectors)
    return bytes(entry)


def read_mbr_partition_entry(image_path: Path, slot: int) -> dict:
    """Read one partition entry (bootable, ptype, start_lba, size_sectors)."""
    if not (0 <= slot < MBR_PARTITION_SLOTS):
        raise ValueError(f"MBR slot {slot} out of range")
    offset = MBR_PARTITION_TABLE_OFFSET + slot * MBR_PARTITION_ENTRY_SIZE
    with image_path.open("rb") as f:
        f.seek(offset)
        raw = f.read(MBR_PARTITION_ENTRY_SIZE)
    if len(raw) != MBR_PARTITION_ENTRY_SIZE:
        raise IOError(f"short read for MBR slot {slot} in {image_path}")
    return {
        "bootable": raw[0] == 0x80,
        "ptype": raw[4],
        "start_lba": struct.unpack_from("<I", raw, 8)[0],
        "size_sectors": struct.unpack_from("<I", raw, 12)[0],
    }


def write_mbr_partition_entry(image_path: Path, slot: int, entry: bytes) -> None:
    """Write one partition entry at the given slot (0..3)."""
    if not (0 <= slot < MBR_PARTITION_SLOTS):
        raise ValueError(f"MBR slot {slot} out of range")
    if len(entry) != MBR_PARTITION_ENTRY_SIZE:
        raise ValueError(
            f"expected {MBR_PARTITION_ENTRY_SIZE}-byte entry, got {len(entry)}"
        )
    offset = MBR_PARTITION_TABLE_OFFSET + slot * MBR_PARTITION_ENTRY_SIZE
    with image_path.open("r+b") as f:
        f.seek(offset)
        f.write(entry)


# ---------------------------------------------------------------------------
# Main operation
# ---------------------------------------------------------------------------

def append_corefs_partition(
    *,
    image: Path,
    size_bytes: int,
    slot: int,
    label: str,
    mkfs_corefs_host: Path,
) -> None:
    """Extend the image and format a fresh CoreFS volume in the appended space."""
    if not image.is_file():
        raise FileNotFoundError(f"image not found: {image}")
    if size_bytes % SECTOR_SIZE != 0:
        raise ValueError(
            f"--size must be a multiple of {SECTOR_SIZE} bytes (got {size_bytes})"
        )
    if not os.access(mkfs_corefs_host, os.X_OK):
        raise FileNotFoundError(
            f"mkfs-corefs-host not found or not executable: {mkfs_corefs_host}\n"
            f"  build via:  cd tools/mkfs-corefs-host && ./build.sh"
        )

    orig_size = image.stat().st_size
    if orig_size % SECTOR_SIZE != 0:
        raise ValueError(
            f"image size {orig_size} is not sector-aligned — refusing to append"
        )

    # Sanity check: the MBR slot we're about to use should currently be
    # empty (all zeros / ptype=0).  If it isn't, the caller already ran
    # this tool — bail out instead of silently truncating CoreFS.
    existing = read_mbr_partition_entry(image, slot)
    if existing["ptype"] != 0:
        raise RuntimeError(
            f"MBR slot {slot} is already populated (ptype=0x{existing['ptype']:02X}) — "
            f"refusing to overwrite. Rebuild the base image first."
        )

    new_size = orig_size + size_bytes
    start_lba = orig_size // SECTOR_SIZE
    size_sectors = size_bytes // SECTOR_SIZE

    print(
        f"anyos_append_corefs: {image.name} "
        f"({orig_size // (1024 * 1024)} MiB → {new_size // (1024 * 1024)} MiB)"
    )
    print(f"  appending CoreFS partition at LBA {start_lba}, {size_sectors} sectors "
          f"({size_bytes // (1024 * 1024)} MiB)")

    # Step 1: extend the image.
    with image.open("r+b") as f:
        f.truncate(new_size)

    # Step 2: write the MBR descriptor pointing at the appended region.
    entry = build_mbr_partition_entry(
        bootable=False,
        ptype=MBR_TYPE_COREFS,
        start_lba=start_lba,
        size_sectors=size_sectors,
    )
    write_mbr_partition_entry(image, slot, entry)
    print(f"  wrote MBR partition slot {slot} (type=0xCF, boot=0)")

    # Step 3: format the newly-reserved region as CoreFS.
    cmd = [
        str(mkfs_corefs_host),
        "--output", str(image),
        "--offset", str(orig_size),
        "--size", str(size_bytes),
        "--label", label,
    ]
    print(f"  invoking mkfs-corefs-host…")
    result = subprocess.run(cmd, check=False, capture_output=True, text=True)
    for line in result.stdout.splitlines():
        print(f"    | {line}")
    if result.returncode != 0:
        if result.stderr:
            print(result.stderr, file=sys.stderr)
        raise RuntimeError(f"mkfs-corefs-host exited with code {result.returncode}")

    print(f"anyos_append_corefs: done.")


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

def main() -> int:
    parser = argparse.ArgumentParser(
        description=(
            "Append a CoreFS system partition to an existing anyOS disk "
            "image.  Extends the image by --size bytes, writes a new MBR "
            "partition entry, and invokes mkfs-corefs-host on the region."
        ),
    )
    parser.add_argument("--image", required=True, type=Path,
                        help="Path to the existing anyOS disk image.")
    parser.add_argument("--size", required=True, type=parse_size,
                        help="Size of the CoreFS partition (accepts k/m/g/t suffix).")
    parser.add_argument("--mkfs-corefs-host", required=True, type=Path,
                        help="Path to the mkfs-corefs-host binary "
                             "(tools/mkfs-corefs-host/target/.../mkfs-corefs-host).")
    parser.add_argument("--label", default="system",
                        help="Volume label for the CoreFS partition (default: system).")
    parser.add_argument("--slot", type=int, default=1,
                        help="MBR partition slot to populate (0..3, default: 1).")
    args = parser.parse_args()

    try:
        append_corefs_partition(
            image=args.image,
            size_bytes=args.size,
            slot=args.slot,
            label=args.label,
            mkfs_corefs_host=args.mkfs_corefs_host,
        )
    except (FileNotFoundError, ValueError, IOError, RuntimeError) as e:
        print(f"anyos_append_corefs: ERROR: {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
