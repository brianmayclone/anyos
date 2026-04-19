//! SQL query executor.
//!
//! Takes a parsed [`Statement`] AST and executes it against the storage engine,
//! producing either a row count (for DDL/DML) or a [`ResultSet`] (for SELECT).

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use crate::types::*;
use crate::engine::Database;
use crate::schema;

/// Execute a non-query statement (CREATE, DROP, INSERT, UPDATE, DELETE).
/// Returns the number of rows affected.
pub fn exec(db: &mut Database, stmt: Statement) -> DbResult<u32> {
    match stmt {
        Statement::CreateTable { name, columns } => {
            db.create_table(&name, &columns)?;
            Ok(0)
        }
        Statement::DropTable { name } => {
            db.drop_table(&name)?;
            Ok(0)
        }
        Statement::AlterTableAddColumn { table, column } => {
            db.add_column(&table, column)?;
            Ok(0)
        }
        Statement::Insert { table, columns, values } => {
            exec_insert(db, &table, &columns, &values)
        }
        Statement::Update { table, assignments, where_clause } => {
            exec_update(db, &table, &assignments, where_clause.as_ref())
        }
        Statement::Delete { table, where_clause } => {
            exec_delete(db, &table, where_clause.as_ref())
        }
        Statement::Select { .. } => {
            Err(DbError::Parse(String::from("Use query() for SELECT statements")))
        }
    }
}

/// Execute a SELECT query and return a result set.
pub fn query(db: &mut Database, stmt: Statement) -> DbResult<ResultSet> {
    match stmt {
        Statement::Select { table, columns, distinct, where_clause, order_by, limit, offset } => {
            exec_select(db, &table, &columns, distinct, where_clause.as_ref(), &order_by, limit, offset)
        }
        _ => Err(DbError::Parse(String::from("Expected SELECT statement"))),
    }
}

// ── INSERT ───────────────────────────────────────────────────────────────────

fn exec_insert(
    db: &mut Database,
    table_name: &str,
    col_names: &[String],
    values: &[Value],
) -> DbResult<u32> {
    let table_idx = schema::find_table(&db.tables, table_name)
        .ok_or_else(|| DbError::TableNotFound(String::from(table_name)))?;

    let schema_cols = db.tables[table_idx].columns.clone();

    // Build the row values in schema column order
    let row_values = if col_names.is_empty() {
        // No explicit columns — values must match schema order and count
        if values.len() != schema_cols.len() {
            let mut msg = String::from("Expected ");
            fmt_usize(&mut msg, schema_cols.len());
            msg.push_str(" values, got ");
            fmt_usize(&mut msg, values.len());
            return Err(DbError::TypeMismatch(msg));
        }
        // Type-check each value
        for (i, val) in values.iter().enumerate() {
            validate_type(val, &schema_cols[i])?;
        }
        values.to_vec()
    } else {
        // Explicit column list — map named columns to schema positions
        if col_names.len() != values.len() {
            return Err(DbError::TypeMismatch(String::from(
                "Column count does not match value count",
            )));
        }
        let mut row = Vec::with_capacity(schema_cols.len());
        for sc in schema_cols.iter() {
            let idx = col_names.iter().position(|c| c.eq_ignore_ascii_case(&sc.name));
            match idx {
                Some(i) => {
                    validate_type(&values[i], sc)?;
                    row.push(values[i].clone());
                }
                None => row.push(Value::Null),
            }
        }
        row
    };

    db.insert_row(table_idx, &row_values)?;
    Ok(1)
}

// ── SELECT ───────────────────────────────────────────────────────────────────

