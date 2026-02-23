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
        Statement::Select { table, columns, where_clause } => {
            exec_select(db, &table, &columns, where_clause.as_ref())
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

    let table = &db.tables[table_idx];
    let schema_cols = &table.columns;

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
    where_clause: Option<&Expr>,
) -> DbResult<ResultSet> {
    let table_idx = schema::find_table(&db.tables, table_name)
        .ok_or_else(|| DbError::TableNotFound(String::from(table_name)))?;

    let table = &db.tables[table_idx];
    let schema_cols = &table.columns;

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
                let idx = table.find_column(name)
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
            if !eval_where(expr, &row.values, schema_cols)? {
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

    Ok(ResultSet {
        col_names,
        col_types,
        rows: result_rows,
    })
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
