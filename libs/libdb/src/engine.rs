//! Page-based storage engine.
//!
//! Manages the on-disk database file: page I/O, row serialization,
//! table scanning, row insertion, deletion, and page allocation.
//! File I/O uses the `syscall` module (same pattern as libanyui).

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use crate::types::*;
use crate::schema;
use crate::syscall;

// ── Database handle ──────────────────────────────────────────────────────────

/// An open database file.
pub struct Database {
    fd: u32,
    /// Cached copy of page 0 (header + table directory).
    page0: [u8; PAGE_SIZE],
    /// Parsed table schemas (in sync with page0).
    pub tables: Vec<TableSchema>,
    /// Number of tables.
    pub table_count: u32,
    /// First free page (for page reuse — 0 = none, allocate at end).
    first_free_page: u32,
    /// Total pages in the file.
    total_pages: u32,
    /// Last error message.
    pub last_error: String,
}

impl Database {
    /// Open or create a database file.
    pub fn open(path: &str) -> DbResult<Database> {
        // Try to open existing file first
        let fd = syscall::open(path, 0); // read-only probe
        let file_exists = fd != u32::MAX;
        let file_size = if file_exists { syscall::file_size(fd) } else { 0 };
        if file_exists {
            syscall::close(fd);
        }

        if file_exists && file_size >= PAGE_SIZE as u32 {
            // File exists with valid content — open for read+write
            let fd = syscall::open(path, syscall::O_WRITE);
            if fd == u32::MAX {
                return Err(DbError::Io(String::from("Cannot open database for writing")));
            }
            let mut db = Database {
                fd,
                page0: [0u8; PAGE_SIZE],
                tables: Vec::new(),
                table_count: 0,
                first_free_page: 0,
                total_pages: 0,
                last_error: String::new(),
            };
            db.load_page0()?;
            Ok(db)
        } else {
            // File does not exist or is empty — create/initialize new database
            let flags = if file_exists {
                syscall::O_WRITE | syscall::O_TRUNC
            } else {
                syscall::O_WRITE | syscall::O_CREATE | syscall::O_TRUNC
            };
            let fd = syscall::open(path, flags);
            if fd == u32::MAX {
                return Err(DbError::Io(String::from("Cannot create database file")));
            }
            let mut db = Database {
                fd,
                page0: [0u8; PAGE_SIZE],
                tables: Vec::new(),
                table_count: 0,
                first_free_page: 0,
                total_pages: 1,
                last_error: String::new(),
            };
            schema::init_header(&mut db.page0);
            db.write_page(0, &db.page0.clone())?;
            Ok(db)
        }
    }

    /// Close the database (flush and release fd).
    pub fn close(&mut self) {
        if self.fd != u32::MAX {
            syscall::close(self.fd);
            self.fd = u32::MAX;
        }
    }

    // ── Page I/O ─────────────────────────────────────────────────────────

    /// Read a page from disk into buffer.
    fn read_page(&self, page_num: u32, buf: &mut [u8; PAGE_SIZE]) -> DbResult<()> {
        let offset = page_num as i32 * PAGE_SIZE as i32;
        if syscall::lseek(self.fd, offset, syscall::SEEK_SET) == u32::MAX {
            return Err(DbError::Io(String::from("Seek failed")));
        }
        let n = syscall::read(self.fd, buf);
        if n == u32::MAX {
            return Err(DbError::Io(String::from("Read failed")));
        }
        // Zero-fill if we read less than a full page (new pages)
        if (n as usize) < PAGE_SIZE {
            buf[n as usize..].fill(0);
        }
        Ok(())
    }

    /// Write a page to disk.
    fn write_page(&self, page_num: u32, buf: &[u8; PAGE_SIZE]) -> DbResult<()> {
        let offset = page_num as i32 * PAGE_SIZE as i32;
        if syscall::lseek(self.fd, offset, syscall::SEEK_SET) == u32::MAX {
            return Err(DbError::Io(String::from("Seek failed")));
        }
        let n = syscall::write(self.fd, buf);
        if n == u32::MAX || n as usize != PAGE_SIZE {
            return Err(DbError::Io(String::from("Write failed")));
        }
        Ok(())
    }

