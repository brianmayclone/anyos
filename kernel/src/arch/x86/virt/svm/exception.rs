//! SVM exception intercept policy.

const VECTOR_UD: u32 = 6;
const VECTOR_DF: u32 = 8;
const VECTOR_TS: u32 = 10;
const VECTOR_NP: u32 = 11;
const VECTOR_SS: u32 = 12;
const VECTOR_GP: u32 = 13;

pub(super) const FATAL_INTERCEPTS: u32 = (1 << VECTOR_UD)
    | (1 << VECTOR_DF)
    | (1 << VECTOR_TS)
    | (1 << VECTOR_NP)
    | (1 << VECTOR_SS)
    | (1 << VECTOR_GP);

pub(super) fn is_exception_exit(exit_code: u64) -> bool {
    (0x40..=0x5f).contains(&exit_code)
}

pub(super) fn vector(exit_code: u64) -> u8 {
    (exit_code - 0x40) as u8
}