fn exec_select(
    db: &mut Database,
    table_name: &str,
    columns: &SelectColumns,
    distinct: bool,
    where_clause: Option<&Expr>,
    order_by: &[OrderBy],
    limit: Option<u64>,
    offset: Option<u64>,
) -> DbResult<ResultSet> {
    let table_idx = schema::find_table(&db.tables, table_name)
        .ok_or_else(|| DbError::TableNotFound(String::from(table_name)))?;

    let schema_cols = db.tables[table_idx].columns.clone();

    // Determine which columns to output
    let (col_indices, col_names, col_types) = match columns {
        SelectColumns::All => {
            let indices: Vec<usize> = (0..schema_cols.len()).collect();
            let names: Vec<String> = schema_cols.iter().map(|c| c.name.clone()).collect();
            let types: Vec<ColumnType> = schema_cols.iter().map(|c| c.col_type).collect();
            (indices, names, types)
        }
        SelectColumns::Named(names) => {
            let mut indices = Vec::with_capacity(names.len());
            let mut out_names = Vec::with_capacity(names.len());
            let mut out_types = Vec::with_capacity(names.len());
            for name in names {
                let idx = schema_cols.iter().position(|c| c.name.eq_ignore_ascii_case(name))
                    .ok_or_else(|| DbError::ColumnNotFound(name.clone()))?;
                indices.push(idx);
                out_names.push(schema_cols[idx].name.clone());
                out_types.push(schema_cols[idx].col_type);
            }
            (indices, out_names, out_types)
        }
    };

    // Scan and filter rows
    let all_rows = db.scan_table(table_idx)?;
    let mut result_rows = Vec::new();

    for (_page, _offset, row) in &all_rows {
        if let Some(expr) = where_clause {
            if !eval_where(expr, &row.values, &schema_cols)? {
                continue;
            }
        }
        // Project selected columns
        let projected: Vec<Value> = col_indices.iter().map(|&i| {
            if i < row.values.len() {
                row.values[i].clone()
            } else {
                Value::Null
            }
        }).collect();
        result_rows.push(Row { values: projected });
    }

    // ORDER BY
    if !order_by.is_empty() {
        // Resolve order-by column indices in the projected result
        let mut sort_specs: Vec<(usize, bool)> = Vec::new();
        for ob in order_by {
            let idx = col_names.iter().position(|n| n.eq_ignore_ascii_case(&ob.column))
                .ok_or_else(|| DbError::ColumnNotFound(ob.column.clone()))?;
            sort_specs.push((idx, ob.ascending));
        }
        result_rows.sort_by(|a, b| {
            for &(idx, asc) in &sort_specs {
                let va = a.values.get(idx).unwrap_or(&Value::Null);
                let vb = b.values.get(idx).unwrap_or(&Value::Null);
                let cmp = cmp_values(va, vb);
                if cmp != core::cmp::Ordering::Equal {
                    return if asc { cmp } else { cmp.reverse() };
                }
            }
            core::cmp::Ordering::Equal
        });
    }

    // DISTINCT
    if distinct {
        let mut unique: Vec<Row> = Vec::new();
        for row in result_rows {
            let is_dup = unique.iter().any(|u| u.values == row.values);
            if !is_dup {
                unique.push(row);
            }
        }
        result_rows = unique;
    }

    // OFFSET
    if let Some(off) = offset {
        let off = off as usize;
        if off >= result_rows.len() {
            result_rows.clear();
        } else {
            result_rows = result_rows.split_off(off);
        }
    }

    // LIMIT
    if let Some(lim) = limit {
        result_rows.truncate(lim as usize);
    }

    Ok(ResultSet {
        col_names,
        col_types,
        rows: result_rows,
    })
}

/// Compare two Values for ordering (NULL is smallest).
fn cmp_values(a: &Value, b: &Value) -> core::cmp::Ordering {
    match (a, b) {
        (Value::Null, Value::Null) => core::cmp::Ordering::Equal,
        (Value::Null, _) => core::cmp::Ordering::Less,
        (_, Value::Null) => core::cmp::Ordering::Greater,
        (Value::Integer(x), Value::Integer(y)) => x.cmp(y),
        (Value::Text(x), Value::Text(y)) => x.cmp(y),
        (Value::Blob(x), Value::Blob(y)) => x.cmp(y),
        (Value::Integer(x), Value::Text(_)) => {
            // Integer before text
            core::cmp::Ordering::Less
        }
        (Value::Text(_), Value::Integer(_)) => core::cmp::Ordering::Greater,
        (Value::Blob(_), Value::Integer(_) | Value::Text(_)) => core::cmp::Ordering::Greater,
        (Value::Integer(_) | Value::Text(_), Value::Blob(_)) => core::cmp::Ordering::Less,
    }
}

// ── UPDATE ───────────────────────────────────────────────────────────────────

