//! Database header and legacy v1 schema helpers.

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use crate::types::*;

#[derive(Debug, Clone, Copy)]
pub struct HeaderInfo {
    pub format: DbFormat,
    pub table_count: u32,
    pub first_free_page: u32,
    pub first_table_dir_page: u32,
}

/// Read the database file header and validate magic.
pub fn read_header(page: &[u8; PAGE_SIZE]) -> DbResult<HeaderInfo> {
    let format = if &page[0..8] == MAGIC_V1 {
        DbFormat::V1
    } else if &page[0..8] == MAGIC_V2 {
        DbFormat::V2
    } else {
        return Err(DbError::Corrupt(String::from("Invalid magic bytes")));
    };

    let table_count = u32::from_le_bytes([page[12], page[13], page[14], page[15]]);
    let first_free_page = u32::from_le_bytes([page[16], page[17], page[18], page[19]]);
    let first_table_dir_page = if format == DbFormat::V2 {
        u32::from_le_bytes([page[20], page[21], page[22], page[23]])
    } else {
        0
    };

    Ok(HeaderInfo {
        format,
        table_count,
        first_free_page,
        first_table_dir_page,
    })
}

/// Initialize a fresh page 0 with a v2 header and zero tables.
pub fn init_header(page: &mut [u8; PAGE_SIZE]) {
    page.fill(0);
    page[0..8].copy_from_slice(MAGIC_V2);
    let ps = (PAGE_SIZE as u32).to_le_bytes();
    page[8..12].copy_from_slice(&ps);
}

/// Write updated header fields back to page 0.
pub fn write_header_fields(
    page: &mut [u8; PAGE_SIZE],
    format: DbFormat,
    table_count: u32,
    first_free_page: u32,
    first_table_dir_page: u32,
) {
    page[0..8].copy_from_slice(match format {
        DbFormat::V1 => MAGIC_V1,
        DbFormat::V2 => MAGIC_V2,
    });
    page[12..16].copy_from_slice(&table_count.to_le_bytes());
    page[16..20].copy_from_slice(&first_free_page.to_le_bytes());
    page[20..24].copy_from_slice(&first_table_dir_page.to_le_bytes());
}

/// Read all table schemas from a legacy v1 page0 directory.
pub fn read_tables_v1(page: &[u8; PAGE_SIZE], table_count: u32) -> DbResult<Vec<TableSchema>> {
    let mut tables = Vec::with_capacity(table_count as usize);
    for i in 0..table_count as usize {
        let off = HEADER_SIZE + i * TABLE_ENTRY_SIZE;
        if off + TABLE_ENTRY_SIZE > PAGE_SIZE {
            return Err(DbError::Corrupt(String::from("Table directory exceeds page 0")));
        }
        tables.push(read_v1_table_entry(&page[off..off + TABLE_ENTRY_SIZE])?);
    }
    Ok(tables)
}

/// Find a table by name (case-insensitive) in the schema list. Returns index.
pub fn find_table(tables: &[TableSchema], name: &str) -> Option<usize> {
    tables.iter().position(|t| t.name.eq_ignore_ascii_case(name))
}

fn read_v1_table_entry(entry: &[u8]) -> DbResult<TableSchema> {
    let name_end = entry[0..32].iter().position(|&b| b == 0).unwrap_or(32);
    let name = core::str::from_utf8(&entry[0..name_end])
        .map_err(|_| DbError::Corrupt(String::from("Invalid table name encoding")))?;

    let col_count = u16::from_le_bytes([entry[32], entry[33]]) as usize;
    let row_count = u32::from_le_bytes([entry[36], entry[37], entry[38], entry[39]]);
    let first_data_page = u32::from_le_bytes([entry[40], entry[41], entry[42], entry[43]]);

    if col_count > MAX_INLINE_COLUMNS {
        return Err(DbError::Corrupt(String::from("Legacy v1 column count exceeds inline limit")));
    }

    const COL_ENTRY_SIZE: usize = 26;
    const COL_NAME_BYTES: usize = 24;

    let mut columns = Vec::with_capacity(col_count);
    for c in 0..col_count {
        let coff = 48 + c * COL_ENTRY_SIZE;
        let cname_end = entry[coff..coff + COL_NAME_BYTES]
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(COL_NAME_BYTES);
        let cname = core::str::from_utf8(&entry[coff..coff + cname_end])
            .map_err(|_| DbError::Corrupt(String::from("Invalid column name encoding")))?;
        let ctype_raw =
            u16::from_le_bytes([entry[coff + COL_NAME_BYTES], entry[coff + COL_NAME_BYTES + 1]]);
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
        indexes: Vec::new(),
        row_count,
        first_data_page,
        schema_page: 0,
        dir_page: 0,
    })
}