    /// Load page 0 and parse table directory.
    fn load_page0(&mut self) -> DbResult<()> {
        // Inline page read to avoid borrow conflict (self vs self.page0)
        if syscall::lseek(self.fd, 0, syscall::SEEK_SET) == u32::MAX {
            return Err(DbError::Io(String::from("Seek failed")));
        }
        let n = syscall::read(self.fd, &mut self.page0);
        if n == u32::MAX {
            return Err(DbError::Io(String::from("Read failed")));
        }
        if (n as usize) < PAGE_SIZE {
            self.page0[n as usize..].fill(0);
        }
        let (tc, ff) = schema::read_header(&self.page0)?;
        self.table_count = tc;
        self.first_free_page = ff;
        self.tables = schema::read_tables(&self.page0, tc)?;
        let file_size = syscall::file_size(self.fd);
        self.total_pages = if file_size > 0 {
            (file_size as usize / PAGE_SIZE).max(1) as u32
        } else {
            1
        };
        Ok(())
    }

    /// Flush page 0 to disk (after schema changes).
    fn flush_page0(&mut self) -> DbResult<()> {
        schema::write_header_fields(&mut self.page0, self.table_count, self.first_free_page);
        for (i, table) in self.tables.iter().enumerate() {
            schema::write_table_entry(&mut self.page0, i, table);
        }
        self.write_page(0, &self.page0.clone())
    }

    // ── Page allocation ──────────────────────────────────────────────────

    /// Allocate a new data page. Returns page number.
    fn alloc_page(&mut self) -> DbResult<u32> {
        if self.first_free_page != 0 {
            // Reuse a free page
            let page_num = self.first_free_page;
            let mut page = [0u8; PAGE_SIZE];
            self.read_page(page_num, &mut page)?;
            // Free page's first 4 bytes point to next free page
            self.first_free_page = u32::from_le_bytes([page[0], page[1], page[2], page[3]]);
            page.fill(0);
            self.write_page(page_num, &page)?;
            Ok(page_num)
        } else {
            // Allocate at end of file
            let page_num = self.total_pages;
            self.total_pages += 1;
            let page = [0u8; PAGE_SIZE];
            self.write_page(page_num, &page)?;
            Ok(page_num)
        }
    }

    /// Free a data page (add to free list).
    fn free_page(&mut self, page_num: u32) -> DbResult<()> {
        let mut page = [0u8; PAGE_SIZE];
        // Write next-free pointer as first 4 bytes
        page[0..4].copy_from_slice(&self.first_free_page.to_le_bytes());
        self.write_page(page_num, &page)?;
        self.first_free_page = page_num;
        Ok(())
    }

    // ── Table management ─────────────────────────────────────────────────

    /// Create a new table with the given schema.
    pub fn create_table(&mut self, name: &str, columns: &[ColumnDef]) -> DbResult<()> {
        if self.table_count as usize >= MAX_TABLES {
            return Err(DbError::TooManyTables);
        }
        if columns.len() > MAX_COLUMNS {
            return Err(DbError::TooManyColumns);
        }
        if schema::find_table(&self.tables, name).is_some() {
            return Err(DbError::TableExists(String::from(name)));
        }
        if name.len() > MAX_TABLE_NAME {
            return Err(DbError::Parse(String::from("Table name too long")));
        }
        for col in columns {
            if col.name.len() > MAX_COL_NAME {
                return Err(DbError::Parse(String::from("Column name too long")));
            }
        }

        let table = TableSchema {
            name: String::from(name),
            columns: columns.to_vec(),
            row_count: 0,
            first_data_page: 0,
        };
        self.tables.push(table);
        self.table_count += 1;
        self.flush_page0()
    }

    /// Drop a table by name, freeing all its data pages.
    pub fn drop_table(&mut self, name: &str) -> DbResult<()> {
        let idx = schema::find_table(&self.tables, name)
            .ok_or_else(|| DbError::TableNotFound(String::from(name)))?;

        // Free all data pages in the chain
        let mut page_num = self.tables[idx].first_data_page;
        while page_num != 0 {
            let mut page = [0u8; PAGE_SIZE];
            self.read_page(page_num, &mut page)?;
            let next = u32::from_le_bytes([page[0], page[1], page[2], page[3]]);
            self.free_page(page_num)?;
            page_num = next;
        }

        // Remove from schema list and compact
        self.tables.remove(idx);
        self.table_count -= 1;

        // Rewrite all entries in page0 (compact)
        for i in 0..MAX_TABLES {
            schema::clear_table_entry(&mut self.page0, i);
        }
        self.flush_page0()
    }

