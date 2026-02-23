//! Core type definitions for the database engine.
//!
//! Defines column types, values, table schemas, rows, result sets,
//! and error types used throughout libdb.

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

// ── Constants ────────────────────────────────────────────────────────────────

/// Database file page size in bytes.
pub const PAGE_SIZE: usize = 4096;

/// Magic bytes identifying an anyOS database file.
pub const MAGIC: &[u8; 8] = b"ANYDB100";

/// Header size in bytes at the start of page 0.
pub const HEADER_SIZE: usize = 32;

/// Size of a table directory entry in bytes.
pub const TABLE_ENTRY_SIZE: usize = 128;

/// Maximum number of tables per database (limited by page 0 capacity).
pub const MAX_TABLES: usize = (PAGE_SIZE - HEADER_SIZE) / TABLE_ENTRY_SIZE; // 31

/// Maximum columns per table.
pub const MAX_COLUMNS: usize = 8;

/// Maximum table name length (null-terminated in 32 bytes).
pub const MAX_TABLE_NAME: usize = 31;

/// Maximum column name length (null-terminated in 8 bytes).
pub const MAX_COL_NAME: usize = 7;

/// Data page header size in bytes.
pub const DATA_PAGE_HEADER: usize = 8;

/// Usable data area per page.
pub const DATA_AREA_SIZE: usize = PAGE_SIZE - DATA_PAGE_HEADER;

// ── Row tags ─────────────────────────────────────────────────────────────────

/// Row is active (first byte of row).
pub const ROW_ACTIVE: u8 = 0x00;

/// Row is deleted (first byte of row).
pub const ROW_DELETED: u8 = 0xFF;

// ── Value tags ───────────────────────────────────────────────────────────────

/// Tag byte for NULL value.
pub const TAG_NULL: u8 = 0x00;

/// Tag byte for INTEGER value (i64).
pub const TAG_INTEGER: u8 = 0x01;

/// Tag byte for TEXT value (u16 length + bytes).
pub const TAG_TEXT: u8 = 0x02;

// ── Column type ──────────────────────────────────────────────────────────────

/// SQL column type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum ColumnType {
    Integer = 1,
    Text = 2,
}

impl ColumnType {
    /// Decode from on-disk u16 representation.
    pub fn from_u16(v: u16) -> Option<ColumnType> {
        match v {
            1 => Some(ColumnType::Integer),
            2 => Some(ColumnType::Text),
            _ => None,
        }
    }
}

// ── Column definition ────────────────────────────────────────────────────────

/// A single column definition (name + type).
#[derive(Debug, Clone)]
pub struct ColumnDef {
    pub name: String,
    pub col_type: ColumnType,
}

// ── Value ────────────────────────────────────────────────────────────────────

/// A database value (cell contents).
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Integer(i64),
    Text(String),
}

impl Value {
    /// Returns the serialized byte size of this value (tag + data).
    pub fn serialized_size(&self) -> usize {
        match self {
            Value::Null => 1,
            Value::Integer(_) => 9,       // tag + 8 bytes
            Value::Text(s) => 3 + s.len(), // tag + u16 len + data
        }
    }
}

// ── Row ──────────────────────────────────────────────────────────────────────

/// A database row (ordered list of values matching the table schema).
#[derive(Debug, Clone)]
pub struct Row {
    pub values: Vec<Value>,
}

// ── Table schema ─────────────────────────────────────────────────────────────

/// In-memory representation of a table's schema.
#[derive(Debug, Clone)]
pub struct TableSchema {
    pub name: String,
    pub columns: Vec<ColumnDef>,
    pub row_count: u32,
    pub first_data_page: u32,
}

impl TableSchema {
    /// Find a column index by name (case-insensitive).
    pub fn find_column(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.name.eq_ignore_ascii_case(name))
    }
}

// ── Result set ───────────────────────────────────────────────────────────────

/// Query result set returned by SELECT statements.
#[derive(Debug)]
pub struct ResultSet {
    pub col_names: Vec<String>,
    pub col_types: Vec<ColumnType>,
    pub rows: Vec<Row>,
}

// ── Error type ───────────────────────────────────────────────────────────────

/// Database error.
#[derive(Debug)]
pub enum DbError {
    /// File I/O error.
    Io(String),
    /// SQL parse error.
    Parse(String),
    /// Table not found.
    TableNotFound(String),
    /// Table already exists.
    TableExists(String),
    /// Column not found.
    ColumnNotFound(String),
    /// Type mismatch (e.g. inserting text into integer column).
    TypeMismatch(String),
    /// Too many tables (max 31).
    TooManyTables,
    /// Too many columns (max 8).
    TooManyColumns,
    /// Value too large (text > 255 bytes).
    ValueTooLarge,
    /// Corrupt database file.
    Corrupt(String),
    /// Row too large to fit in a single page.
    RowTooLarge,
}

impl DbError {
    /// Format error as a human-readable message.
    pub fn message(&self) -> String {
        match self {
            DbError::Io(s) => {
                let mut m = String::from("I/O error: ");
                m.push_str(s);
                m
            }
            DbError::Parse(s) => {
                let mut m = String::from("Parse error: ");
                m.push_str(s);
                m
            }
            DbError::TableNotFound(s) => {
                let mut m = String::from("Table not found: ");
                m.push_str(s);
                m
            }
            DbError::TableExists(s) => {
                let mut m = String::from("Table already exists: ");
                m.push_str(s);
                m
            }
            DbError::ColumnNotFound(s) => {
                let mut m = String::from("Column not found: ");
                m.push_str(s);
                m
            }
            DbError::TypeMismatch(s) => {
                let mut m = String::from("Type mismatch: ");
                m.push_str(s);
                m
            }
            DbError::TooManyTables => String::from("Too many tables (max 31)"),
            DbError::TooManyColumns => String::from("Too many columns (max 8)"),
            DbError::ValueTooLarge => String::from("Value too large (text max 255 bytes)"),
            DbError::Corrupt(s) => {
                let mut m = String::from("Corrupt database: ");
                m.push_str(s);
                m
            }
            DbError::RowTooLarge => String::from("Row too large for page"),
        }
    }
}

/// Convenience type alias for Results using DbError.
pub type DbResult<T> = Result<T, DbError>;

// ── Parsed SQL AST ───────────────────────────────────────────────────────────

/// A parsed SQL statement.
#[derive(Debug)]
pub enum Statement {
    CreateTable {
        name: String,
        columns: Vec<ColumnDef>,
    },
    DropTable {
        name: String,
    },
    Insert {
        table: String,
        columns: Vec<String>,
        values: Vec<Value>,
    },
    Select {
        table: String,
        columns: SelectColumns,
        where_clause: Option<Expr>,
    },
    Update {
        table: String,
        assignments: Vec<(String, Value)>,
        where_clause: Option<Expr>,
    },
    Delete {
        table: String,
        where_clause: Option<Expr>,
    },
}

/// SELECT column specification.
#[derive(Debug)]
pub enum SelectColumns {
    /// SELECT *
    All,
    /// SELECT col1, col2, ...
    Named(Vec<String>),
}

/// WHERE clause expression.
#[derive(Debug)]
pub enum Expr {
    /// Column reference.
    Column(String),
    /// Literal value.
    Literal(Value),
    /// Binary comparison: col op val.
    BinOp {
        op: CmpOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    /// Logical AND.
    And(Box<Expr>, Box<Expr>),
    /// Logical OR.
    Or(Box<Expr>, Box<Expr>),
}

/// Comparison operators for WHERE clauses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}
