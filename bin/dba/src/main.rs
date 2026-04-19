#![no_std]
#![no_main]

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use anyos_std::fs;
use libdb_client::{Database, QueryResult};

anyos_std::entry!(main);

const MAX_INPUT: usize = 2048;
const PAGE_SIZE: usize = 4096;
const MAGIC_V1: &[u8; 8] = b"ANYDB100";
const MAGIC_V2: &[u8; 8] = b"ANYDB200";
const HEADER_SIZE: usize = 32;
const TABLE_ENTRY_SIZE: usize = 256;

#[derive(Clone, Copy, PartialEq, Eq)]
enum DbFormat {
    V1,
    V2,
}

#[derive(Clone)]
struct ColumnMeta {
    name: String,
    ty: String,
}

#[derive(Clone)]
struct IndexMeta {
    name: String,
    column: String,
    unique: bool,
}

#[derive(Clone)]
struct TableMeta {
    name: String,
    columns: Vec<ColumnMeta>,
    indexes: Vec<IndexMeta>,
}

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"");

    if raw.contains("--help") || raw.contains("-h") {
        print_help();
        return;
    }

    let Some(path) = args.pos(0) else {
        print_help();
        return;
    };

    if !libdb_client::init() {
        anyos_std::println!("dba: failed to load libdb.so");
        anyos_std::process::exit(1);
    }

    let db = match Database::open(path) {
        Some(db) => db,
        None => {
            anyos_std::println!("dba: cannot open '{}'", path);
            anyos_std::process::exit(1);
        }
    };

    if let Some(sql) = args.pos(1) {
        run_command(&db, path, sql);
        return;
    }

    anyos_std::println!("dba - libdb console");
    anyos_std::println!("Opened {}", path);
    anyos_std::println!("Commands: .tables, .schema [table], .help, .quit");
    anyos_std::println!("");

    let mut input_buf = [0u8; MAX_INPUT];
    loop {
        anyos_std::print!("dba> ");
        let line = read_line(&mut input_buf);
        if line.is_empty() {
            continue;
        }
        if is_quit_command(line) {
            break;
        }
        run_command(&db, path, line);
    }
}

fn print_help() {
    anyos_std::println!("dba - libdb console");
    anyos_std::println!("");
    anyos_std::println!("Usage:");
    anyos_std::println!("  dba <database.db>              Start interactive REPL");
    anyos_std::println!("  dba <database.db> \"SELECT * FROM t\"  Run one SQL statement");
    anyos_std::println!("");
    anyos_std::println!("Meta commands:");
    anyos_std::println!("  .tables               List table names");
    anyos_std::println!("  .schema               Print schema for all tables");
    anyos_std::println!("  .schema <table>       Print schema for one table");
    anyos_std::println!("  .help                 Show help");
    anyos_std::println!("  .quit                 Exit");
}

fn run_command(db: &Database, path: &str, line: &str) {
    let trimmed = trim_statement(line);
    if trimmed.is_empty() {
        return;
    }

    if trimmed == ".help" || trimmed == "help" {
        print_help();
        return;
    }

    if trimmed == ".tables" {
        cmd_tables(path);
        return;
    }

    if trimmed.starts_with(".schema") {
        cmd_schema(path, trimmed);
        return;
    }

    if is_select(trimmed) {
        match db.query(trimmed) {
            Ok(result) => print_result(&result),
            Err(err) => anyos_std::println!("Error: {}", err),
        }
        return;
    }

    match db.exec(trimmed) {
        Ok(affected) => {
            let _ = db.flush();
            anyos_std::println!("OK ({} rows affected)", affected);
        }
        Err(err) => anyos_std::println!("Error: {}", err),
    }
}

fn cmd_tables(path: &str) {
    match read_schema(path) {
        Ok(tables) => {
            if tables.is_empty() {
                anyos_std::println!("(no tables)");
                return;
            }
            for table in &tables {
                anyos_std::println!("{}", table.name);
            }
        }
        Err(err) => anyos_std::println!("Error: {}", err),
    }
}

