//! Compiler-known language and runtime items.
//!
//! This module is the boundary for names that are not discovered through normal
//! crate metadata yet. Keeping them here prevents resolver, type checking, and
//! MIR lowering from growing independent name-based compatibility tables.

pub struct KnownItem {
    pub local_name: &'static str,
    pub full_path: &'static str,
}

pub const PRELUDE_TYPES: &[KnownItem] = &[
    KnownItem {
        local_name: "Option",
        full_path: "Option",
    },
    KnownItem {
        local_name: "Result",
        full_path: "Result",
    },
    KnownItem {
        local_name: "Vec",
        full_path: "Vec",
    },
    KnownItem {
        local_name: "String",
        full_path: "String",
    },
    KnownItem {
        local_name: "Arguments",
        full_path: "Arguments",
    },
    KnownItem {
        local_name: "Box",
        full_path: "Box",
    },
    KnownItem {
        local_name: "Clone",
        full_path: "Clone",
    },
    KnownItem {
        local_name: "Copy",
        full_path: "Copy",
    },
    KnownItem {
        local_name: "Debug",
        full_path: "Debug",
    },
    KnownItem {
        local_name: "Drop",
        full_path: "Drop",
    },
    KnownItem {
        local_name: "Eq",
        full_path: "Eq",
    },
    KnownItem {
        local_name: "From",
        full_path: "From",
    },
    KnownItem {
        local_name: "Into",
        full_path: "Into",
    },
    KnownItem {
        local_name: "TryFrom",
        full_path: "TryFrom",
    },
    KnownItem {
        local_name: "TryInto",
        full_path: "TryInto",
    },
    KnownItem {
        local_name: "AsRef",
        full_path: "AsRef",
    },
    KnownItem {
        local_name: "AsMut",
        full_path: "AsMut",
    },
    KnownItem {
        local_name: "Iterator",
        full_path: "Iterator",
    },
    KnownItem {
        local_name: "IntoIterator",
        full_path: "IntoIterator",
    },
    KnownItem {
        local_name: "FromIterator",
        full_path: "FromIterator",
    },
    KnownItem {
        local_name: "Extend",
        full_path: "Extend",
    },
    KnownItem {
        local_name: "ExactSizeIterator",
        full_path: "ExactSizeIterator",
    },
    KnownItem {
        local_name: "DoubleEndedIterator",
        full_path: "DoubleEndedIterator",
    },
    KnownItem {
        local_name: "Default",
        full_path: "Default",
    },
    KnownItem {
        local_name: "PartialEq",
        full_path: "PartialEq",
    },
    KnownItem {
        local_name: "PartialOrd",
        full_path: "PartialOrd",
    },
    KnownItem {
        local_name: "Ord",
        full_path: "Ord",
    },
    KnownItem {
        local_name: "Send",
        full_path: "Send",
    },
    KnownItem {
        local_name: "Sync",
        full_path: "Sync",
    },
    KnownItem {
        local_name: "Sized",
        full_path: "Sized",
    },
];

pub const PRELUDE_VALUES: &[KnownItem] = &[
    KnownItem {
        local_name: "Some",
        full_path: "Option::Some",
    },
    KnownItem {
        local_name: "None",
        full_path: "Option::None",
    },
    KnownItem {
        local_name: "Ok",
        full_path: "Result::Ok",
    },
    KnownItem {
        local_name: "Err",
        full_path: "Result::Err",
    },
    KnownItem {
        local_name: "__anyrc_println",
        full_path: "__anyrc_println",
    },
    KnownItem {
        local_name: "__anyrc_format",
        full_path: "__anyrc_format",
    },
    KnownItem {
        local_name: "__anyrc_format_args",
        full_path: "__anyrc_format_args",
    },
    KnownItem {
        local_name: "Vec::new",
        full_path: "Vec::new",
    },
    KnownItem {
        local_name: "exit",
        full_path: "exit",
    },
    KnownItem {
        local_name: "drop",
        full_path: "drop",
    },
];

pub const PRIMITIVE_TYPES: &[&str] = &[
    "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize", "f32",
    "f64", "bool", "char", "str",
];
