//! Collection types for anyOS programs.
//!
//! Provides the same collection types as `std::collections`:
//! - [`HashMap`] — hash map (FNV-1a, open addressing)
//! - [`HashSet`] — hash set built on HashMap
//! - [`BTreeMap`] — sorted map (B-tree, from alloc)
//! - [`BTreeSet`] — sorted set (B-tree, from alloc)
//! - [`VecDeque`] — double-ended queue (from alloc)
//! - [`BinaryHeap`] — priority queue (from alloc)
//! - [`LinkedList`] — doubly-linked list (from alloc)
//!
//! # Usage
//! ```ignore
//! use anyos_std::collections::{HashMap, HashSet, BTreeMap, VecDeque};
//! ```

pub mod hash_map;
pub mod hash_set;

pub use hash_map::{HashMap, Iter, IterMut, Keys, Values, IntoIter};
pub use hash_set::HashSet;

// Re-export alloc collections directly
pub use alloc::collections::{BTreeMap, BTreeSet, BinaryHeap, LinkedList, VecDeque};
pub use alloc::collections::btree_map;
pub use alloc::collections::btree_set;
