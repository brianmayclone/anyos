/*
 * gpt.c — GPT partition table creation for anyOS disk image builder
 *
 * Implements protective MBR, primary and backup GPT headers, and partition
 * entry arrays.  GUIDs are stored in the mixed-endian format mandated by
 * the UEFI specification: the first three fields are little-endian and the
 * last two fields are big-endian (stored as raw bytes).
 *
 * Written in C99 for TCC compatibility.
 */

#include "mkimage.h"

/* ── GUID helpers ─────────────────────────────────────────────────────── */

/*
 * guid_esp — EFI System Partition type GUID
 *
 * Canonical form: C12A7328-F81F-11D2-BA4B-00A0C93EC93B
 *
 * Mixed-endian layout in the 16-byte buffer:
 *   bytes  0- 3 : time_low       = 0xC12A7328  (little-endian)
 *   bytes  4- 5 : time_mid       = 0xF81F      (little-endian)
 *   bytes  6- 7 : time_hi_ver    = 0x11D2      (little-endian)
 *   bytes  8-15 : clock_seq + node               (big-endian / raw)
 *                 0xBA, 0x4B, 0x00, 0xA0, 0xC9, 0x3E, 0xC9, 0x3B
 */
void guid_esp(uint8_t *out)
{
    /* time_low: C12A7328 (LE32) */
    write_le32(out + 0, 0xC12A7328u);
    /* time_mid: F81F (LE16) */
    write_le16(out + 4, 0xF81Fu);
    /* time_hi_and_version: 11D2 (LE16) */
    write_le16(out + 6, 0x11D2u);
    /* clock_seq_hi, clock_seq_low, node[6] — stored as raw bytes (BE) */
    out[8]  = 0xBA;
    out[9]  = 0x4B;
    out[10] = 0x00;
    out[11] = 0xA0;
    out[12] = 0xC9;
    out[13] = 0x3E;
    out[14] = 0xC9;
    out[15] = 0x3B;
}

/*
 * guid_basic_data — Basic Data partition type GUID
 *
 * Canonical form: EBD0A0A2-B9E5-4433-87C0-68B6B72699C7
 */
void guid_basic_data(uint8_t *out)
{
    /* time_low: EBD0A0A2 (LE32) */
    write_le32(out + 0, 0xEBD0A0A2u);
    /* time_mid: B9E5 (LE16) */
    write_le16(out + 4, 0xB9E5u);
    /* time_hi_and_version: 4433 (LE16) */
    write_le16(out + 6, 0x4433u);
    /* clock_seq_hi, clock_seq_low, node[6] — stored as raw bytes (BE) */
    out[8]  = 0x87;
    out[9]  = 0xC0;
    out[10] = 0x68;
    out[11] = 0xB6;
    out[12] = 0xB7;
    out[13] = 0x26;
    out[14] = 0x99;
    out[15] = 0xC7;
}

/*
 * guid_random — Generate a pseudo-random version-4 GUID using rand().
 *
 * The caller is responsible for seeding rand() before the first call.
 * Byte layout follows mixed-endian GPT convention: the raw 16 bytes are
 * filled with random data, then the version/variant nibbles are fixed up.
 */
void guid_random(uint8_t *out)
{
    int i;
    for (i = 0; i < 16; i++) {
        out[i] = (uint8_t)(rand() & 0xFF);
    }
    /* Version 4: top nibble of byte 6 = 0x4 */
    out[6] = (uint8_t)((out[6] & 0x0Fu) | 0x40u);
    /* Variant 1 (RFC 4122): top two bits of byte 8 = 10b */
    out[8] = (uint8_t)((out[8] & 0x3Fu) | 0x80u);
}

/* ── Protective MBR ───────────────────────────────────────────────────── */

/*
 * write_protective_mbr — Write a GUID Protective MBR at offset 0 of image.
 *
 * Partition entry 1 (offset 446) covers the entire disk with type 0xEE so
 * that legacy BIOS tools recognise the disk as "in use" and do not
 * accidentally overwrite it.  The remaining three partition entries are
 * zeroed.
 */
