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
use crate::index::{encode_index_key, RowLocation, TableIndexState};

// ── Page cache ──────────────────────────────────────────────────────────────

/// Maximum number of pages held in the cache.
const CACHE_CAPACITY: usize = 64;

/// A single cached page.
struct CachedPage {
    page_num: u32,
    data: [u8; PAGE_SIZE],
    dirty: bool,
}

impl Clone for CachedPage {
    fn clone(&self) -> Self {
        Self {
            page_num: self.page_num,
            data: self.data,
            dirty: self.dirty,
        }
    }
}

/// Simple page cache with LRU eviction.
/// Pages are heap-allocated (Box) so Vec operations only move 8-byte pointers,
/// not 4 KiB page buffers — prevents stack overflow during cache reshuffling.
struct PageCache {
    pages: Vec<alloc::boxed::Box<CachedPage>>,
    fd: u32,
    in_memory: bool,
}

impl PageCache {
    fn new(fd: u32, in_memory: bool) -> Self {
        PageCache { pages: Vec::with_capacity(CACHE_CAPACITY), fd, in_memory }
    }

    /// Look up a page in the cache. Returns a reference to the data if found.
    /// Moves the page to the front (MRU position).
    fn get(&mut self, page_num: u32) -> Option<&[u8; PAGE_SIZE]> {
        let pos = self.pages.iter().position(|p| p.page_num == page_num)?;
        if pos > 0 {
            let entry = self.pages.remove(pos); // moves Box (8 bytes), not 4 KiB
            self.pages.insert(0, entry);
        }
        Some(&self.pages[0].data)
    }

    /// Allocate a CachedPage on the heap without going through the stack.
    /// Uses alloc + ptr::write to avoid placing 4 KiB on the stack.
    fn alloc_page_box(page_num: u32, data: &[u8; PAGE_SIZE], dirty: bool) -> alloc::boxed::Box<CachedPage> {
        use alloc::alloc::{alloc, Layout};
        use core::ptr;
        unsafe {
            let layout = Layout::new::<CachedPage>();
            let raw = alloc(layout) as *mut CachedPage;
            if raw.is_null() {
                // OOM — shouldn't happen with reasonable cache sizes
                panic!("PageCache: alloc failed");
            }
            // Write fields directly into heap memory (no stack copy)
            ptr::write(&mut (*raw).page_num, page_num);
            ptr::write(&mut (*raw).dirty, dirty);
            ptr::copy_nonoverlapping(data.as_ptr(), (*raw).data.as_mut_ptr(), PAGE_SIZE);
            alloc::boxed::Box::from_raw(raw)
        }
    }

    /// Insert or update a page in the cache.
    /// If the cache is full, the LRU entry is evicted (dirty pages are written to disk).
    fn put(&mut self, page_num: u32, data: &[u8; PAGE_SIZE], dirty: bool) {
        // Check if already cached — update in place (copy into existing heap allocation)
        if let Some(pos) = self.pages.iter().position(|p| p.page_num == page_num) {
            // Copy directly into heap — no stack temp
            unsafe {
                core::ptr::copy_nonoverlapping(
                    data.as_ptr(),
                    self.pages[pos].data.as_mut_ptr(),
                    PAGE_SIZE,
                );
            }
            self.pages[pos].dirty = dirty || self.pages[pos].dirty;
            if pos > 0 {
                let entry = self.pages.remove(pos);
                self.pages.insert(0, entry);
            }
            return;
        }

        // Evict LRU if full
        if self.pages.len() >= CACHE_CAPACITY {
            let victim = self.pages.pop().unwrap();
            if victim.dirty && !self.in_memory {
                Self::write_to_disk(self.fd, victim.page_num, &victim.data);
            }
        }

        // Allocate on heap directly — never touches the stack with 4 KiB
        let boxed = Self::alloc_page_box(page_num, data, dirty);
        self.pages.insert(0, boxed);
    }

    /// Flush all dirty pages to disk.
    fn flush(&mut self) {
        if self.in_memory {
            for entry in &mut self.pages {
                entry.dirty = false;
            }
            return;
        }
        for entry in &mut self.pages {
            if entry.dirty {
                Self::write_to_disk(self.fd, entry.page_num, &entry.data);
                entry.dirty = false;
            }
        }
    }

    /// Invalidate a specific page (e.g. after free_page).
    fn invalidate(&mut self, page_num: u32) {
        if let Some(pos) = self.pages.iter().position(|p| p.page_num == page_num) {
            self.pages.remove(pos);
        }
    }

    fn clear(&mut self) {
        self.pages.clear();
    }

    /// Write a single page to disk (used internally for eviction and flush).
    fn write_to_disk(fd: u32, page_num: u32, data: &[u8; PAGE_SIZE]) {
        let offset = page_num as i32 * PAGE_SIZE as i32;
        syscall::lseek(fd, offset, syscall::SEEK_SET);
        syscall::write(fd, data);
    }
}

#[derive(Clone)]
struct TxSnapshot {
    page0: [u8; PAGE_SIZE],
    format: DbFormat,
    tables: Vec<TableSchema>,
    table_count: u32,
    first_free_page: u32,
    first_table_dir_page: u32,
    total_pages: u32,
    pages: Vec<[u8; PAGE_SIZE]>,
}


// ── Database handle ──────────────────────────────────────────────────────────

/// An open database file.
pub struct Database {
    fd: u32,
    in_memory: bool,
    /// Cached copy of page 0 (header + table directory).
    page0: [u8; PAGE_SIZE],
    /// On-disk format.
    format: DbFormat,
    /// Parsed table schemas (in sync with page0).
    pub tables: Vec<TableSchema>,
    /// In-memory equality indexes rebuilt from persisted definitions.
    index_states: Vec<TableIndexState>,
    /// Number of tables.
    pub table_count: u32,
    /// First free page (for page reuse — 0 = none, allocate at end).
    first_free_page: u32,
    /// First linked-list page containing table metadata in v2.
    first_table_dir_page: u32,
    /// Total pages in the file.
    total_pages: u32,
    /// Last error message.
    pub last_error: String,
    /// Page cache — avoids repeated disk reads.
    cache: PageCache,
    /// Optional transaction snapshot.
    tx_snapshot: Option<TxSnapshot>,
}

