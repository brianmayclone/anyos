//! Pure-Rust Git library for anyOS.
//!
//! Implements core Git functionality:
//! - Object storage (blob, tree, commit, tag) with loose and pack formats
//! - SHA-1 hashing (via libtls)
//! - Zlib inflate/deflate (from libzip)
//! - Index file (.git/index v2) parsing and writing
//! - Ref management (branches, HEAD, tags, remotes)
//! - Diff computation (LCS-based unified diff)
//! - Pack file v2 format (parsing, writing, delta application)
//! - Smart HTTP transport (clone, fetch, push)
//! - Remote management
//! - Checkout (tree → working directory)
//! - Fast-forward merge
//! - .gitignore pattern matching
//! - Git config parsing

#![no_std]

extern crate alloc;
extern crate std;

pub mod sha1;
pub mod inflate;
pub mod deflate;
pub mod object;
pub mod oid;
pub mod repo;
pub mod index;
pub mod refs;
pub mod diff;
pub mod tree;
pub mod config;
pub mod pack;
pub mod stream;
pub mod transport;
pub mod remote;
pub mod checkout;
pub mod ignore;
pub mod merge;

pub use oid::Oid;
pub use object::{Object, ObjectType, Commit};
pub use repo::Repository;
pub use index::Index;
