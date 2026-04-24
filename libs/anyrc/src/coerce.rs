//! Type coercion classification.
//!
//! Type checking owns constraint solving. This module owns language-level
//! coercions, so they can grow into a real coercion engine instead of becoming
//! scattered unification exceptions.

use crate::ast::Mutability;
use crate::typeck::TyKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoercionKind {
    RefToRawPtr,
}

pub fn classify(expected: &TyKind, actual: &TyKind) -> Option<CoercionKind> {
    match (expected, actual) {
        (TyKind::RawPtr(_, expected_mut), TyKind::Ref(_, actual_mut))
            if raw_ptr_accepts_ref(*expected_mut, *actual_mut) =>
        {
            Some(CoercionKind::RefToRawPtr)
        }
        _ => None,
    }
}

fn raw_ptr_accepts_ref(expected: Mutability, actual: Mutability) -> bool {
    match (expected, actual) {
        (Mutability::Immutable, _) => true,
        (Mutability::Mut, Mutability::Mut) => true,
        (Mutability::Mut, Mutability::Immutable) => false,
    }
}
