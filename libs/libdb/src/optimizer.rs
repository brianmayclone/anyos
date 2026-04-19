//! Tiny query-planning helpers for libdb.

extern crate alloc;

use alloc::vec::Vec;

use crate::types::{CmpOp, Expr, Value};

/// Collects simple equality lookup candidates from an expression tree.
pub fn collect_equality_lookups<'a>(expr: &'a Expr) -> Vec<(&'a str, &'a Value)> {
    let mut out = Vec::new();
    collect(expr, &mut out);
    out
}

fn collect<'a>(expr: &'a Expr, out: &mut Vec<(&'a str, &'a Value)>) {
    match expr {
        Expr::BinOp { op: CmpOp::Eq, left, right } => match (&**left, &**right) {
            (Expr::Column(column), Expr::Literal(value)) => out.push((column.as_str(), value)),
            (Expr::Literal(value), Expr::Column(column)) => out.push((column.as_str(), value)),
            _ => {}
        },
        Expr::And(left, right) => {
            collect(left, out);
            collect(right, out);
        }
        _ => {}
    }
}
