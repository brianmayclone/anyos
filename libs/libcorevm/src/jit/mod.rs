//! JIT acceleration for x86 instruction execution.
//!
//! This module implements a multi-phase dynamic binary translation (DBT)
//! pipeline for same-ISA JIT compilation: guest x86 → host x86-64.
//!
//! ## Phase 1 (complete): Decode Cache
//! Basic blocks are decoded once and cached in a [`DecodeCache`], eliminating
//! redundant per-instruction decoding overhead for hot loops.
//!
//! ## Phase 2 (complete): JIT Infrastructure
//! - [`emitter`] — x86-64 machine code assembler
//! - [`executable_mem`] — W^X memory management for compiled code
//! - [`helpers`] — Runtime helper functions callable from JIT code (C ABI)
//! - [`translator`] — Guest→Host instruction translation
//!
//! ## Phase 3 (current): Core Instruction Translation
//! Native translation for Tier 1 instructions (MOV, ADD, SUB, CMP, JMP,
//! Jcc, TEST, INC/DEC, NOP) with interpreter fallback for the rest.

// ── Sub-modules ──────────────────────────────────────────────────────────

/// Basic block detection and representation.
pub mod block;
/// Decode cache for pre-decoded basic blocks.
pub mod cache;
/// x86-64 machine code emitter.
pub mod emitter;
/// W^X executable memory management for JIT-compiled code.
pub mod executable_mem;
/// Runtime helper functions callable from JIT-compiled code (C ABI).
pub mod helpers;
/// Guest→Host instruction translator.
pub mod translator;

// ── JitEngine ────────────────────────────────────────────────────────────

use crate::jit::block::BlockKey;
use crate::jit::executable_mem::JitBuffer;
use crate::jit::translator::Translator;

/// Compiled block entry in the JIT code cache.
///
/// Maps a [`BlockKey`] to the compiled native code stored in the
/// [`JitBuffer`].
struct CompiledEntry {
    /// Byte offset within the JitBuffer where the native code starts.
    code_offset: usize,
    /// Length of the native code in bytes.
    #[allow(dead_code)]
    code_len: usize,
    /// Number of guest instructions in this block.
    guest_instruction_count: u64,
}

/// The JIT engine: coordinates decode caching, translation, and compiled
/// code execution.
///
/// Owns the executable memory buffer and the compiled-block lookup table.
/// The decode cache lives on the [`Cpu`] struct (shared with the
/// interpreter fast path).
pub struct JitEngine {
    /// Buffer holding all compiled native code.
    buffer: JitBuffer,
    /// Translator that converts decoded blocks to native code.
    translator: Translator,
    /// Map from block key to compiled code location.
    compiled: alloc::collections::BTreeMap<BlockKey, CompiledEntry>,
    /// Blocks that currently produce no native instructions.
    /// These are better executed directly by the interpreter path.
    no_native: alloc::collections::BTreeSet<BlockKey>,
    /// Whether the JIT engine is enabled.
    enabled: bool,
    /// Total number of blocks compiled.
    blocks_compiled: u64,
}

impl JitEngine {
    /// Minimum native-translation ratio (percent) required to keep a block in JIT.
    const MIN_NATIVE_RATIO_PCT: u64 = 70;

    /// Create a new JIT engine (disabled by default).
    pub fn new() -> Self {
        JitEngine {
            buffer: JitBuffer::new(),
            translator: Translator::new(),
            compiled: alloc::collections::BTreeMap::new(),
            no_native: alloc::collections::BTreeSet::new(),
            enabled: false,
            blocks_compiled: 0,
        }
    }

    /// Enable or disable the JIT engine.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Whether the JIT engine is currently enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Look up a compiled block by its key.
    ///
    /// Returns the function pointer and guest instruction count if found.
    pub fn lookup_compiled(&self, key: &BlockKey) -> Option<(usize, u64)> {
        self.compiled.get(key).map(|entry| {
            (entry.code_offset, entry.guest_instruction_count)
        })
    }