void write_protective_mbr(uint8_t *image, uint64_t total_sectors)
{
    uint8_t *entry = image + 446;   /* first partition entry */
    uint64_t prot_size;

    /* Zero the four partition entries and the boot signature area */
    memset(entry, 0, 64 + 2);

    /* Byte 0 : boot indicator — 0x00 (not bootable) */
    entry[0] = 0x00;

    /* Bytes 1-3: CHS start = 0x000200 (head 0, sector 2, cylinder 0) */
    entry[1] = 0x00;
    entry[2] = 0x02;
    entry[3] = 0x00;

    /* Byte 4 : partition type = 0xEE (GPT protective) */
    entry[4] = 0xEE;

    /* Bytes 5-7: CHS end = 0xFFFFFF (clamped maximum) */
    entry[5] = 0xFF;
    entry[6] = 0xFF;
    entry[7] = 0xFF;

    /* Bytes 8-11: starting LBA = 1 (LE32) */
    write_le32(entry + 8, 1u);

    /* Bytes 12-15: size in sectors, clamped to 0xFFFFFFFF (LE32) */
    prot_size = total_sectors - 1u;
    if (prot_size > 0xFFFFFFFFu) {
        prot_size = 0xFFFFFFFFu;
    }
    write_le32(entry + 12, (uint32_t)prot_size);

    /* Boot signature */
    image[510] = 0x55;
    image[511] = 0xAA;
}

/* ── GPT header + entries ─────────────────────────────────────────────── */

/*
 * write_gpt_header — Serialise one GPT header into a 512-byte sector buffer.
 *
 * Parameters:
 *   buf          — pointer to the 512-byte sector (already zeroed by caller)
 *   my_lba       — LBA of this header
 *   alt_lba      — LBA of the alternate header
 *   first_usable — first LBA available for partitions
 *   last_usable  — last  LBA available for partitions
 *   disk_guid    — 16-byte disk GUID
 *   entries_lba  — LBA where the partition entry array begins
 *   entries_crc  — CRC32 of the partition entry array
 */
static void write_gpt_header(uint8_t *buf,
                             uint64_t my_lba,
                             uint64_t alt_lba,
                             uint64_t first_usable,
                             uint64_t last_usable,
                             const uint8_t *disk_guid,
                             uint64_t entries_lba,
                             uint32_t entries_crc)
{
    uint32_t hdr_crc;

    /* Signature: "EFI PART" */
    memcpy(buf + 0, "EFI PART", 8);

    /* Revision: 1.0 = 0x00010000 */
    write_le32(buf + 8,  0x00010000u);

    /* Header size: 92 bytes */
    write_le32(buf + 12, GPT_HEADER_SIZE);

    /* Header CRC32: zeroed before computation */
    write_le32(buf + 16, 0u);

    /* Reserved */
    write_le32(buf + 20, 0u);

    /* My LBA */
    write_le64(buf + 24, my_lba);

    /* Alternate header LBA */
    write_le64(buf + 32, alt_lba);

    /* First and last usable LBAs */
    write_le64(buf + 40, first_usable);
    write_le64(buf + 48, last_usable);

    /* Disk GUID */
    memcpy(buf + 56, disk_guid, 16);

    /* Starting LBA of partition entries */
    write_le64(buf + 72, entries_lba);

    /* Number of partition entries */
    write_le32(buf + 80, GPT_ENTRY_COUNT);

    /* Size of each partition entry */
    write_le32(buf + 84, GPT_ENTRY_SIZE);

    /* CRC32 of partition entries */
    write_le32(buf + 88, entries_crc);

    /* Compute header CRC over first 92 bytes (CRC field already zero) */
    hdr_crc = crc32(buf, GPT_HEADER_SIZE);
    write_le32(buf + 16, hdr_crc);
}

/*
 * create_gpt — Write the complete GPT structure into image.
 *
 * Layout:
 *   LBA 0                     : Protective MBR
 *   LBA 1                     : Primary GPT header
 *   LBA 2 .. LBA 33           : Primary partition entry array (32 sectors)
 *   LBA 34 .. last_usable     : Partition data
 *   LBA (total-33)..(total-2) : Backup partition entry array
 *   LBA (total-1)             : Backup GPT header
 */