fn exec_update(
    db: &mut Database,
    table_name: &str,
    assignments: &[(String, Value)],
    where_clause: Option<&Expr>,
) -> DbResult<u32> {
    let table_idx = schema::find_table(&db.tables, table_name)
        .ok_or_else(|| DbError::TableNotFound(String::from(table_name)))?;

    let schema_cols = db.tables[table_idx].columns.clone();

    // Resolve assignment column indices and validate types
    let mut assign_indices = Vec::with_capacity(assignments.len());
    for (col_name, val) in assignments {
        let idx = db.tables[table_idx].find_column(col_name)
            .ok_or_else(|| DbError::ColumnNotFound(col_name.clone()))?;
        validate_type(val, &schema_cols[idx])?;
        assign_indices.push((idx, val.clone()));
    }

    // Scan for matching rows
    let all_rows = db.scan_table(table_idx)?;
    let mut to_update: Vec<(u32, usize, Vec<Value>)> = Vec::new();

    for (page, offset, row) in &all_rows {
        if let Some(expr) = where_clause {
            if !eval_where(expr, &row.values, &schema_cols)? {
                continue;
            }
        }
        // Build updated row
        let mut new_values = row.values.clone();
        for (idx, val) in &assign_indices {
            if *idx < new_values.len() {
                new_values[*idx] = val.clone();
            }
        }
        to_update.push((*page, *offset, new_values));
    }

    let count = to_update.len() as u32;
    for (page, offset, new_values) in to_update {
        db.update_row(table_idx, page, offset, &new_values)?;
    }

    Ok(count)
}

// ── DELETE ───────────────────────────────────────────────────────────────────

fn exec_delete(
    db: &mut Database,
    table_name: &str,
    where_clause: Option<&Expr>,
) -> DbResult<u32> {
    let table_idx = schema::find_table(&db.tables, table_name)
        .ok_or_else(|| DbError::TableNotFound(String::from(table_name)))?;

    let schema_cols = db.tables[table_idx].columns.clone();

    // Scan for matching rows
    let all_rows = db.scan_table(table_idx)?;
    let mut to_delete: Vec<(u32, usize)> = Vec::new();

    for (page, offset, row) in &all_rows {
        if let Some(expr) = where_clause {
            if !eval_where(expr, &row.values, &schema_cols)? {
                continue;
            }
        }
        to_delete.push((*page, *offset));
    }

    let count = to_delete.len() as u32;
    // Delete in reverse order to avoid offset shifts within same page
    for (page, offset) in to_delete.into_iter().rev() {
        db.delete_row(table_idx, page, offset)?;
    }

    Ok(count)
}

// ── WHERE expression evaluation ──────────────────────────────────────────────

/// Evaluate a WHERE expression against a row's values.
fn eval_where(expr: &Expr, values: &[Value], schema: &[ColumnDef]) -> DbResult<bool> {
    match expr {
        Expr::BinOp { op, left, right } => {
            let lval = eval_value(left, values, schema)?;
            let rval = eval_value(right, values, schema)?;
            Ok(compare_values(&lval, &rval, *op))
        }
        Expr::And(l, r) => {
            Ok(eval_where(l, values, schema)? && eval_where(r, values, schema)?)
        }
        Expr::Or(l, r) => {
            Ok(eval_where(l, values, schema)? || eval_where(r, values, schema)?)
        }
        Expr::Not(inner) => {
            Ok(!eval_where(inner, values, schema)?)
        }
        Expr::IsNull(inner) => {
            let val = eval_value(inner, values, schema)?;
            Ok(matches!(val, Value::Null))
        }
        Expr::IsNotNull(inner) => {
            let val = eval_value(inner, values, schema)?;
            Ok(!matches!(val, Value::Null))
        }
        Expr::Like { expr, pattern } => {
            let val = eval_value(expr, values, schema)?;
            match val {
                Value::Text(ref s) => Ok(like_match(s, pattern)),
                Value::Null => Ok(false),
                Value::Integer(v) => {
                    let mut s = String::new();
                    fmt_i64_val(&mut s, v);
                    Ok(like_match(&s, pattern))
                }
                Value::Blob(_) => Ok(false),
            }
        }
        Expr::NotLike { expr, pattern } => {
            let val = eval_value(expr, values, schema)?;
            match val {
                Value::Text(ref s) => Ok(!like_match(s, pattern)),
                Value::Null => Ok(false),
                Value::Integer(v) => {
                    let mut s = String::new();
                    fmt_i64_val(&mut s, v);
                    Ok(!like_match(&s, pattern))
                }
                Value::Blob(_) => Ok(false),
            }
        }
        Expr::Literal(Value::Integer(0)) => Ok(false),
        Expr::Literal(_) => Ok(true),
        Expr::Column(name) => {
            // Truthy check: non-null, non-zero
            let idx = schema.iter().position(|c| c.name.eq_ignore_ascii_case(name))
                .ok_or_else(|| DbError::ColumnNotFound(name.clone()))?;
            if idx < values.len() {
                match &values[idx] {
                    Value::Null => Ok(false),
                    Value::Integer(0) => Ok(false),
                    _ => Ok(true),
                }
            } else {
                Ok(false)
            }
        }
    }
}