    // ── Row serialization ────────────────────────────────────────────────

    /// Serialize a row's values into bytes. Returns serialized data.
    pub fn serialize_row(values: &[Value]) -> DbResult<Vec<u8>> {
        let total_size: usize = values.iter().map(|v| v.serialized_size()).sum();
        // Row format: flag(1) + row_len(2) + value data
        let row_size = 1 + 2 + total_size;
        if row_size > DATA_AREA_SIZE {
            return Err(DbError::RowTooLarge);
        }

        let mut buf = Vec::with_capacity(row_size);
        buf.push(ROW_ACTIVE); // flag
        buf.push((total_size & 0xFF) as u8);       // row_len low
        buf.push(((total_size >> 8) & 0xFF) as u8); // row_len high

        for val in values {
            match val {
                Value::Null => buf.push(TAG_NULL),
                Value::Integer(v) => {
                    buf.push(TAG_INTEGER);
                    buf.extend_from_slice(&v.to_le_bytes());
                }
                Value::Text(s) => {
                    if s.len() > 255 {
                        return Err(DbError::ValueTooLarge);
                    }
                    buf.push(TAG_TEXT);
                    buf.push((s.len() & 0xFF) as u8);
                    buf.push(((s.len() >> 8) & 0xFF) as u8);
                    buf.extend_from_slice(s.as_bytes());
                }
            }
        }
        Ok(buf)
    }

    /// Deserialize a row from bytes at the given offset.
    /// Returns (row, bytes_consumed) or None if deleted/empty.
    pub fn deserialize_row(data: &[u8], offset: usize, col_count: usize) -> Option<(Row, usize)> {
        if offset >= data.len() { return None; }

        let flag = data[offset];
        if flag != ROW_ACTIVE && flag != ROW_DELETED { return None; }

        if offset + 3 > data.len() { return None; }
        let row_len = u16::from_le_bytes([data[offset + 1], data[offset + 2]]) as usize;
        let total = 3 + row_len;

        if flag == ROW_DELETED {
            return Some((Row { values: Vec::new() }, total));
        }

        let mut values = Vec::with_capacity(col_count);
        let mut pos = offset + 3;
        let end = offset + 3 + row_len;

        for _ in 0..col_count {
            if pos >= end { break; }
            match data[pos] {
                TAG_NULL => {
                    values.push(Value::Null);
                    pos += 1;
                }
                TAG_INTEGER => {
                    if pos + 9 > end { break; }
                    let v = i64::from_le_bytes([
                        data[pos + 1], data[pos + 2], data[pos + 3], data[pos + 4],
                        data[pos + 5], data[pos + 6], data[pos + 7], data[pos + 8],
                    ]);
                    values.push(Value::Integer(v));
                    pos += 9;
                }
                TAG_TEXT => {
                    if pos + 3 > end { break; }
                    let slen = u16::from_le_bytes([data[pos + 1], data[pos + 2]]) as usize;
                    if pos + 3 + slen > end { break; }
                    let s = core::str::from_utf8(&data[pos + 3..pos + 3 + slen]).unwrap_or("");
                    values.push(Value::Text(String::from(s)));
                    pos += 3 + slen;
                }
                _ => break,
            }
        }

        Some((Row { values }, total))
    }

    // ── Table scan ───────────────────────────────────────────────────────

    /// Scan all active rows of a table. Returns a Vec of (page_num, offset_in_page, Row).
    pub fn scan_table(&self, table_idx: usize) -> DbResult<Vec<(u32, usize, Row)>> {
        let table = &self.tables[table_idx];
        let col_count = table.columns.len();
        let mut results = Vec::new();
        let mut page_num = table.first_data_page;

        while page_num != 0 {
            let mut page = [0u8; PAGE_SIZE];
            self.read_page(page_num, &mut page)?;

            let next_page = u32::from_le_bytes([page[0], page[1], page[2], page[3]]);
            let data_end = u16::from_le_bytes([page[6], page[7]]) as usize;
            let data_end = if data_end == 0 { DATA_PAGE_HEADER } else { data_end };

            let mut offset = DATA_PAGE_HEADER;
            while offset < data_end {
                match Self::deserialize_row(&page, offset, col_count) {
                    Some((row, consumed)) => {
                        if !row.values.is_empty() {
                            results.push((page_num, offset, row));
                        }
                        offset += consumed;
                    }
                    None => break,
                }
            }

            page_num = next_page;
        }

        Ok(results)
    }