impl Database {
    /// Open or create a database file.
    pub fn open(path: &str) -> DbResult<Database> {
        if path == ":memory:" {
            return Self::open_in_memory();
        }
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
                in_memory: false,
                page0: [0u8; PAGE_SIZE],
                format: DbFormat::V2,
                tables: Vec::new(),
                index_states: Vec::new(),
                table_count: 0,
                first_free_page: 0,
                first_table_dir_page: 0,
                total_pages: 0,
                last_error: String::new(),
                cache: PageCache::new(fd, false),
                tx_snapshot: None,
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
                in_memory: false,
                page0: [0u8; PAGE_SIZE],
                format: DbFormat::V2,
                tables: Vec::new(),
                index_states: Vec::new(),
                table_count: 0,
                first_free_page: 0,
                first_table_dir_page: 0,
                total_pages: 1,
                last_error: String::new(),
                cache: PageCache::new(fd, false),
                tx_snapshot: None,
            };
            schema::init_header(&mut db.page0);
            // Write page 0 directly to disk so the file has content
            db.write_page_raw(0, &db.page0)?;
            // Also put in cache for subsequent reads
            db.cache.put(0, &db.page0, false);
            Ok(db)
        }
    }

    pub fn open_in_memory() -> DbResult<Database> {
        let fd = u32::MAX;
        let mut db = Database {
            fd,
            in_memory: true,
            page0: [0u8; PAGE_SIZE],
            format: DbFormat::V2,
            tables: Vec::new(),
            index_states: Vec::new(),
            table_count: 0,
            first_free_page: 0,
            first_table_dir_page: 0,
            total_pages: 1,
            last_error: String::new(),
            cache: PageCache::new(fd, true),
            tx_snapshot: None,
        };
        schema::init_header(&mut db.page0);
        db.cache.put(0, &db.page0, false);
        Ok(db)
    }

    /// Close the database (flush dirty pages and release fd).
    pub fn close(&mut self) {
        if self.fd != u32::MAX {
            let _ = self.flush();
            syscall::close(self.fd);
            self.fd = u32::MAX;
        }
    }

    // ── Page I/O ─────────────────────────────────────────────────────────

    /// Read a page — returns cached copy if available, otherwise reads from disk.
    fn read_page(&mut self, page_num: u32, buf: &mut [u8; PAGE_SIZE]) -> DbResult<()> {
        // Check cache first
        if let Some(cached) = self.cache.get(page_num) {
            *buf = *cached;
            return Ok(());
        }

        // Cache miss — read from disk
        if self.in_memory {
            buf.fill(0);
            self.cache.put(page_num, buf, false);
            return Ok(());
        }
        let offset = page_num as i32 * PAGE_SIZE as i32;
        if syscall::lseek(self.fd, offset, syscall::SEEK_SET) == u32::MAX {
            return Err(DbError::Io(String::from("Seek failed")));
        }
        let n = syscall::read(self.fd, buf);
        if n == u32::MAX {
            return Err(DbError::Io(String::from("Read failed")));
        }
        if (n as usize) < PAGE_SIZE {
            buf[n as usize..].fill(0);
        }

        // Store in cache (eviction writes to disk automatically)
        self.cache.put(page_num, buf, false);
        Ok(())
    }

    pub fn begin_transaction(&mut self) -> DbResult<()> {
        if self.tx_snapshot.is_some() {
            return Err(DbError::Parse(String::from("Transaction already active")));
        }
        let mut pages = Vec::with_capacity(self.total_pages as usize);
        for page_num in 0..self.total_pages {
            let mut page = [0u8; PAGE_SIZE];
            self.read_page(page_num, &mut page)?;
            pages.push(page);
        }
        self.tx_snapshot = Some(TxSnapshot {
            page0: self.page0,
            format: self.format,
            tables: self.tables.clone(),
            table_count: self.table_count,
            first_free_page: self.first_free_page,
            first_table_dir_page: self.first_table_dir_page,
            total_pages: self.total_pages,
            pages,
        });
        Ok(())
    }

    pub fn commit_transaction(&mut self) -> DbResult<()> {
        if self.tx_snapshot.is_none() {
            return Err(DbError::Parse(String::from("No active transaction")));
        }
        self.flush()?;
        self.tx_snapshot = None;
        Ok(())
    }

    pub fn rollback_transaction(&mut self) -> DbResult<()> {
        let snapshot = self.tx_snapshot.take()
            .ok_or_else(|| DbError::Parse(String::from("No active transaction")))?;
        self.page0 = snapshot.page0;
        self.format = snapshot.format;
        self.tables = snapshot.tables;
        self.table_count = snapshot.table_count;
        self.first_free_page = snapshot.first_free_page;
        self.first_table_dir_page = snapshot.first_table_dir_page;
        self.total_pages = snapshot.total_pages;
        self.index_states.clear();
        self.cache.clear();
        for (page_num, page) in snapshot.pages.iter().enumerate() {
            self.cache.put(page_num as u32, page, true);
            if !self.in_memory {
                self.write_page_raw(page_num as u32, page)?;
            }
        }
        self.flush()?;
        self.rebuild_all_indexes()?;
        Ok(())
    }

    /// Write a page — updates cache (dirty) and defers disk write until flush.
    fn write_page(&mut self, page_num: u32, buf: &[u8; PAGE_SIZE]) -> DbResult<()> {
        self.cache.put(page_num, buf, true);
        Ok(())
    }

    /// Write a page directly to disk (bypasses cache).
    fn write_page_raw(&self, page_num: u32, buf: &[u8; PAGE_SIZE]) -> DbResult<()> {
        if self.in_memory {
            return Ok(());
        }
        let offset = page_num as i32 * PAGE_SIZE as i32;
        if syscall::lseek(self.fd, offset, syscall::SEEK_SET) == u32::MAX {
            return Err(DbError::Io(String::from("Seek failed")));
        }
        let n = syscall::write(self.fd, buf);
        if n == u32::MAX || n as usize != PAGE_SIZE {
            return Err(DbError::Io(String::from("Write failed")));
        }
        if syscall::fsync(self.fd) == u32::MAX {
            return Err(DbError::Io(String::from("fsync failed after raw write")));
        }
        Ok(())
    }

    /// Flush all dirty cached pages to disk.
    pub fn flush(&mut self) -> DbResult<()> {
        self.cache.flush();
        if self.in_memory {
            return Ok(());
        }
        if syscall::fsync(self.fd) == u32::MAX {
            return Err(DbError::Io(String::from("fsync failed")));
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
        let header = schema::read_header(&self.page0)?;
        self.format = header.format;
        self.table_count = header.table_count;
        self.first_free_page = header.first_free_page;
        self.first_table_dir_page = header.first_table_dir_page;
        let file_size = syscall::file_size(self.fd);
        self.total_pages = if file_size > 0 {
            (file_size as usize / PAGE_SIZE).max(1) as u32
        } else {
            1
        };

        self.tables = match self.format {
            DbFormat::V1 => schema::read_tables_v1(&self.page0, self.table_count)?,
            DbFormat::V2 => self.load_table_directory(self.first_table_dir_page)?,
        };
        self.rebuild_all_indexes()?;
        Ok(())
    }

    /// Flush page 0 to disk (after schema changes).
    fn flush_page0(&mut self) -> DbResult<()> {
        self.format = DbFormat::V2;
        self.write_table_directory()?;
        schema::write_header_fields(
            &mut self.page0,
            self.format,
            self.table_count,
            self.first_free_page,
            self.first_table_dir_page,
        );
        self.cache.put(0, &self.page0, true);
        Ok(())
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
            // Allocate at end of file — must write to disk to extend the file
            let page_num = self.total_pages;
            self.total_pages += 1;
            let page = [0u8; PAGE_SIZE];
            self.write_page_raw(page_num, &page)?;
            self.cache.put(page_num, &page, false);
            Ok(page_num)
        }
    }

    /// Free a data page (add to free list).
    fn free_page(&mut self, page_num: u32) -> DbResult<()> {
        self.cache.invalidate(page_num);
        let mut page = [0u8; PAGE_SIZE];
        // Write next-free pointer as first 4 bytes
        page[0..4].copy_from_slice(&self.first_free_page.to_le_bytes());
        self.write_page(page_num, &page)?;
        self.first_free_page = page_num;
        Ok(())
    }

    fn free_page_chain(&mut self, mut page_num: u32) -> DbResult<()> {
        while page_num != 0 {
            let mut page = [0u8; PAGE_SIZE];
            self.read_page(page_num, &mut page)?;
            let next = u32::from_le_bytes([page[0], page[1], page[2], page[3]]);
            self.free_page(page_num)?;
            page_num = next;
        }
        Ok(())
    }

    fn free_table_data_chain(&mut self, mut page_num: u32) -> DbResult<()> {
        while page_num != 0 {
            let mut page = [0u8; PAGE_SIZE];
            self.read_page(page_num, &mut page)?;
            let next = u32::from_le_bytes([page[0], page[1], page[2], page[3]]);
            let data_end = u16::from_le_bytes([page[6], page[7]]) as usize;
            let data_end = if data_end == 0 { DATA_PAGE_HEADER } else { data_end };

            let mut offset = DATA_PAGE_HEADER;
            while offset < data_end {
                if page[offset] == ROW_OVERFLOW {
                    if let Some((first_page, _total_len, consumed)) =
                        Self::overflow_stub_info(&page, offset)
                    {
                        if first_page != 0 {
                            self.free_page_chain(first_page)?;
                        }
                        offset += consumed;
                        continue;
                    }
                }

                match Self::deserialize_row(&page, offset, 0) {
                    Some((_row, consumed)) if consumed > 0 => offset += consumed,
                    _ => break,
                }
            }

            self.free_page(page_num)?;
            page_num = next;
        }
        Ok(())
    }

    fn load_table_directory(&mut self, mut page_num: u32) -> DbResult<Vec<TableSchema>> {
        let mut tables = Vec::new();
        while page_num != 0 {
            let mut page = [0u8; PAGE_SIZE];
            self.read_page(page_num, &mut page)?;
            let next_page = u32::from_le_bytes([page[0], page[1], page[2], page[3]]);
            let row_count = u32::from_le_bytes([page[4], page[5], page[6], page[7]]);
            let first_data_page = u32::from_le_bytes([page[8], page[9], page[10], page[11]]);
            let schema_page = u32::from_le_bytes([page[12], page[13], page[14], page[15]]);
            let name_len = u16::from_le_bytes([page[16], page[17]]) as usize;
            if name_len == 0 || 18 + name_len > PAGE_SIZE {
                return Err(DbError::Corrupt(String::from("Invalid table directory entry")));
            }
            let name = core::str::from_utf8(&page[18..18 + name_len])
                .map_err(|_| DbError::Corrupt(String::from("Invalid table name encoding")))?;
            let (columns, indexes) = self.read_schema_definition(schema_page)?;
            tables.push(TableSchema {
                name: String::from(name),
                columns,
                indexes,
                row_count,
                first_data_page,
                schema_page,
                dir_page: page_num,
            });
            page_num = next_page;
        }
        Ok(tables)
    }

    fn read_schema_definition(&mut self, mut page_num: u32) -> DbResult<(Vec<ColumnDef>, Vec<IndexDef>)> {
        let mut columns = Vec::new();
        let mut indexes = Vec::new();
        while page_num != 0 {
            let mut page = [0u8; PAGE_SIZE];
            self.read_page(page_num, &mut page)?;
            let next_page = u32::from_le_bytes([page[0], page[1], page[2], page[3]]);
            let used_end = u16::from_le_bytes([page[4], page[5]]) as usize;
            let end = if used_end == 0 { 8 } else { used_end.min(PAGE_SIZE) };
            let structured = page[6] == 1;
            let mut pos = 8usize;
            while pos < end {
                if structured {
                    if pos + 1 > end {
                        break;
                    }
                    let tag = page[pos];
                    pos += 1;
                    match tag {
                        1 => {
                            if pos + 2 > end {
                                return Err(DbError::Corrupt(String::from("Schema column truncated")));
                            }
                            let name_len = page[pos] as usize;
                            let col_type = match page[pos + 1] as u16 {
                                1 => ColumnType::Integer,
                                2 => ColumnType::Text,
                                3 => ColumnType::Blob,
                                _ => return Err(DbError::Corrupt(String::from("Invalid schema column type"))),
                            };
                            pos += 2;
                            if pos + name_len > end {
                                return Err(DbError::Corrupt(String::from("Schema column name truncated")));
                            }
                            let name = core::str::from_utf8(&page[pos..pos + name_len])
                                .map_err(|_| DbError::Corrupt(String::from("Invalid schema column name")))?;
                            columns.push(ColumnDef {
                                name: String::from(name),
                                col_type,
                            });
                            pos += name_len;
                        }
                        2 => {
                            if pos + 3 > end {
                                return Err(DbError::Corrupt(String::from("Schema index truncated")));
                            }
                            let name_len = page[pos] as usize;
                            let col_len = page[pos + 1] as usize;
                            let flags = page[pos + 2];
                            pos += 3;
                            if pos + name_len + col_len > end {
                                return Err(DbError::Corrupt(String::from("Schema index name truncated")));
                            }
                            let name = core::str::from_utf8(&page[pos..pos + name_len])
                                .map_err(|_| DbError::Corrupt(String::from("Invalid schema index name")))?;
                            pos += name_len;
                            let column = core::str::from_utf8(&page[pos..pos + col_len])
                                .map_err(|_| DbError::Corrupt(String::from("Invalid schema index column")))?;
                            pos += col_len;
                            indexes.push(IndexDef {
                                name: String::from(name),
                                column: String::from(column),
                                unique: (flags & 0x01) != 0,
                            });
                        }
                        0 => break,
                        _ => return Err(DbError::Corrupt(String::from("Unknown schema entry tag"))),
                    }
                } else {
                    if pos + 2 > end {
                        break;
                    }
                    let name_len = page[pos] as usize;
                    let col_type = match page[pos + 1] as u16 {
                        1 => ColumnType::Integer,
                        2 => ColumnType::Text,
                        3 => ColumnType::Blob,
                        _ => return Err(DbError::Corrupt(String::from("Invalid schema column type"))),
                    };
                    pos += 2;
                    if pos + name_len > end {
                        return Err(DbError::Corrupt(String::from("Schema page truncated")));
                    }
                    let name = core::str::from_utf8(&page[pos..pos + name_len])
                        .map_err(|_| DbError::Corrupt(String::from("Invalid schema column name")))?;
                    columns.push(ColumnDef {
                        name: String::from(name),
                        col_type,
                    });
                    pos += name_len;
                }
            }
            page_num = next_page;
        }
        Ok((columns, indexes))
    }

    fn write_schema_definition(
        &mut self,
        columns: &[ColumnDef],
        indexes: &[IndexDef],
        old_first_page: u32,
    ) -> DbResult<u32> {
        if old_first_page != 0 {
            self.free_page_chain(old_first_page)?;
        }

        let mut serialized = Vec::new();
        for column in columns {
            if column.name.len() > MAX_COL_NAME {
                return Err(DbError::Parse(String::from("Column name too long")));
            }
            serialized.push(1);
            serialized.push(column.name.len() as u8);
            serialized.push(column.col_type as u8);
            serialized.extend_from_slice(column.name.as_bytes());
        }
        for index in indexes {
            if index.name.len() > MAX_COL_NAME {
                return Err(DbError::Parse(String::from("Index name too long")));
            }
            if index.column.len() > MAX_COL_NAME {
                return Err(DbError::Parse(String::from("Indexed column name too long")));
            }
            serialized.push(2);
            serialized.push(index.name.len() as u8);
            serialized.push(index.column.len() as u8);
            serialized.push(if index.unique { 1 } else { 0 });
            serialized.extend_from_slice(index.name.as_bytes());
            serialized.extend_from_slice(index.column.as_bytes());
        }

        let mut offset = 0usize;
        let mut first_page = 0u32;
        let mut prev_page = 0u32;
        while offset < serialized.len() || first_page == 0 {
            let page_num = self.alloc_page()?;
            if first_page == 0 {
                first_page = page_num;
            }
            let mut page = [0u8; PAGE_SIZE];
            let space = PAGE_SIZE - 8;
            let remaining = serialized.len().saturating_sub(offset);
            let chunk = remaining.min(space);
            if chunk > 0 {
                page[8..8 + chunk].copy_from_slice(&serialized[offset..offset + chunk]);
                offset += chunk;
            }
            page[6] = 1;
            let used_end = 8 + chunk;
            page[4..6].copy_from_slice(&(used_end as u16).to_le_bytes());
            self.write_page(page_num, &page)?;

            if prev_page != 0 {
                let mut prev = [0u8; PAGE_SIZE];
                self.read_page(prev_page, &mut prev)?;
                prev[0..4].copy_from_slice(&page_num.to_le_bytes());
                self.write_page(prev_page, &prev)?;
            }
            prev_page = page_num;
        }
        Ok(first_page)
    }

    fn write_table_directory(&mut self) -> DbResult<()> {
        for i in 0..self.tables.len() {
            if self.tables[i].dir_page == 0 {
                self.tables[i].dir_page = self.alloc_page()?;
            }
            let old_schema_page = self.tables[i].schema_page;
            let columns = self.tables[i].columns.clone();
            let indexes = self.tables[i].indexes.clone();
            self.tables[i].schema_page =
                self.write_schema_definition(&columns, &indexes, old_schema_page)?;
        }

        self.first_table_dir_page = self.tables.first().map(|t| t.dir_page).unwrap_or(0);

        for i in 0..self.tables.len() {
            let next = if i + 1 < self.tables.len() {
                self.tables[i + 1].dir_page
            } else {
                0
            };
            let table = &self.tables[i];
            let name_bytes = table.name.as_bytes();
            if name_bytes.len() > MAX_TABLE_NAME {
                return Err(DbError::Parse(String::from("Table name too long")));
            }
            let mut page = [0u8; PAGE_SIZE];
            page[0..4].copy_from_slice(&next.to_le_bytes());
            page[4..8].copy_from_slice(&table.row_count.to_le_bytes());
            page[8..12].copy_from_slice(&table.first_data_page.to_le_bytes());
            page[12..16].copy_from_slice(&table.schema_page.to_le_bytes());
            page[16..18].copy_from_slice(&(name_bytes.len() as u16).to_le_bytes());
            page[18..18 + name_bytes.len()].copy_from_slice(name_bytes);
            self.write_page(table.dir_page, &page)?;
        }

        Ok(())
    }

    // ── Table management ─────────────────────────────────────────────────

    /// Create a new table with the given schema.
    pub fn create_table(&mut self, name: &str, columns: &[ColumnDef]) -> DbResult<()> {
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
            indexes: Vec::new(),
            row_count: 0,
            first_data_page: 0,
            schema_page: 0,
            dir_page: 0,
        };
        self.tables.push(table);
        self.index_states.push(TableIndexState { maps: Vec::new() });
        self.table_count += 1;
        self.flush_page0()
    }

    /// Drop a table by name, freeing all its data pages.
    pub fn drop_table(&mut self, name: &str) -> DbResult<()> {
        let idx = schema::find_table(&self.tables, name)
            .ok_or_else(|| DbError::TableNotFound(String::from(name)))?;

        self.free_table_data_chain(self.tables[idx].first_data_page)?;
        self.free_page_chain(self.tables[idx].schema_page)?;
        if self.tables[idx].dir_page != 0 {
            self.free_page(self.tables[idx].dir_page)?;
        }

        // Remove from schema list and compact
        self.tables.remove(idx);
        if idx < self.index_states.len() {
            self.index_states.remove(idx);
        }
        self.table_count -= 1;
        self.flush_page0()
    }

    pub fn add_column(&mut self, table_name: &str, column: ColumnDef) -> DbResult<()> {
        let table_idx = schema::find_table(&self.tables, table_name)
            .ok_or_else(|| DbError::TableNotFound(String::from(table_name)))?;
        if column.name.len() > MAX_COL_NAME {
            return Err(DbError::Parse(String::from("Column name too long")));
        }
        if self.tables[table_idx].find_column(&column.name).is_some() {
            return Err(DbError::TableExists(String::from("Column already exists")));
        }
        if self.tables[table_idx].columns.len() >= MAX_COLUMNS {
            return Err(DbError::TooManyColumns);
        }
        self.tables[table_idx].columns.push(column);
        self.flush_page0()
    }

    pub fn rename_table(&mut self, old_name: &str, new_name: &str) -> DbResult<()> {
        let table_idx = schema::find_table(&self.tables, old_name)
            .ok_or_else(|| DbError::TableNotFound(String::from(old_name)))?;
        if new_name.len() > MAX_TABLE_NAME {
            return Err(DbError::Parse(String::from("Table name too long")));
        }
        if self.tables.iter().enumerate().any(|(idx, table)| {
            idx != table_idx && table.name.eq_ignore_ascii_case(new_name)
        }) {
            return Err(DbError::TableExists(String::from(new_name)));
        }
        self.tables[table_idx].name = String::from(new_name);
        self.flush_page0()
    }

    pub fn rename_column(&mut self, table_name: &str, old_name: &str, new_name: &str) -> DbResult<()> {
        let table_idx = schema::find_table(&self.tables, table_name)
            .ok_or_else(|| DbError::TableNotFound(String::from(table_name)))?;
        if new_name.len() > MAX_COL_NAME {
            return Err(DbError::Parse(String::from("Column name too long")));
        }
        let column_idx = self.tables[table_idx]
            .find_column(old_name)
            .ok_or_else(|| DbError::ColumnNotFound(String::from(old_name)))?;
        if self.tables[table_idx].columns.iter().enumerate().any(|(idx, col)| {
            idx != column_idx && col.name.eq_ignore_ascii_case(new_name)
        }) {
            return Err(DbError::TableExists(String::from("Column already exists")));
        }

        self.tables[table_idx].columns[column_idx].name = String::from(new_name);
        for index in &mut self.tables[table_idx].indexes {
            if index.column.eq_ignore_ascii_case(old_name) {
                index.column = String::from(new_name);
            }
        }
        self.flush_page0()
    }

    pub fn drop_column(&mut self, table_name: &str, column_name: &str) -> DbResult<()> {
        let table_idx = schema::find_table(&self.tables, table_name)
            .ok_or_else(|| DbError::TableNotFound(String::from(table_name)))?;
        let column_idx = self.tables[table_idx]
            .find_column(column_name)
            .ok_or_else(|| DbError::ColumnNotFound(String::from(column_name)))?;
        if self.tables[table_idx].columns.len() <= 1 {
            return Err(DbError::Parse(String::from("Cannot drop the last column")));
        }

        let rows = self.scan_table(table_idx)?;
        let mut rewritten_rows = Vec::with_capacity(rows.len());
        for (_page_num, _offset, row) in rows {
            let mut values = row.values;
            if column_idx < values.len() {
                values.remove(column_idx);
            }
            rewritten_rows.push(values);
        }

        let old_first_data_page = self.tables[table_idx].first_data_page;
        if old_first_data_page != 0 {
            self.free_table_data_chain(old_first_data_page)?;
        }

        self.tables[table_idx].columns.remove(column_idx);
        self.tables[table_idx]
            .indexes
            .retain(|index| !index.column.eq_ignore_ascii_case(column_name));
        self.tables[table_idx].first_data_page = 0;
        self.tables[table_idx].row_count = 0;

        if table_idx >= self.index_states.len() {
            self.index_states
                .resize_with(self.tables.len(), || TableIndexState { maps: Vec::new() });
        }
        self.index_states[table_idx] = TableIndexState {
            maps: Vec::with_capacity(self.tables[table_idx].indexes.len()),
        };
        for _ in 0..self.tables[table_idx].indexes.len() {
            self.index_states[table_idx].maps.push(Default::default());
        }

        self.flush_page0()?;
        for values in &rewritten_rows {
            self.insert_row(table_idx, values)?;
        }
        Ok(())
    }

    pub fn create_index(
        &mut self,
        table_name: &str,
        index_name: &str,
        column_name: &str,
        unique: bool,
    ) -> DbResult<()> {
        let table_idx = schema::find_table(&self.tables, table_name)
            .ok_or_else(|| DbError::TableNotFound(String::from(table_name)))?;
        if index_name.len() > MAX_COL_NAME {
            return Err(DbError::Parse(String::from("Index name too long")));
        }
        if self.tables[table_idx]
            .indexes
            .iter()
            .any(|index| index.name.eq_ignore_ascii_case(index_name))
        {
            return Err(DbError::IndexExists(String::from(index_name)));
        }
        if self.tables[table_idx].find_column(column_name).is_none() {
            return Err(DbError::ColumnNotFound(String::from(column_name)));
        }
        self.tables[table_idx].indexes.push(IndexDef {
            name: String::from(index_name),
            column: String::from(column_name),
            unique,
        });
        if let Err(err) = self.rebuild_indexes_for_table(table_idx) {
            self.tables[table_idx].indexes.pop();
            let _ = self.rebuild_indexes_for_table(table_idx);
            return Err(err);
        }
        self.flush_page0()
    }

    fn rebuild_all_indexes(&mut self) -> DbResult<()> {
        self.index_states.clear();
        self.index_states
            .resize_with(self.tables.len(), || TableIndexState { maps: Vec::new() });
        for table_idx in 0..self.tables.len() {
            self.rebuild_indexes_for_table(table_idx)?;
        }
        Ok(())
    }

    fn rebuild_indexes_for_table(&mut self, table_idx: usize) -> DbResult<()> {
        let index_defs = self.tables[table_idx].indexes.clone();
        let rows = self.scan_table_pages(table_idx)?;
        let mut maps: Vec<crate::index::IndexMap> = Vec::with_capacity(index_defs.len());
        for _ in 0..index_defs.len() {
            maps.push(Default::default());
        }

        for (page_num, offset, row) in rows {
            let location = RowLocation { page_num, offset };
            for (index_idx, index) in index_defs.iter().enumerate() {
                let Some(column_idx) = self.tables[table_idx].find_column(&index.column) else {
                    return Err(DbError::ColumnNotFound(index.column.clone()));
                };
                let key = encode_index_key(row.values.get(column_idx).unwrap_or(&Value::Null));
                let bucket = maps[index_idx].entry(key).or_insert_with(Vec::new);
                if index.unique && !bucket.is_empty() {
                    return Err(DbError::TypeMismatch(String::from("Unique index violation")));
                }
                bucket.push(location);
            }
        }

        self.index_states[table_idx] = TableIndexState { maps };
        Ok(())
    }

    fn add_row_to_indexes(
        &mut self,
        table_idx: usize,
        location: RowLocation,
        values: &[Value],
    ) -> DbResult<()> {
        if table_idx >= self.index_states.len() {
            return Ok(());
        }
        for (index_idx, index) in self.tables[table_idx].indexes.iter().enumerate() {
            let Some(column_idx) = self.tables[table_idx].find_column(&index.column) else {
                return Err(DbError::ColumnNotFound(index.column.clone()));
            };
            let key = encode_index_key(values.get(column_idx).unwrap_or(&Value::Null));
            let bucket = self.index_states[table_idx].maps[index_idx]
                .entry(key)
                .or_insert_with(Vec::new);
            if index.unique && !bucket.is_empty() {
                return Err(DbError::TypeMismatch(String::from("Unique index violation")));
            }
            bucket.push(location);
        }
        Ok(())
    }

    fn check_unique_constraints(&self, table_idx: usize, values: &[Value]) -> DbResult<()> {
        if table_idx >= self.index_states.len() {
            return Ok(());
        }
        for (index_idx, index) in self.tables[table_idx].indexes.iter().enumerate() {
            if !index.unique {
                continue;
            }
            let Some(column_idx) = self.tables[table_idx].find_column(&index.column) else {
                return Err(DbError::ColumnNotFound(index.column.clone()));
            };
            let key = encode_index_key(values.get(column_idx).unwrap_or(&Value::Null));
            if self.index_states[table_idx].maps[index_idx]
                .get(&key)
                .map(|bucket| !bucket.is_empty())
                .unwrap_or(false)
            {
                return Err(DbError::TypeMismatch(String::from("Unique index violation")));
            }
        }
        Ok(())
    }

    fn remove_row_from_indexes(
        &mut self,
        table_idx: usize,
        location: RowLocation,
        values: &[Value],
    ) -> DbResult<()> {
        if table_idx >= self.index_states.len() {
            return Ok(());
        }
        for (index_idx, index) in self.tables[table_idx].indexes.iter().enumerate() {
            let Some(column_idx) = self.tables[table_idx].find_column(&index.column) else {
                return Err(DbError::ColumnNotFound(index.column.clone()));
            };
            let key = encode_index_key(values.get(column_idx).unwrap_or(&Value::Null));
            let mut remove_bucket = false;
            if let Some(bucket) = self.index_states[table_idx].maps[index_idx].get_mut(&key) {
                if let Some(pos) = bucket.iter().position(|entry| *entry == location) {
                    bucket.remove(pos);
                }
                if bucket.is_empty() {
                    remove_bucket = true;
                }
            }
            if remove_bucket {
                self.index_states[table_idx].maps[index_idx].remove(&key);
            }
        }
        Ok(())
    }

    pub fn find_rows_by_index(
        &mut self,
        table_idx: usize,
        column_idx: usize,
        value: &Value,
    ) -> DbResult<Option<Vec<(u32, usize, Row)>>> {
        if table_idx >= self.index_states.len() {
            return Ok(None);
        }
        let Some(index_idx) = self.tables[table_idx]
            .indexes
            .iter()
            .position(|index| self.tables[table_idx]
                .find_column(&index.column)
                .map(|idx| idx == column_idx)
                .unwrap_or(false)) else {
            return Ok(None);
        };

        let key = encode_index_key(value);
        let Some(locations) = self.index_states[table_idx].maps[index_idx].get(&key).cloned() else {
            return Ok(Some(Vec::new()));
        };

        let col_count = self.tables[table_idx].columns.len();
        let mut rows = Vec::with_capacity(locations.len());
        for location in locations {
            let mut page = [0u8; PAGE_SIZE];
            self.read_page(location.page_num, &mut page)?;
            if let Some((row, _)) = self.deserialize_row_at(&page, location.offset, col_count)? {
                if !row.values.is_empty() {
                    rows.push((location.page_num, location.offset, row));
                }
            }
        }
        Ok(Some(rows))
    }

    // ── Row serialization ────────────────────────────────────────────────

    /// Serialize a row's values into bytes. Returns serialized data.
    pub fn serialize_row(values: &[Value]) -> DbResult<Vec<u8>> {
        let total_size: usize = values.iter().map(|v| v.serialized_size()).sum();
        // Row format: flag(1) + row_len(2) + value data
        let row_size = 1 + 2 + total_size;
        if total_size > u16::MAX as usize {
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
                    if s.len() > u16::MAX as usize {
                        return Err(DbError::ValueTooLarge);
                    }
                    buf.push(TAG_TEXT);
                    buf.push((s.len() & 0xFF) as u8);
                    buf.push(((s.len() >> 8) & 0xFF) as u8);
                    buf.extend_from_slice(s.as_bytes());
                }
                Value::Blob(b) => {
                    if b.len() > u16::MAX as usize {
                        return Err(DbError::ValueTooLarge);
                    }
                    buf.push(TAG_BLOB);
                    buf.push((b.len() & 0xFF) as u8);
                    buf.push(((b.len() >> 8) & 0xFF) as u8);
                    buf.extend_from_slice(b);
                }
            }
        }
        Ok(buf)
    }

    fn serialize_overflow_stub(first_page: u32, total_len: usize) -> DbResult<Vec<u8>> {
        if total_len > u32::MAX as usize {
            return Err(DbError::RowTooLarge);
        }
        let mut buf = Vec::with_capacity(11);
        buf.push(ROW_OVERFLOW);
        buf.extend_from_slice(&8u16.to_le_bytes());
        buf.extend_from_slice(&first_page.to_le_bytes());
        buf.extend_from_slice(&(total_len as u32).to_le_bytes());
        Ok(buf)
    }

    fn overflow_stub_info(data: &[u8], offset: usize) -> Option<(u32, usize, usize)> {
        if offset + 11 > data.len() || data[offset] != ROW_OVERFLOW {
            return None;
        }
        let row_len = u16::from_le_bytes([data[offset + 1], data[offset + 2]]) as usize;
        if row_len != 8 || offset + 3 + row_len > data.len() {
            return None;
        }
        let first_page = u32::from_le_bytes([
            data[offset + 3],
            data[offset + 4],
            data[offset + 5],
            data[offset + 6],
        ]);
        let total_len = u32::from_le_bytes([
            data[offset + 7],
            data[offset + 8],
            data[offset + 9],
            data[offset + 10],
        ]) as usize;
        Some((first_page, total_len, 3 + row_len))
    }

    fn write_overflow_payload(&mut self, data: &[u8]) -> DbResult<u32> {
        let mut first_page = 0;
        let mut prev_page = 0;
        let mut pos = 0;

        while pos < data.len() {
            let page_num = self.alloc_page()?;
            if first_page == 0 {
                first_page = page_num;
            }
            if prev_page != 0 {
                let mut prev = [0u8; PAGE_SIZE];
                self.read_page(prev_page, &mut prev)?;
                prev[0..4].copy_from_slice(&page_num.to_le_bytes());
                self.write_page(prev_page, &prev)?;
            }

            let remaining = data.len() - pos;
            let used = core::cmp::min(remaining, OVERFLOW_DATA_SIZE);
            let mut page = [0u8; PAGE_SIZE];
            page[4..8].copy_from_slice(&(used as u32).to_le_bytes());
            page[OVERFLOW_PAGE_HEADER..OVERFLOW_PAGE_HEADER + used]
                .copy_from_slice(&data[pos..pos + used]);
            self.write_page(page_num, &page)?;

            pos += used;
            prev_page = page_num;
        }

        Ok(first_page)
    }

    fn read_overflow_payload(&mut self, mut page_num: u32, total_len: usize) -> DbResult<Vec<u8>> {
        let mut out = Vec::with_capacity(total_len);
        let mut remaining = total_len;

        while remaining > 0 {
            if page_num == 0 {
                return Err(DbError::Corrupt(String::from("Truncated overflow row")));
            }
            let mut page = [0u8; PAGE_SIZE];
            self.read_page(page_num, &mut page)?;
            let next = u32::from_le_bytes([page[0], page[1], page[2], page[3]]);
            let used =
                u32::from_le_bytes([page[4], page[5], page[6], page[7]]) as usize;
            if used > OVERFLOW_DATA_SIZE {
                return Err(DbError::Corrupt(String::from("Invalid overflow page")));
            }
            let take = core::cmp::min(used, remaining);
            out.extend_from_slice(
                &page[OVERFLOW_PAGE_HEADER..OVERFLOW_PAGE_HEADER + take],
            );
            remaining -= take;
            page_num = next;
        }

        Ok(out)
    }

    fn deserialize_row_at(
        &mut self,
        data: &[u8],
        offset: usize,
        col_count: usize,
    ) -> DbResult<Option<(Row, usize)>> {
        if data.get(offset).copied() != Some(ROW_OVERFLOW) {
            return Ok(Self::deserialize_row(data, offset, col_count));
        }
        let Some((first_page, total_len, consumed)) = Self::overflow_stub_info(data, offset) else {
            return Err(DbError::Corrupt(String::from("Invalid overflow row stub")));
        };
        let payload = self.read_overflow_payload(first_page, total_len)?;
        Ok(Self::deserialize_row(&payload, 0, col_count)
            .map(|(row, _payload_consumed)| (row, consumed)))
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
                TAG_BLOB => {
                    if pos + 3 > end { break; }
                    let blen = u16::from_le_bytes([data[pos + 1], data[pos + 2]]) as usize;
                    if pos + 3 + blen > end { break; }
                    let mut blob = Vec::with_capacity(blen);
                    blob.extend_from_slice(&data[pos + 3..pos + 3 + blen]);
                    values.push(Value::Blob(blob));
                    pos += 3 + blen;
                }
                _ => break,
            }
        }

        Some((Row { values }, total))
    }

    // ── Table scan ───────────────────────────────────────────────────────

    /// Scan all active rows of a table. Returns a Vec of (page_num, offset_in_page, Row).
    pub fn scan_table(&mut self, table_idx: usize) -> DbResult<Vec<(u32, usize, Row)>> {
        if let Some(rows) = self.scan_table_from_index(table_idx)? {
            return Ok(rows);
        }
        self.scan_table_pages(table_idx)
    }

    fn scan_table_from_index(
        &mut self,
        table_idx: usize,
    ) -> DbResult<Option<Vec<(u32, usize, Row)>>> {
        let expected_rows = self.tables[table_idx].row_count as usize;
        if expected_rows == 0 {
            return Ok(Some(Vec::new()));
        }

        let Some(index_state) = self.index_states.get(table_idx) else {
            return Ok(None);
        };
        let Some(index_map) = index_state.maps.first() else {
            return Ok(None);
        };
        if index_map.is_empty() {
            return Ok(None);
        }

        let mut locations = Vec::with_capacity(expected_rows);
        for bucket in index_map.values() {
            for location in bucket {
                locations.push(*location);
            }
        }
        if locations.len() < expected_rows {
            return Ok(None);
        }

        let col_count = self.tables[table_idx].columns.len();
        let mut results = Vec::with_capacity(expected_rows);
        for location in locations {
            let mut page = [0u8; PAGE_SIZE];
            self.read_page(location.page_num, &mut page)?;
            if let Some((row, _consumed)) =
                self.deserialize_row_at(&page, location.offset, col_count)?
            {
                if !row.values.is_empty() {
                    results.push((location.page_num, location.offset, row));
                    if results.len() >= expected_rows {
                        return Ok(Some(results));
                    }
                }
            }
        }

        Ok(None)
    }

    fn scan_table_pages(&mut self, table_idx: usize) -> DbResult<Vec<(u32, usize, Row)>> {
        let table = &self.tables[table_idx];
        let col_count = table.columns.len();
        let expected_rows = table.row_count as usize;
        let mut results = Vec::with_capacity(expected_rows);
        let mut page_num = table.first_data_page;

        if expected_rows == 0 {
            return Ok(results);
        }

        while page_num != 0 {
            let mut page = [0u8; PAGE_SIZE];
            self.read_page(page_num, &mut page)?;

            let next_page = u32::from_le_bytes([page[0], page[1], page[2], page[3]]);
            let page_rows = u16::from_le_bytes([page[4], page[5]]) as usize;
            let data_end = u16::from_le_bytes([page[6], page[7]]) as usize;
            let data_end = if data_end == 0 { DATA_PAGE_HEADER } else { data_end };

            if page_rows == 0 {
                page_num = next_page;
                continue;
            }

            let mut offset = DATA_PAGE_HEADER;
            let mut found_on_page = 0usize;
            while offset < data_end {
                match self.deserialize_row_at(&page, offset, col_count)? {
                    Some((row, consumed)) => {
                        if !row.values.is_empty() {
                            results.push((page_num, offset, row));
                            found_on_page += 1;
                            if results.len() >= expected_rows {
                                return Ok(results);
                            }
                            if found_on_page >= page_rows {
                                break;
                            }
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
        self.check_unique_constraints(table_idx, values)?;
        let mut row_data = Self::serialize_row(values)?;
        let mut overflow_first_page = 0;
        if row_data.len() > DATA_AREA_SIZE {
            overflow_first_page = self.write_overflow_payload(&row_data)?;
            row_data = Self::serialize_overflow_stub(overflow_first_page, row_data.len())?;
        }
        match self.insert_row_data(table_idx, &row_data, values) {
            Ok(()) => Ok(()),
            Err(err) => {
                if overflow_first_page != 0 {
                    let _ = self.free_page_chain(overflow_first_page);
                }
                Err(err)
            }
        }
    }

    fn insert_row_data(
        &mut self,
        table_idx: usize,
        row_data: &[u8],
        values: &[Value],
    ) -> DbResult<()> {
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
                let row_offset = data_end;
                page[data_end..data_end + row_len].copy_from_slice(row_data);
                let new_end = (data_end + row_len) as u16;
                page[6..8].copy_from_slice(&new_end.to_le_bytes());
                let rc = u16::from_le_bytes([page[4], page[5]]);
                page[4..6].copy_from_slice(&(rc + 1).to_le_bytes());
                self.write_page(page_num, &page)?;

                self.tables[table_idx].row_count += 1;
                self.add_row_to_indexes(
                    table_idx,
                    RowLocation {
                        page_num,
                        offset: row_offset,
                    },
                    values,
                )?;
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
        new_page[DATA_PAGE_HEADER..DATA_PAGE_HEADER + row_len].copy_from_slice(row_data);
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
        self.add_row_to_indexes(
            table_idx,
            RowLocation {
                page_num: new_page_num,
                offset: DATA_PAGE_HEADER,
            },
            values,
        )?;
        self.flush_page0()
    }

    // ── Row deletion ─────────────────────────────────────────────────────

    /// Delete a row at a specific location (page_num, offset).
    pub fn delete_row(&mut self, table_idx: usize, page_num: u32, offset: usize) -> DbResult<()> {
        let mut page = [0u8; PAGE_SIZE];
        self.read_page(page_num, &mut page)?;
        let col_count = self.tables[table_idx].columns.len();
        let overflow_page = if page.get(offset).copied() == Some(ROW_OVERFLOW) {
            Self::overflow_stub_info(&page, offset).map(|(first_page, _total_len, _consumed)| first_page)
        } else {
            None
        };
        let existing_row = self.deserialize_row_at(&page, offset, col_count)?
            .and_then(|(row, _)| if row.values.is_empty() { None } else { Some(row) });

        page[offset] = ROW_DELETED;

        let rc = u16::from_le_bytes([page[4], page[5]]);
        if rc > 0 {
            page[4..6].copy_from_slice(&(rc - 1).to_le_bytes());
        }

        self.write_page(page_num, &page)?;
        if let Some(first_page) = overflow_page {
            if first_page != 0 {
                self.free_page_chain(first_page)?;
            }
        }
        if let Some(row) = existing_row {
            self.remove_row_from_indexes(
                table_idx,
                RowLocation { page_num, offset },
                &row.values,
            )?;
        }

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