/// SQL LIKE pattern matching (case-insensitive).
/// `%` matches any sequence of characters, `_` matches any single character.
fn like_match(text: &str, pattern: &str) -> bool {
    let t = text.as_bytes();
    let p = pattern.as_bytes();
    like_match_impl(t, p, 0, 0)
}

fn like_match_impl(t: &[u8], p: &[u8], ti: usize, pi: usize) -> bool {
    let mut ti = ti;
    let mut pi = pi;

    loop {
        if pi >= p.len() {
            return ti >= t.len();
        }
        if p[pi] == b'%' {
            pi += 1;
            // Skip consecutive %
            while pi < p.len() && p[pi] == b'%' { pi += 1; }
            if pi >= p.len() { return true; }
            // Try matching rest from every position
            while ti <= t.len() {
                if like_match_impl(t, p, ti, pi) { return true; }
                ti += 1;
            }
            return false;
        }
        if ti >= t.len() { return false; }
        if p[pi] == b'_' || to_lower_byte(t[ti]) == to_lower_byte(p[pi]) {
            ti += 1;
            pi += 1;
        } else {
            return false;
        }
    }
}

fn to_lower_byte(b: u8) -> u8 {
    if b >= b'A' && b <= b'Z' { b + 32 } else { b }
}

fn fmt_i64_val(out: &mut String, v: i64) {
    if v == 0 { out.push('0'); return; }
    let (neg, abs) = if v < 0 { (true, (-(v + 1)) as u64 + 1) } else { (false, v as u64) };
    if neg { out.push('-'); }
    let mut buf = [0u8; 20];
    let mut n = 0;
    let mut val = abs;
    while val > 0 { buf[n] = b'0' + (val % 10) as u8; val /= 10; n += 1; }
    for i in (0..n).rev() { out.push(buf[i] as char); }
}

/// Resolve an expression to a concrete value.
fn eval_value(expr: &Expr, values: &[Value], schema: &[ColumnDef]) -> DbResult<Value> {
    match expr {
        Expr::Literal(v) => Ok(v.clone()),
        Expr::Column(name) => {
            let idx = schema.iter().position(|c| c.name.eq_ignore_ascii_case(name))
                .ok_or_else(|| DbError::ColumnNotFound(name.clone()))?;
            if idx < values.len() {
                Ok(values[idx].clone())
            } else {
                Ok(Value::Null)
            }
        }
        _ => Err(DbError::Parse(String::from("Complex expression in comparison"))),
    }
}