fn cmd_schema(path: &str, line: &str) {
    let target = line[".schema".len()..].trim();
    match read_schema(path) {
        Ok(tables) => {
            if tables.is_empty() {
                anyos_std::println!("(no tables)");
                return;
            }

            let mut printed = false;
            for table in &tables {
                if !target.is_empty() && !table.name.eq_ignore_ascii_case(target) {
                    continue;
                }
                print_table_schema(table);
                printed = true;
            }

            if !printed {
                anyos_std::println!("Error: table not found: {}", target);
            }
        }
        Err(err) => anyos_std::println!("Error: {}", err),
    }
}

fn print_table_schema(table: &TableMeta) {
    let mut create = format!("CREATE TABLE {} (", table.name);
    for (idx, col) in table.columns.iter().enumerate() {
        if idx > 0 {
            create.push_str(", ");
        }
        create.push_str(&col.name);
        create.push(' ');
        create.push_str(&col.ty);
    }
    create.push_str(");");
    anyos_std::println!("{}", create);
    for index in &table.indexes {
        let mut line = String::from("CREATE ");
        if index.unique {
            line.push_str("UNIQUE ");
        }
        line.push_str("INDEX ");
        line.push_str(&index.name);
        line.push_str(" ON ");
        line.push_str(&table.name);
        line.push_str(" (");
        line.push_str(&index.column);
        line.push_str(");");
        anyos_std::println!("{}", line);
    }
    anyos_std::println!("");
}

fn print_result(result: &QueryResult) {
    let rows = result.row_count() as usize;
    let cols = result.col_count() as usize;
    if cols == 0 {
        anyos_std::println!("(0 columns)");
        return;
    }

    let mut headers = Vec::with_capacity(cols);
    let mut widths = Vec::with_capacity(cols);
    for col in 0..cols {
        let name = result.col_name(col as u32);
        widths.push(name.len());
        headers.push(name);
    }

    let mut cells: Vec<Vec<String>> = Vec::with_capacity(rows);
    for row in 0..rows {
        let mut rendered = Vec::with_capacity(cols);
        for col in 0..cols {
            let cell = render_cell(result, row as u32, col as u32);
            if cell.len() > widths[col] {
                widths[col] = cell.len();
            }
            rendered.push(cell);
        }
        cells.push(rendered);
    }

    print_row(&headers, &widths);
    print_separator(&widths);
    for row in &cells {
        print_row(row, &widths);
    }
    anyos_std::println!("({} rows)", rows);
}

fn render_cell(result: &QueryResult, row: u32, col: u32) -> String {
    if result.is_null(row, col) {
        return String::from("NULL");
    }
    if let Some(v) = result.get_int(row, col) {
        return format!("{}", v);
    }
    if let Some(text) = result.get_text(row, col) {
        return text;
    }
    if let Some(blob) = result.get_blob(row, col) {
        return format!("<BLOB {} bytes>", blob.len());
    }
    String::new()
}

fn print_row(values: &[String], widths: &[usize]) {
    for idx in 0..values.len() {
        if idx > 0 {
            anyos_std::print!(" | ");
        }
        anyos_std::print!("{}", values[idx]);
        let pad = widths[idx].saturating_sub(values[idx].len());
        for _ in 0..pad {
            anyos_std::print!(" ");
        }
    }
    anyos_std::print!("\n");
}

fn print_separator(widths: &[usize]) {
    for idx in 0..widths.len() {
        if idx > 0 {
            anyos_std::print!("-+-");
        }
        for _ in 0..widths[idx] {
            anyos_std::print!("-");
        }
    }
    anyos_std::print!("\n");
}

fn read_schema(path: &str) -> Result<Vec<TableMeta>, String> {
    let data = fs::read_to_vec(path).map_err(|_| format!("cannot read '{}'", path))?;
    if data.len() < PAGE_SIZE {
        return Ok(Vec::new());
    }

    let format = detect_format(&data[..PAGE_SIZE])?;
    match format {
        DbFormat::V1 => read_schema_v1(&data),
        DbFormat::V2 => read_schema_v2(&data),
    }
}