void create_gpt(uint8_t *image, uint64_t total_sectors,
                const GptPartition *parts, int nparts)
{
    /* Seed for reproducible unique GUIDs */
    srand(0x414E594Fu);

    uint8_t  disk_guid[16];
    guid_random(disk_guid);

    /* Each partition entry is GPT_ENTRY_SIZE (128) bytes.
     * The entry array occupies GPT_ENTRY_COUNT * GPT_ENTRY_SIZE bytes.
     * entry_sectors = ceil(128 * 128 / 512) = 32 sectors.               */
    const uint32_t entry_sectors =
        (GPT_ENTRY_COUNT * GPT_ENTRY_SIZE + (SECTOR_SIZE - 1)) / SECTOR_SIZE;

    const uint64_t first_usable_lba = 2u + entry_sectors;          /* LBA 34 */
    const uint64_t last_usable_lba  =
        total_sectors - 1u - entry_sectors - 1u;

    /* ── Build the partition entry array ─────────────────────────────── */

    const size_t entries_bytes = (size_t)GPT_ENTRY_COUNT * GPT_ENTRY_SIZE;
    uint8_t *entries = (uint8_t *)calloc(1, entries_bytes);
    if (!entries) {
        fatal("create_gpt: out of memory for partition entries");
    }

    int i;
    for (i = 0; i < nparts && i < GPT_ENTRY_COUNT; i++) {
        uint8_t *e = entries + (size_t)i * GPT_ENTRY_SIZE;
        const GptPartition *p = &parts[i];
        const char *nm;
        int j;

        /* Type GUID (16 bytes) */
        memcpy(e + 0, p->type_guid, 16);

        /* Unique/partition GUID (16 bytes) */
        memcpy(e + 16, p->unique_guid, 16);

        /* First LBA (LE64) */
        write_le64(e + 32, p->first_lba);

        /* Last LBA (LE64) */
        write_le64(e + 40, p->last_lba);

        /* Attributes (LE64) — 0 for ordinary partitions */
        write_le64(e + 48, 0u);

        /* Name as UTF-16LE, up to 36 code units (72 bytes), NUL-terminated */
        nm = p->name ? p->name : "";
        for (j = 0; j < 36 && nm[j] != '\0'; j++) {
            write_le16(e + 56 + (size_t)j * 2, (uint16_t)(unsigned char)nm[j]);
        }
        /* Remaining name bytes are already zero from calloc */
    }

    uint32_t entries_crc = crc32(entries, entries_bytes);

    /* ── Primary GPT header (LBA 1) ─────────────────────────────────── */

    uint8_t *primary_hdr = image + 1u * SECTOR_SIZE;
    memset(primary_hdr, 0, SECTOR_SIZE);
    write_gpt_header(primary_hdr,
                     /*my_lba=*/      1u,
                     /*alt_lba=*/     total_sectors - 1u,
                     first_usable_lba,
                     last_usable_lba,
                     disk_guid,
                     /*entries_lba=*/ 2u,
                     entries_crc);

    /* ── Primary partition entry array (LBA 2..33) ───────────────────── */

    memcpy(image + 2u * SECTOR_SIZE, entries, entries_bytes);

    /* ── Backup partition entry array (last_usable+1 .. total-2) ─────── */

    uint64_t backup_entries_lba = total_sectors - 1u - entry_sectors;
    memcpy(image + backup_entries_lba * SECTOR_SIZE, entries, entries_bytes);

    /* ── Backup GPT header (last LBA) ────────────────────────────────── */

    uint8_t *backup_hdr = image + (total_sectors - 1u) * SECTOR_SIZE;
    memset(backup_hdr, 0, SECTOR_SIZE);
    write_gpt_header(backup_hdr,
                     /*my_lba=*/      total_sectors - 1u,
                     /*alt_lba=*/     1u,
                     first_usable_lba,
                     last_usable_lba,
                     disk_guid,
                     /*entries_lba=*/ backup_entries_lba,
                     entries_crc);

    free(entries);

    /* ── Print partition table summary ───────────────────────────────── */

    printf("GPT: %d partition(s), disk=%016llx%016llx\n",
           nparts,
           (unsigned long long)(
               ((uint64_t)disk_guid[0]  << 56) |
               ((uint64_t)disk_guid[1]  << 48) |
               ((uint64_t)disk_guid[2]  << 40) |
               ((uint64_t)disk_guid[3]  << 32) |
               ((uint64_t)disk_guid[4]  << 24) |
               ((uint64_t)disk_guid[5]  << 16) |
               ((uint64_t)disk_guid[6]  <<  8) |
               ((uint64_t)disk_guid[7])),
           (unsigned long long)(
               ((uint64_t)disk_guid[8]  << 56) |
               ((uint64_t)disk_guid[9]  << 48) |
               ((uint64_t)disk_guid[10] << 40) |
               ((uint64_t)disk_guid[11] << 32) |
               ((uint64_t)disk_guid[12] << 24) |
               ((uint64_t)disk_guid[13] << 16) |
               ((uint64_t)disk_guid[14] <<  8) |
               ((uint64_t)disk_guid[15])));

    for (i = 0; i < nparts; i++) {
        const GptPartition *p = &parts[i];
        printf("  [%d] \"%s\" LBA %llu..%llu (%llu sectors)\n",
               i,
               p->name ? p->name : "",
               (unsigned long long)p->first_lba,
               (unsigned long long)p->last_lba,
               (unsigned long long)(p->last_lba - p->first_lba + 1u));
    }
}
