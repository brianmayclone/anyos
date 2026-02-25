# anyOS Database Library (libdb) API Reference

The **libdb** shared library provides a file-based SQL database engine with page-based storage. It supports a subset of SQL (CREATE TABLE, DROP TABLE, INSERT, SELECT, UPDATE, DELETE) with INTEGER and TEXT column types, WHERE clauses with AND/OR logic, and case-insensitive identifiers.

**Format:** ELF64 shared object (.so), loaded via `dl_open("/Libraries/libdb.so")`
**Exports:** 13
**Client crate:** `libdb_client` (uses `dynlink::dl_open` / `dl_sym`)

---

## Table of Contents

- [Getting Started](#getting-started)
- [Client Wrapper Types](#client-wrapper-types)
  - [Database](#database)
  - [QueryResult](#queryresult)
- [C ABI Exports](#c-abi-exports)
  - [libdb_open](#libdb_open)
  - [libdb_close](#libdb_close)
  - [libdb_error](#libdb_error)
  - [libdb_exec](#libdb_exec)
  - [libdb_query](#libdb_query)
  - [libdb_result_row_count](#libdb_result_row_count)
  - [libdb_result_col_count](#libdb_result_col_count)
  - [libdb_result_col_name](#libdb_result_col_name)
  - [libdb_result_get_int](#libdb_result_get_int)
  - [libdb_result_get_int_hi](#libdb_result_get_int_hi)
  - [libdb_result_get_text](#libdb_result_get_text)
  - [libdb_result_is_null](#libdb_result_is_null)
  - [libdb_result_free](#libdb_result_free)
- [SQL Subset](#sql-subset)
  - [CREATE TABLE](#create-table)
  - [DROP TABLE](#drop-table)
  - [INSERT](#insert)
  - [SELECT](#select)
  - [UPDATE](#update)
  - [DELETE](#delete)
  - [WHERE Clauses](#where-clauses)
- [Database File Format](#database-file-format)
  - [Page 0 (Header)](#page-0-header)
  - [Table Directory Entry](#table-directory-entry)
  - [Data Pages](#data-pages)
  - [Row Format](#row-format)
  - [Value Encoding](#value-encoding)
  - [Free Page List](#free-page-list)
- [Constraints](#constraints)
- [Error Types](#error-types)
- [Examples](#examples)

---

## Getting Started

### Dependencies

Add to your program's `Cargo.toml`:

```toml
[dependencies]
anyos_std = { path = "../../libs/stdlib" }
dynlink = { path = "../../libs/dynlink" }
libdb_client = { path = "../../libs/libdb_client" }
```

### Example

```rust
#![no_std]
#![no_main]

use anyos_std::*;
use libdb_client;

anyos_std::entry!(main);

fn main() {
    // Initialize (loads libdb.so, resolves all 13 symbols)
    if !libdb_client::init() {
        println!("Failed to load libdb");
        return;
    }

    // Open or create a database file
    let db = match libdb_client::Database::open("/data/myapp.db") {
        Some(db) => db,
        None => {
            println!("Failed to open database");
            return;
        }
    };

    // Create a table
    db.exec("CREATE TABLE users (name TEXT, age INTEGER)").unwrap();

    // Insert rows
    db.exec("INSERT INTO users (name, age) VALUES ('Alice', 30)").unwrap();
    db.exec("INSERT INTO users (name, age) VALUES ('Bob', 25)").unwrap();

    // Query
    let result = db.query("SELECT * FROM users WHERE age > 20").unwrap();
    println!("Found {} rows, {} columns", result.row_count(), result.col_count());

    for row in 0..result.row_count() {
        let name = result.get_text(row, 0).unwrap_or_default();
        let age = result.get_int(row, 1).unwrap_or(0);
        println!("{}: {}", name, age);
    }

    // Update
    let affected = db.exec("UPDATE users SET age = 31 WHERE name = 'Alice'").unwrap();
    println!("Updated {} rows", affected);

    // Delete
    let affected = db.exec("DELETE FROM users WHERE name = 'Bob'").unwrap();
    println!("Deleted {} rows", affected);

    // Drop table
    db.exec("DROP TABLE users").unwrap();

    // Database is closed automatically when `db` is dropped
}
```

---

## Client Wrapper Types

The `libdb_client` crate provides two safe wrapper types that manage handles and result sets with automatic cleanup via `Drop`.

### Database

An open database handle. Created via `Database::open()`, closed automatically on drop.

```rust
pub struct Database { /* handle: u32 */ }
```

#### `Database::open(path) -> Option<Database>`

Open or create a database file. If the file does not exist, a new empty database is created with an initialized page 0. If the file exists and is at least one page (4096 bytes), it is opened and the table directory is loaded.

| Parameter | Type | Description |
|-----------|------|-------------|
| path | `&str` | Filesystem path to the `.db` file |
| **Returns** | `Option<Database>` | `Some(db)` on success, `None` on failure |

#### `Database::exec(sql) -> Result<u32, String>`

Execute a non-query SQL statement (CREATE TABLE, DROP TABLE, INSERT, UPDATE, DELETE).

| Parameter | Type | Description |
|-----------|------|-------------|
| sql | `&str` | SQL statement to execute |
| **Returns** | `Result<u32, String>` | Number of rows affected, or error message |

**Notes:**
- CREATE TABLE and DROP TABLE return 0 on success
- INSERT returns 1 on success
- UPDATE and DELETE return the number of matching rows affected
- Returns `u32::MAX` mapped to `Err(String)` on any error (parse error, table not found, type mismatch, I/O error)

#### `Database::query(sql) -> Result<QueryResult, String>`

Execute a SELECT query and return a result set for iterating rows.

| Parameter | Type | Description |
|-----------|------|-------------|
| sql | `&str` | SELECT statement to execute |
| **Returns** | `Result<QueryResult, String>` | Result set, or error message |

**Notes:**
- Only accepts SELECT statements; passing other statement types returns an error
- The returned `QueryResult` holds the full result set in memory
- The result set is freed automatically when the `QueryResult` is dropped

#### `Database::last_error() -> String`

Get the last error message for this database handle. Returns `"Unknown error"` if no error details are available.

#### `Database::close(self)`

Close the database explicitly. This consumes the `Database` value. Equivalent to letting the value drop -- the `Drop` impl calls `libdb_close` automatically.

---

### QueryResult

A query result set returned by `Database::query()`. Freed automatically on drop via `libdb_result_free`.

```rust
pub struct QueryResult { /* id: u32 */ }
```

#### `QueryResult::row_count() -> u32`

Number of rows in the result set.

#### `QueryResult::col_count() -> u32`

Number of columns in the result set.

#### `QueryResult::col_name(col) -> String`

Get a column name by zero-based index.

| Parameter | Type | Description |
|-----------|------|-------------|
| col | `u32` | Column index (0-based) |
| **Returns** | `String` | Column name |

#### `QueryResult::col_names() -> Vec<String>`

Get all column names as a vector.

#### `QueryResult::get_int(row, col) -> Option<i64>`

Get an integer value from a cell. Returns `None` if the cell is NULL.

| Parameter | Type | Description |
|-----------|------|-------------|
| row | `u32` | Row index (0-based) |
| col | `u32` | Column index (0-based) |
| **Returns** | `Option<i64>` | Integer value, or `None` if NULL |

**Notes:**
- Internally combines two 32-bit calls (`libdb_result_get_int` + `libdb_result_get_int_hi`) to reconstruct the full 64-bit value
- Returns `None` (not 0) for NULL cells -- check `is_null()` if you need to distinguish NULL from 0

#### `QueryResult::get_text(row, col) -> Option<String>`

Get a text value from a cell. Returns `None` if the cell is NULL.

| Parameter | Type | Description |
|-----------|------|-------------|
| row | `u32` | Row index (0-based) |
| col | `u32` | Column index (0-based) |
| **Returns** | `Option<String>` | Text value, or `None` if NULL |

**Notes:**
- Returns `Some(String::new())` for empty strings (not `None`)
- Text is read into a 256-byte buffer internally, matching the 255-byte maximum text value size

#### `QueryResult::is_null(row, col) -> bool`

Check if a cell is NULL.

| Parameter | Type | Description |
|-----------|------|-------------|
| row | `u32` | Row index (0-based) |
| col | `u32` | Column index (0-based) |
| **Returns** | `bool` | `true` if the cell is NULL |

---

## C ABI Exports

All 13 exported functions use `extern "C"` with `#[no_mangle]`. Strings are passed as `(pointer, length)` pairs -- not null-terminated. Handles and result IDs are 1-based; 0 indicates failure.

### libdb_open

```c
u32 libdb_open(const u8 *path_ptr, u32 path_len)
```

Open or create a database file. Returns a handle (1+) on success, 0 on failure.

### libdb_close

```c
void libdb_close(u32 handle)
```

Close a database handle and release all associated resources.

### libdb_error

```c
u32 libdb_error(u32 handle, u8 *buf_ptr, u32 buf_len)
```

Get the last error message for a handle. Returns the number of bytes written to `buf`, or 0 if no error.

### libdb_exec

```c
u32 libdb_exec(u32 handle, const u8 *sql_ptr, u32 sql_len)
```

Execute a non-query SQL statement. Returns rows affected, or `u32::MAX` (`0xFFFFFFFF`) on error. Use `libdb_error` to retrieve the error message.

### libdb_query

```c
u32 libdb_query(u32 handle, const u8 *sql_ptr, u32 sql_len)
```

Execute a SELECT query. Returns a result ID (1+), or 0 on error. The result set must be freed with `libdb_result_free` when no longer needed.

### libdb_result_row_count

```c
u32 libdb_result_row_count(u32 result_id)
```

Get the number of rows in a result set. Returns 0 if the result ID is invalid.

### libdb_result_col_count

```c
u32 libdb_result_col_count(u32 result_id)
```

Get the number of columns in a result set. Returns 0 if the result ID is invalid.

### libdb_result_col_name

```c
u32 libdb_result_col_name(u32 result_id, u32 col, u8 *buf_ptr, u32 buf_len)
```

Get a column name by index. Returns bytes written to `buf`, or 0 if the column index or result ID is invalid.

### libdb_result_get_int

```c
u32 libdb_result_get_int(u32 result_id, u32 row, u32 col)
```

Get the low 32 bits of an INTEGER cell value. Returns 0 if the cell is not an integer or indices are out of range.

### libdb_result_get_int_hi

```c
u32 libdb_result_get_int_hi(u32 result_id, u32 row, u32 col)
```

Get the high 32 bits of an INTEGER cell value. Combined with `libdb_result_get_int`, reconstructs the full 64-bit `i64`: `(hi << 32) | lo`.

### libdb_result_get_text

```c
u32 libdb_result_get_text(u32 result_id, u32 row, u32 col, u8 *buf_ptr, u32 buf_len)
```

Get a TEXT cell value. Returns bytes written to `buf`, or 0 if the cell is not text or indices are out of range.

### libdb_result_is_null

```c
u32 libdb_result_is_null(u32 result_id, u32 row, u32 col)
```

Check if a cell is NULL. Returns 1 if null, 0 otherwise. Returns 1 if the result ID or indices are invalid (treat missing data as null).

### libdb_result_free

```c
void libdb_result_free(u32 result_id)
```

Free a result set and release its memory. Must be called when the result is no longer needed to avoid leaking one of the 16 result slots.

---

## SQL Subset

The SQL parser is case-insensitive for keywords and identifier lookups. Line comments (`-- ...`) are supported. Statements may optionally end with a semicolon.

### CREATE TABLE

```sql
CREATE TABLE name (col1 TYPE, col2 TYPE, ...)
```

Create a new table. Types are `INTEGER` (aliases: `INT`) and `TEXT` (aliases: `VARCHAR`). Returns 0 rows affected.

**Errors:** Table already exists, too many tables (max 31), too many columns (max 8), table name too long (max 31 chars), column name too long (max 7 chars).

### DROP TABLE

```sql
DROP TABLE name
```

Drop a table and free all its data pages. Returns 0 rows affected.

**Errors:** Table not found.

### INSERT

```sql
INSERT INTO name (col1, col2) VALUES (val1, val2)
INSERT INTO name VALUES (val1, val2)
```

Insert a row. With explicit column names, unmentioned columns default to NULL. Without column names, values must match the schema column count and order exactly. Returns 1 row affected.

**Values:** Integer literals (`42`, `-7`), string literals (`'hello'`), and `NULL`. Single quotes within strings are escaped by doubling: `'it''s'`.

**Errors:** Table not found, column count mismatch, type mismatch (e.g. integer value for a TEXT column), value too large (text > 255 bytes), row too large for page.

### SELECT

```sql
SELECT * FROM name
SELECT col1, col2 FROM name WHERE condition
```

Query rows from a table. Supports `*` (all columns) or a named column list. Returns a result set accessible via `QueryResult`.

**Errors:** Table not found, column not found.

### UPDATE

```sql
UPDATE name SET col1 = val1, col2 = val2 WHERE condition
```

Update matching rows. Without a WHERE clause, all rows are updated. Returns the number of rows affected.

**Implementation note:** Updates are performed as delete + re-insert internally. This is correct but may change row ordering within pages.

**Errors:** Table not found, column not found, type mismatch.

### DELETE

```sql
DELETE FROM name WHERE condition
DELETE FROM name
```

Delete matching rows. Without a WHERE clause, all rows are deleted (but the table remains). Returns the number of rows deleted.

**Errors:** Table not found.

### WHERE Clauses

WHERE clauses support comparison operators and boolean logic:

| Operator | Description |
|----------|-------------|
| `=` | Equal (case-insensitive for TEXT) |
| `!=`, `<>` | Not equal |
| `<` | Less than |
| `>` | Greater than |
| `<=` | Less than or equal |
| `>=` | Greater than or equal |
| `AND` | Logical AND |
| `OR` | Logical OR (lower precedence than AND) |
| `(...)` | Parenthesized subexpressions |

**Operator precedence** (lowest to highest): OR, AND, comparison.

**NULL comparison:** `NULL = NULL` is true (unlike standard SQL). Any other comparison involving NULL returns false except `!=` which returns true.

**Cross-type comparison:** When comparing INTEGER and TEXT, the TEXT value is parsed as an integer if possible. If parsing fails, only `!=` returns true.

**Text equality:** The `=` operator for TEXT values uses case-insensitive ASCII comparison.

---

## Database File Format

The database file uses a page-based layout with 4096-byte pages. Page 0 contains the file header and table directory. Data pages form linked chains per table.

**Magic:** `ANYDB100` (8 bytes)
**Page size:** 4096 bytes (fixed)

### Page 0 (Header)

| Offset | Size | Field |
|--------|------|-------|
| 0 | 8 | Magic: `ANYDB100` |
| 8 | 4 | Page size (u32 LE, always 4096) |
| 12 | 4 | Table count (u32 LE) |
| 16 | 4 | First free page (u32 LE, 0 = none) |
| 20 | 12 | Reserved (zeroed) |
| 32 | 4064 | Table directory: up to 31 entries of 128 bytes each |

### Table Directory Entry

Each table entry is 128 bytes:

| Offset | Size | Field |
|--------|------|-------|
| 0 | 32 | Table name (null-terminated ASCII, max 31 chars) |
| 32 | 2 | Column count (u16 LE) |
| 34 | 2 | Reserved |
| 36 | 4 | Row count (u32 LE) |
| 40 | 4 | First data page number (u32 LE, 0 = no data) |
| 44 | 4 | Reserved |
| 48 | 80 | Column definitions: up to 8 entries of 10 bytes each |

Each column definition (10 bytes):

| Offset | Size | Field |
|--------|------|-------|
| 0 | 8 | Column name (null-terminated ASCII, max 7 chars) |
| 8 | 2 | Column type (u16 LE): 1 = INTEGER, 2 = TEXT |

### Data Pages

Data pages store rows in a linked list per table.

| Offset | Size | Field |
|--------|------|-------|
| 0 | 4 | Next page number (u32 LE, 0 = end of chain) |
| 4 | 2 | Active row count in this page (u16 LE) |
| 6 | 2 | Data end offset (u16 LE, byte offset within page) |
| 8 | 4088 | Row data area |

Rows are packed sequentially starting at offset 8. When a row is deleted, its flag byte is set to `0xFF` but it remains in place (lazy deletion). New rows are appended to the first page with sufficient space, or a new page is allocated and linked at the end of the chain.

### Row Format

Each row is serialized as:

| Offset | Size | Field |
|--------|------|-------|
| 0 | 1 | Flag: `0x00` = active, `0xFF` = deleted |
| 1 | 2 | Value data length (u16 LE, excludes flag and length bytes) |
| 3 | variable | Serialized column values |

### Value Encoding

Each value is encoded with a tag byte followed by data:

| Tag | Value | Encoding |
|-----|-------|----------|
| `0x00` | NULL | Tag only (1 byte) |
| `0x01` | INTEGER | Tag + i64 LE (9 bytes total) |
| `0x02` | TEXT | Tag + length (u16 LE) + UTF-8 bytes (3 + N bytes total) |

### Free Page List

Freed pages (from DROP TABLE or future compaction) form a singly-linked list. Each free page stores the next-free page number in its first 4 bytes (u32 LE). The head of the list is stored in the page 0 header at offset 16. When allocating a new page, free pages are reused first; otherwise a new page is appended at the end of the file.

---

## Constraints

| Resource | Limit | Notes |
|----------|-------|-------|
| Concurrent open databases | 8 | Per-process, across all `Database::open()` calls |
| Concurrent result sets | 16 | Per-process, across all `Database::query()` calls |
| Tables per database | 31 | `(4096 - 32) / 128` entries fit in page 0 |
| Columns per table | 8 | 80 bytes available in table entry (8 x 10 bytes) |
| Table name length | 31 chars | Null-terminated in 32-byte field |
| Column name length | 7 chars | Null-terminated in 8-byte field |
| Text value size | 255 bytes | Length stored as u16 but limited to 255 |
| Column types | 2 | INTEGER (i64) and TEXT only |
| Page size | 4096 bytes | Fixed, not configurable |
| Row size | 4088 bytes max | Must fit in single data page area |

---

## Error Types

The server-side `DbError` enum maps to human-readable error strings returned via `libdb_error`:

| Error | Message Format |
|-------|---------------|
| `Io` | `"I/O error: <details>"` |
| `Parse` | `"Parse error: <details>"` |
| `TableNotFound` | `"Table not found: <name>"` |
| `TableExists` | `"Table already exists: <name>"` |
| `ColumnNotFound` | `"Column not found: <name>"` |
| `TypeMismatch` | `"Type mismatch: <details>"` |
| `TooManyTables` | `"Too many tables (max 31)"` |
| `TooManyColumns` | `"Too many columns (max 8)"` |
| `ValueTooLarge` | `"Value too large (text max 255 bytes)"` |
| `Corrupt` | `"Corrupt database: <details>"` |
| `RowTooLarge` | `"Row too large for page"` |

---

## Examples

### Key-Value Store

```rust
use libdb_client;

fn kv_store_example() {
    libdb_client::init();
    let db = libdb_client::Database::open("/data/settings.db").unwrap();

    // Create key-value table
    db.exec("CREATE TABLE prefs (key TEXT, value TEXT)").unwrap();

    // Set values
    db.exec("INSERT INTO prefs (key, value) VALUES ('theme', 'dark')").unwrap();
    db.exec("INSERT INTO prefs (key, value) VALUES ('font_size', '14')").unwrap();

    // Get a value
    let result = db.query("SELECT value FROM prefs WHERE key = 'theme'").unwrap();
    if result.row_count() > 0 {
        let theme = result.get_text(0, 0).unwrap_or_default();
        // theme = "dark"
    }

    // Update a value
    db.exec("UPDATE prefs SET value = 'light' WHERE key = 'theme'").unwrap();

    // Delete a value
    db.exec("DELETE FROM prefs WHERE key = 'font_size'").unwrap();
}
```

### Iterating All Results

```rust
use libdb_client;

fn print_table(db: &libdb_client::Database, table_name: &str) {
    let sql = "SELECT * FROM ";
    let mut query = anyos_std::String::from(sql);
    query.push_str(table_name);

    let result = db.query(&query).unwrap();

    // Print column headers
    let names = result.col_names();
    for name in &names {
        anyos_std::print!("{}\t", name);
    }
    anyos_std::println!("");

    // Print rows
    for row in 0..result.row_count() {
        for col in 0..result.col_count() {
            if result.is_null(row, col) {
                anyos_std::print!("NULL\t");
            } else if let Some(i) = result.get_int(row, col) {
                anyos_std::print!("{}\t", i);
            } else if let Some(s) = result.get_text(row, col) {
                anyos_std::print!("{}\t", s);
            }
        }
        anyos_std::println!("");
    }
}
```

### Error Handling

```rust
use libdb_client;

fn safe_exec(db: &libdb_client::Database, sql: &str) {
    match db.exec(sql) {
        Ok(affected) => {
            anyos_std::println!("OK, {} rows affected", affected);
        }
        Err(msg) => {
            anyos_std::println!("Error: {}", msg);
        }
    }
}
```

## Architecture

libdb uses two library crates:

- **libdb** (`libs/libdb/`) -- the shared library itself, built as a `staticlib` and linked by `anyld` into an ELF64 `.so`. Exports 13 `#[no_mangle] pub extern "C"` symbols. Contains the SQL parser (recursive-descent tokenizer + parser), schema manager (page 0 directory), storage engine (page I/O, row serialization, table scanning), and query executor.

- **libdb_client** (`libs/libdb_client/`) -- client wrapper that resolves symbols via `dynlink::dl_open("/Libraries/libdb.so")` + `dl_sym()`. Caches function pointers in a static `LibDb` struct. Provides `Database` and `QueryResult` types with `Drop` impls for automatic resource cleanup.