    // ── Row insertion ────────────────────────────────────────────────────

    /// Insert a row into a table. Updates page chain and row count.
    pub fn insert_row(&mut self, table_idx: usize, values: &[Value]) -> DbResult<()> {
        let row_data = Self::serialize_row(values)?;
        let row_len = row_data.len();

        let table = &self.tables[table_idx];
        let mut page_num = table.first_data_page;

        // Try to find a page with enough space
        let mut prev_page_num: u32 = 0;
        while page_num != 0 {
            let mut page = [0u8; PAGE_SIZE];
            self.read_page(page_num, &mut page)?;

            let data_end = u16::from_le_bytes([page[6], page[7]]) as usize;
            let data_end = if data_end == 0 { DATA_PAGE_HEADER } else { data_end };

            if data_end + row_len <= PAGE_SIZE {
                page[data_end..data_end + row_len].copy_from_slice(&row_data);
                let new_end = (data_end + row_len) as u16;
                page[6..8].copy_from_slice(&new_end.to_le_bytes());
                let rc = u16::from_le_bytes([page[4], page[5]]);
                page[4..6].copy_from_slice(&(rc + 1).to_le_bytes());
                self.write_page(page_num, &page)?;

                self.tables[table_idx].row_count += 1;
                self.flush_page0()?;
                return Ok(());
            }

            prev_page_num = page_num;
            page_num = u32::from_le_bytes([page[0], page[1], page[2], page[3]]);
        }

        // No existing page has space — allocate a new page
        let new_page_num = self.alloc_page()?;
        let mut new_page = [0u8; PAGE_SIZE];
        new_page[4..6].copy_from_slice(&1u16.to_le_bytes());
        let data_end = DATA_PAGE_HEADER + row_len;
        new_page[6..8].copy_from_slice(&(data_end as u16).to_le_bytes());
        new_page[DATA_PAGE_HEADER..DATA_PAGE_HEADER + row_len].copy_from_slice(&row_data);
        self.write_page(new_page_num, &new_page)?;

        if prev_page_num != 0 {
            let mut prev = [0u8; PAGE_SIZE];
            self.read_page(prev_page_num, &mut prev)?;
            prev[0..4].copy_from_slice(&new_page_num.to_le_bytes());
            self.write_page(prev_page_num, &prev)?;
        } else {
            self.tables[table_idx].first_data_page = new_page_num;
        }

        self.tables[table_idx].row_count += 1;
        self.flush_page0()
    }

    // ── Row deletion ─────────────────────────────────────────────────────

    /// Delete a row at a specific location (page_num, offset).
    pub fn delete_row(&mut self, table_idx: usize, page_num: u32, offset: usize) -> DbResult<()> {
        let mut page = [0u8; PAGE_SIZE];
        self.read_page(page_num, &mut page)?;

        page[offset] = ROW_DELETED;

        let rc = u16::from_le_bytes([page[4], page[5]]);
        if rc > 0 {
            page[4..6].copy_from_slice(&(rc - 1).to_le_bytes());
        }

        self.write_page(page_num, &page)?;

        if self.tables[table_idx].row_count > 0 {
            self.tables[table_idx].row_count -= 1;
        }
        self.flush_page0()
    }

    // ── Row update ───────────────────────────────────────────────────────

    /// Update a row: delete old + insert new. Simple but correct for v1.
    pub fn update_row(
        &mut self,
        table_idx: usize,
        page_num: u32,
        offset: usize,
        new_values: &[Value],
    ) -> DbResult<()> {
        self.delete_row(table_idx, page_num, offset)?;
        self.insert_row(table_idx, new_values)
    }
}

impl Drop for Database {
    fn drop(&mut self) {
        self.close();
    }
}