/// Compare two values with a comparison operator.
fn compare_values(left: &Value, right: &Value, op: CmpOp) -> bool {
    match (left, right) {
        (Value::Null, Value::Null) => matches!(op, CmpOp::Eq),
        (Value::Null, _) | (_, Value::Null) => matches!(op, CmpOp::Ne),
        (Value::Integer(a), Value::Integer(b)) => {
            match op {
                CmpOp::Eq => a == b,
                CmpOp::Ne => a != b,
                CmpOp::Lt => a < b,
                CmpOp::Gt => a > b,
                CmpOp::Le => a <= b,
                CmpOp::Ge => a >= b,
            }
        }
        (Value::Text(a), Value::Text(b)) => {
            match op {
                CmpOp::Eq => a.eq_ignore_ascii_case(b),
                CmpOp::Ne => !a.eq_ignore_ascii_case(b),
                CmpOp::Lt => a < b,
                CmpOp::Gt => a > b,
                CmpOp::Le => a <= b,
                CmpOp::Ge => a >= b,
            }
        }
        (Value::Blob(a), Value::Blob(b)) => {
            match op {
                CmpOp::Eq => a == b,
                CmpOp::Ne => a != b,
                CmpOp::Lt => a < b,
                CmpOp::Gt => a > b,
                CmpOp::Le => a <= b,
                CmpOp::Ge => a >= b,
            }
        }
        // Cross-type comparison: integer vs text
        (Value::Integer(a), Value::Text(b)) => {
            // Try parsing text as integer
            if let Some(bv) = parse_int(b) {
                compare_values(&Value::Integer(*a), &Value::Integer(bv), op)
            } else {
                matches!(op, CmpOp::Ne)
            }
        }
        (Value::Text(a), Value::Integer(b)) => {
            if let Some(av) = parse_int(a) {
                compare_values(&Value::Integer(av), &Value::Integer(*b), op)
            } else {
                matches!(op, CmpOp::Ne)
            }
        }
        (Value::Blob(_), _) | (_, Value::Blob(_)) => matches!(op, CmpOp::Ne),
    }
}

/// Try to parse a string as i64.
fn parse_int(s: &str) -> Option<i64> {
    let bytes = s.as_bytes();
    if bytes.is_empty() { return None; }
    let (neg, start) = if bytes[0] == b'-' { (true, 1) } else { (false, 0) };
    if start >= bytes.len() { return None; }
    let mut val: i64 = 0;
    for &b in &bytes[start..] {
        if !b.is_ascii_digit() { return None; }
        val = val.checked_mul(10)?.checked_add((b - b'0') as i64)?;
    }
    if neg { Some(-val) } else { Some(val) }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Validate that a value matches the expected column type.
fn validate_type(val: &Value, col: &ColumnDef) -> DbResult<()> {
    match (val, col.col_type) {
        (Value::Null, _) => Ok(()), // NULL is always valid
        (Value::Integer(_), ColumnType::Integer) => Ok(()),
        (Value::Text(_), ColumnType::Text) => Ok(()),
        (Value::Blob(_), ColumnType::Blob) => Ok(()),
        (Value::Integer(_), ColumnType::Text) => {
            let mut msg = String::from("Column '");
            msg.push_str(&col.name);
            msg.push_str("' expects TEXT, got INTEGER");
            Err(DbError::TypeMismatch(msg))
        }
        (Value::Text(_), ColumnType::Integer) => {
            let mut msg = String::from("Column '");
            msg.push_str(&col.name);
            msg.push_str("' expects INTEGER, got TEXT");
            Err(DbError::TypeMismatch(msg))
        }
        (Value::Integer(_), ColumnType::Blob) => {
            let mut msg = String::from("Column '");
            msg.push_str(&col.name);
            msg.push_str("' expects BLOB, got INTEGER");
            Err(DbError::TypeMismatch(msg))
        }
        (Value::Text(_), ColumnType::Blob) => {
            let mut msg = String::from("Column '");
            msg.push_str(&col.name);
            msg.push_str("' expects BLOB, got TEXT");
            Err(DbError::TypeMismatch(msg))
        }
        (Value::Blob(_), ColumnType::Integer) => {
            let mut msg = String::from("Column '");
            msg.push_str(&col.name);
            msg.push_str("' expects INTEGER, got BLOB");
            Err(DbError::TypeMismatch(msg))
        }
        (Value::Blob(_), ColumnType::Text) => {
            let mut msg = String::from("Column '");
            msg.push_str(&col.name);
            msg.push_str("' expects TEXT, got BLOB");
            Err(DbError::TypeMismatch(msg))
        }
    }
}

/// Format a usize into a string (no_std helper).
fn fmt_usize(out: &mut String, v: usize) {
    if v == 0 { out.push('0'); return; }
    let mut buf = [0u8; 20];
    let mut n = 0;
    let mut val = v;
    while val > 0 {
        buf[n] = b'0' + (val % 10) as u8;
        val /= 10;
        n += 1;
    }
    for i in (0..n).rev() {
        out.push(buf[i] as char);
    }
}