    /// Whether this block key should skip JIT compilation/execution.
    pub fn should_skip_compile(&self, key: &BlockKey) -> bool {
        self.no_native.contains(key)
    }

    /// Compile a decoded basic block and store it in the code buffer.
    ///
    /// Returns the code offset and guest instruction count on success,
    /// or `None` if the buffer is full.
    pub fn compile_block(
        &mut self,
        key: BlockKey,
        block: &crate::jit::block::BasicBlock,
    ) -> Option<(usize, u64)> {
        let compiled = self.translator.translate_block(block, key.phys_addr, key.mode);
        if compiled.native_instruction_count == 0 {
            self.no_native.insert(key);
            return None;
        }
        let total = compiled.guest_instruction_count.max(1);
        let native = compiled.native_instruction_count;
        if native.saturating_mul(100) < total.saturating_mul(Self::MIN_NATIVE_RATIO_PCT) {
            self.no_native.insert(key);
            return None;
        }
        self.no_native.remove(&key);

        let code_offset = self.buffer.emit(&compiled.code)?;

        let entry = CompiledEntry {
            code_offset,
            code_len: compiled.code.len(),
            guest_instruction_count: compiled.guest_instruction_count,
        };

        self.blocks_compiled += 1;
        let inst_count = entry.guest_instruction_count;
        self.compiled.insert(key, entry);

        Some((code_offset, inst_count))
    }

    /// Get a raw pointer to compiled code at the given offset.
    ///
    /// # Safety
    ///
    /// The returned pointer must only be called when the buffer is in
    /// executable mode and the offset is valid.
    pub unsafe fn code_ptr(&self, offset: usize) -> *const u8 {
        self.buffer.code_ptr(offset)
    }

    /// Mark the code buffer as executable (W→X transition).
    pub fn make_executable(&mut self) {
        self.buffer.make_executable();
    }

    /// Mark the code buffer as writable (X→W transition).
    pub fn make_writable(&mut self) {
        self.buffer.make_writable();
    }

    /// Flush all compiled blocks (e.g., on CR3 change or mode switch).
    pub fn flush(&mut self) {
        self.compiled.clear();
        self.no_native.clear();
        self.buffer.reset();
    }

    /// Invalidate all compiled blocks whose physical address falls within
    /// the given 4 KiB page. Called on self-modifying code detection.
    ///
    /// Because compiled code is stored in a flat linear buffer (no per-block
    /// free list), we cannot reclaim the buffer space for individual blocks.
    /// We simply remove the lookup entries — on next execution the block will
    /// be recompiled into the buffer. If the buffer fills up, `compile_block`
    /// returns `None` and the interpreter fallback is used instead.
    pub fn invalidate_page(&mut self, page_phys: u64) {
        let page_base = page_phys & !0xFFF;
        let page_end = page_base.saturating_add(0x1000);
        self.compiled.retain(|key, _| {
            key.phys_addr < page_base || key.phys_addr >= page_end
        });
        self.no_native.retain(|key| {
            key.phys_addr < page_base || key.phys_addr >= page_end
        });
    }

    /// Return the number of compiled blocks.
    pub fn blocks_compiled(&self) -> u64 {
        self.blocks_compiled
    }

    /// Return the number of currently cached compiled blocks.
    pub fn compiled_count(&self) -> usize {
        self.compiled.len()
    }

    /// Return the number of bytes used in the code buffer.
    pub fn code_buffer_used(&self) -> usize {
        self.buffer.used()
    }

    /// Return the total capacity of the code buffer.
    pub fn code_buffer_capacity(&self) -> usize {
        self.buffer.capacity()
    }

    /// Return the number of instructions translated natively.
    pub fn native_count(&self) -> u64 {
        self.translator.native_count
    }

    /// Return the number of instructions that fell back to interpreter.
    pub fn fallback_count(&self) -> u64 {
        self.translator.fallback_count
    }
}
