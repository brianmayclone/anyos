//! Table schema directory management.
//!
//! Reads and writes the table directory stored in page 0 of the database file.
//! Each table entry is 128 bytes containing the table name, column definitions,
//! row count, and first data page pointer.

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use crate::types::*;

// ── Page 0 layout ────────────────────────────────────────────────────────────
//
// Bytes  0..8    magic "ANYDB100"
// Bytes  8..12   page_size (u32 LE, always 4096)
// Bytes 12..16   table_count (u32 LE)
// Bytes 16..20   first_free_page (u32 LE, 0 = none)
// Bytes 20..32   reserved (zeroed)
// Bytes 32..4096 table directory entries (128 bytes each, max 31)
//
// ── Table entry layout (128 bytes) ──────────────────────────────────────────
//
// Bytes  0..32   name (null-terminated ASCII)
// Bytes 32..34   col_count (u16 LE)
// Bytes 34..36   reserved (u16)
// Bytes 36..40   row_count (u32 LE)
// Bytes 40..44   first_data_page (u32 LE, page number, 0 = none)
// Bytes 44..48   reserved
// Bytes 48..128  columns: up to 8 entries of 10 bytes each = 80 bytes
//                column entry: name[8] (null-terminated) + col_type (u16 LE)

/// Read the database file header and validate magic.
pub fn read_header(page: &[u8; PAGE_SIZE]) -> DbResult<(u32, u32)> {
    if &page[0..8] != MAGIC {
        return Err(DbError::Corrupt(String::from("Invalid magic bytes")));
    }
    let table_count = u32::from_le_bytes([page[12], page[13], page[14], page[15]]);
    let first_free = u32::from_le_bytes([page[16], page[17], page[18], page[19]]);
    Ok((table_count, first_free))
}

/// Initialize a fresh page 0 with header and zero tables.
pub fn init_header(page: &mut [u8; PAGE_SIZE]) {
    page.fill(0);
    page[0..8].copy_from_slice(MAGIC);
    let ps = (PAGE_SIZE as u32).to_le_bytes();
    page[8..12].copy_from_slice(&ps);
    // table_count = 0, first_free_page = 0, rest zeroed
}

/// Write updated header fields (table_count, first_free_page) back to page 0.
pub fn write_header_fields(page: &mut [u8; PAGE_SIZE], table_count: u32, first_free: u32) {
    page[12..16].copy_from_slice(&table_count.to_le_bytes());
    page[16..20].copy_from_slice(&first_free.to_le_bytes());
}

/// Read all table schemas from page 0.
pub fn read_tables(page: &[u8; PAGE_SIZE], table_count: u32) -> DbResult<Vec<TableSchema>> {
    let count = table_count as usize;
    if count > MAX_TABLES {
        return Err(DbError::Corrupt(String::from("Table count exceeds maximum")));
    }
    let mut tables = Vec::with_capacity(count);
    for i in 0..count {
        let off = HEADER_SIZE + i * TABLE_ENTRY_SIZE;
        tables.push(read_table_entry(&page[off..off + TABLE_ENTRY_SIZE])?);
    }
    Ok(tables)
}

/// Read a single table schema entry from a 128-byte slice.
fn read_table_entry(entry: &[u8]) -> DbResult<TableSchema> {
    // Name: bytes 0..32, null-terminated
    let name_end = entry[0..32].iter().position(|&b| b == 0).unwrap_or(32);
    let name = core::str::from_utf8(&entry[0..name_end])
        .map_err(|_| DbError::Corrupt(String::from("Invalid table name encoding")))?;

    let col_count = u16::from_le_bytes([entry[32], entry[33]]) as usize;
    let row_count = u32::from_le_bytes([entry[36], entry[37], entry[38], entry[39]]);
    let first_data_page = u32::from_le_bytes([entry[40], entry[41], entry[42], entry[43]]);

    if col_count > MAX_COLUMNS {
        return Err(DbError::Corrupt(String::from("Column count exceeds maximum")));
    }

    let mut columns = Vec::with_capacity(col_count);
    for c in 0..col_count {
        let coff = 48 + c * 10;
        let cname_end = entry[coff..coff + 8].iter().position(|&b| b == 0).unwrap_or(8);
        let cname = core::str::from_utf8(&entry[coff..coff + cname_end])
            .map_err(|_| DbError::Corrupt(String::from("Invalid column name encoding")))?;
        let ctype_raw = u16::from_le_bytes([entry[coff + 8], entry[coff + 9]]);
        let col_type = ColumnType::from_u16(ctype_raw)
            .ok_or_else(|| DbError::Corrupt(String::from("Invalid column type")))?;
        columns.push(ColumnDef {
            name: String::from(cname),
            col_type,
        });
    }

    Ok(TableSchema {
        name: String::from(name),
        columns,
        row_count,
        first_data_page,
    })
}

/// Write a table schema entry into a 128-byte region of page 0.
pub fn write_table_entry(page: &mut [u8; PAGE_SIZE], index: usize, schema: &TableSchema) {
    let off = HEADER_SIZE + index * TABLE_ENTRY_SIZE;
    let entry = &mut page[off..off + TABLE_ENTRY_SIZE];
    entry.fill(0);

    // Name
    let name_bytes = schema.name.as_bytes();
    let nlen = name_bytes.len().min(MAX_TABLE_NAME);
    entry[0..nlen].copy_from_slice(&name_bytes[..nlen]);

    // Column count
    let cc = (schema.columns.len() as u16).to_le_bytes();
    entry[32..34].copy_from_slice(&cc);

    // Row count
    entry[36..40].copy_from_slice(&schema.row_count.to_le_bytes());

    // First data page
    entry[40..44].copy_from_slice(&schema.first_data_page.to_le_bytes());

    // Columns
    for (c, col) in schema.columns.iter().enumerate() {
        if c >= MAX_COLUMNS { break; }
        let coff = 48 + c * 10;
        let cb = col.name.as_bytes();
        let clen = cb.len().min(MAX_COL_NAME);
        entry[coff..coff + clen].copy_from_slice(&cb[..clen]);
        let ct = (col.col_type as u16).to_le_bytes();
        entry[coff + 8..coff + 10].copy_from_slice(&ct);
    }
}

/// Clear a table entry (fill with zeros).
pub fn clear_table_entry(page: &mut [u8; PAGE_SIZE], index: usize) {
    let off = HEADER_SIZE + index * TABLE_ENTRY_SIZE;
    page[off..off + TABLE_ENTRY_SIZE].fill(0);
}

/// Find a table by name (case-insensitive) in the schema list. Returns index.
pub fn find_table(tables: &[TableSchema], name: &str) -> Option<usize> {
    tables.iter().position(|t| t.name.eq_ignore_ascii_case(name))
}