fn detect_format(page0: &[u8]) -> Result<DbFormat, String> {
    if page0.len() < 8 {
        return Err(String::from("database file too small"));
    }
    if &page0[..8] == MAGIC_V1 {
        Ok(DbFormat::V1)
    } else if &page0[..8] == MAGIC_V2 {
        Ok(DbFormat::V2)
    } else {
        Err(String::from("invalid libdb magic"))
    }
}

fn read_schema_v1(data: &[u8]) -> Result<Vec<TableMeta>, String> {
    let table_count = read_u32(data, 12)? as usize;
    let mut tables = Vec::with_capacity(table_count);
    for idx in 0..table_count {
        let off = HEADER_SIZE + idx * TABLE_ENTRY_SIZE;
        if off + TABLE_ENTRY_SIZE > PAGE_SIZE || off + TABLE_ENTRY_SIZE > data.len() {
            return Err(String::from("corrupt v1 table directory"));
        }
        let entry = &data[off..off + TABLE_ENTRY_SIZE];
        let name = read_cstr(&entry[..32]);
        if name.is_empty() {
            continue;
        }

        let col_count = read_u16(entry, 32)? as usize;
        let mut columns = Vec::with_capacity(col_count);
        for cidx in 0..col_count {
            let coff = 48 + cidx * 26;
            if coff + 26 > entry.len() {
                return Err(String::from("corrupt v1 column definition"));
            }
            let cname = read_cstr(&entry[coff..coff + 24]);
            let cty = match read_u16(entry, coff + 24)? {
                1 => "INTEGER",
                2 => "TEXT",
                3 => "BLOB",
                _ => "?",
            };
            columns.push(ColumnMeta {
                name: cname,
                ty: String::from(cty),
            });
        }
        tables.push(TableMeta {
            name,
            columns,
            indexes: Vec::new(),
        });
    }
    Ok(tables)
}

fn read_schema_v2(data: &[u8]) -> Result<Vec<TableMeta>, String> {
    let mut page_num = read_u32(data, 20)?;
    let mut tables = Vec::new();
    while page_num != 0 {
        let page = page_slice(data, page_num)?;
        let next_page = read_u32(page, 0)?;
        let schema_page = read_u32(page, 12)?;
        let name_len = read_u16(page, 16)? as usize;
        if name_len == 0 || 18 + name_len > page.len() {
            return Err(String::from("corrupt v2 table directory entry"));
        }
        let name = str_from_bytes(&page[18..18 + name_len])?;
        let (columns, indexes) = read_schema_definition_v2(data, schema_page)?;
        tables.push(TableMeta {
            name,
            columns,
            indexes,
        });
        page_num = next_page;
    }
    Ok(tables)
}

fn read_schema_definition_v2(data: &[u8], mut page_num: u32) -> Result<(Vec<ColumnMeta>, Vec<IndexMeta>), String> {
    let mut columns = Vec::new();
    let mut indexes = Vec::new();

    while page_num != 0 {
        let page = page_slice(data, page_num)?;
        let next_page = read_u32(page, 0)?;
        let used_end = read_u16(page, 4)? as usize;
        let end = if used_end == 0 { 8 } else { used_end.min(page.len()) };
        let structured = page[6] == 1;
        let mut pos = 8usize;

        while pos < end {
            if structured {
                let tag = page[pos];
                pos += 1;
                match tag {
                    0 => break,
                    1 => {
                        if pos + 2 > end {
                            return Err(String::from("corrupt v2 schema column"));
                        }
                        let name_len = page[pos] as usize;
                        let ty = column_type_name(page[pos + 1]);
                        pos += 2;
                        if pos + name_len > end {
                            return Err(String::from("corrupt v2 schema column name"));
                        }
                        columns.push(ColumnMeta {
                            name: str_from_bytes(&page[pos..pos + name_len])?,
                            ty: String::from(ty),
                        });
                        pos += name_len;
                    }
                    2 => {
                        if pos + 3 > end {
                            return Err(String::from("corrupt v2 schema index"));
                        }
                        let name_len = page[pos] as usize;
                        let col_len = page[pos + 1] as usize;
                        let flags = page[pos + 2];
                        pos += 3;
                        if pos + name_len + col_len > end {
                            return Err(String::from("corrupt v2 schema index name"));
                        }
                        let name = str_from_bytes(&page[pos..pos + name_len])?;
                        pos += name_len;
                        let column = str_from_bytes(&page[pos..pos + col_len])?;
                        pos += col_len;
                        indexes.push(IndexMeta {
                            name,
                            column,
                            unique: (flags & 0x01) != 0,
                        });
                    }
                    _ => return Err(String::from("unknown v2 schema tag")),
                }
            } else {
                if pos + 2 > end {
                    break;
                }
                let name_len = page[pos] as usize;
                let ty = column_type_name(page[pos + 1]);
                pos += 2;
                if pos + name_len > end {
                    return Err(String::from("corrupt v2 legacy schema entry"));
                }
                columns.push(ColumnMeta {
                    name: str_from_bytes(&page[pos..pos + name_len])?,
                    ty: String::from(ty),
                });
                pos += name_len;
            }
        }

        page_num = next_page;
    }

    Ok((columns, indexes))
}

