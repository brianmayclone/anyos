//! JIT acceleration for x86 instruction execution (v2).
//!
//! Every instruction is translated to native code — either directly or via
//! helper_execute_one. No interpreter fallback, no ratio threshold.

pub mod block;
pub mod cache;
pub mod emitter;
pub mod executable_mem;
pub mod helpers;
pub mod translator;
pub mod lookup_table;
pub mod session;