fn page_slice(data: &[u8], page_num: u32) -> Result<&[u8], String> {
    let off = page_num as usize * PAGE_SIZE;
    if off + PAGE_SIZE > data.len() {
        return Err(String::from("page reference outside file"));
    }
    Ok(&data[off..off + PAGE_SIZE])
}

fn read_cstr(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    let slice = &bytes[..end];
    str_from_bytes(slice).unwrap_or_default()
}

fn str_from_bytes(bytes: &[u8]) -> Result<String, String> {
    let s = core::str::from_utf8(bytes).map_err(|_| String::from("invalid utf-8 in schema"))?;
    Ok(String::from(s))
}

fn read_u16(bytes: &[u8], off: usize) -> Result<u16, String> {
    if off + 2 > bytes.len() {
        return Err(String::from("short read"));
    }
    Ok(u16::from_le_bytes([bytes[off], bytes[off + 1]]))
}

fn read_u32(bytes: &[u8], off: usize) -> Result<u32, String> {
    if off + 4 > bytes.len() {
        return Err(String::from("short read"));
    }
    Ok(u32::from_le_bytes([
        bytes[off],
        bytes[off + 1],
        bytes[off + 2],
        bytes[off + 3],
    ]))
}

fn column_type_name(raw: u8) -> &'static str {
    match raw {
        1 => "INTEGER",
        2 => "TEXT",
        3 => "BLOB",
        _ => "?",
    }
}

fn is_select(sql: &str) -> bool {
    let first = sql.split_whitespace().next().unwrap_or("");
    first.eq_ignore_ascii_case("select")
}

fn trim_statement(line: &str) -> &str {
    let s = line.trim();
    if s.ends_with(';') {
        s[..s.len() - 1].trim()
    } else {
        s
    }
}

fn is_quit_command(line: &str) -> bool {
    let s = trim_statement(line);
    s == ".quit" || s == ".q" || s.eq_ignore_ascii_case("quit") || s.eq_ignore_ascii_case("exit")
}

fn read_line(buf: &mut [u8; MAX_INPUT]) -> &str {
    let mut pos = 0usize;
    loop {
        let mut byte = [0u8; 1];
        let n = anyos_std::fs::read(0, &mut byte);
        if n == 0 {
            anyos_std::process::sleep(10);
            continue;
        }
        if n == u32::MAX {
            return "";
        }
        match byte[0] {
            b'\n' | b'\r' => {
                anyos_std::print!("\n");
                break;
            }
            8 | 127 => {
                if pos > 0 {
                    pos -= 1;
                    anyos_std::print!("\x08 \x08");
                }
            }
            b if b >= 0x20 && pos < MAX_INPUT - 1 => {
                buf[pos] = b;
                pos += 1;
                anyos_std::print!("{}", b as char);
            }
            _ => {}
        }
    }
    core::str::from_utf8(&buf[..pos]).unwrap_or("").trim()
}
