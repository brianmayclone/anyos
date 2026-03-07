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

use crate::instruction::DecodedInst;
use crate::jit::block::BlockKey;
use crate::jit::executable_mem::JitBuffer;
use crate::jit::translator::Translator;

/// Compiled block entry in the JIT code cache.
struct CompiledEntry {
    code_offset: usize,
    #[allow(dead_code)]
    code_len: usize,
    guest_instruction_count: u64,
}

/// The JIT engine: coordinates translation and compiled code execution.
pub struct JitEngine {
    buffer: JitBuffer,
    translator: Translator,
    compiled: alloc::collections::BTreeMap<BlockKey, CompiledEntry>,
    /// Pages that contain compiled JIT blocks (fast SMC reject).
    jit_code_pages: alloc::collections::BTreeSet<u64>,
    enabled: bool,
    blocks_compiled: u64,
}

impl JitEngine {
    pub fn new() -> Self {
        JitEngine {
            buffer: JitBuffer::new(),
            translator: Translator::new(),
            compiled: alloc::collections::BTreeMap::new(),
            jit_code_pages: alloc::collections::BTreeSet::new(),
            enabled: false,
            blocks_compiled: 0,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Look up a compiled block. Returns (code_offset, guest_instruction_count).
    pub fn lookup_compiled(&self, key: &BlockKey) -> Option<(usize, u64)> {
        self.compiled.get(key).map(|e| (e.code_offset, e.guest_instruction_count))
    }

    /// Compile a basic block. `inst_ptrs` are raw pointers to DecodedInst
    /// objects in the decode cache — these get embedded into generated code
    /// for helper_execute_one calls.
    pub fn compile_block(
        &mut self,
        key: BlockKey,
        block: &crate::jit::block::BasicBlock,
        inst_ptrs: &[*const DecodedInst],
    ) -> Option<(usize, u64)> {
        let compiled = self.translator.translate_block(block, inst_ptrs, key.phys_addr, key.mode);
        let code_offset = self.buffer.emit(&compiled.code)?;

        let entry = CompiledEntry {
            code_offset,
            code_len: compiled.code.len(),
            guest_instruction_count: compiled.guest_instruction_count,
        };

        self.blocks_compiled += 1;
        let inst_count = entry.guest_instruction_count;
        let page = key.phys_addr & !0xFFF;
        self.jit_code_pages.insert(page);
        crate::memory::smc::mark_code_page(page);
        self.compiled.insert(key, entry);
        Some((code_offset, inst_count))
    }

    pub unsafe fn code_ptr(&self, offset: usize) -> *const u8 {
        self.buffer.code_ptr(offset)
    }

    pub fn make_executable(&mut self) {
        self.buffer.make_executable();
    }

    pub fn make_writable(&mut self) {
        self.buffer.make_writable();
    }

    pub fn flush(&mut self) {
        self.compiled.clear();
        self.jit_code_pages.clear();
        self.buffer.reset();
    }

    pub fn invalidate_page(&mut self, page_phys: u64) {
        let page_base = page_phys & !0xFFF;
        if !self.jit_code_pages.contains(&page_base) {
            return;
        }
        let page_end = page_base.saturating_add(0x1000);
        self.compiled.retain(|key, _| {
            key.phys_addr < page_base || key.phys_addr >= page_end
        });
        let still_has = self.compiled.keys().any(|key| {
            key.phys_addr >= page_base && key.phys_addr < page_end
        });
        if !still_has {
            self.jit_code_pages.remove(&page_base);
        }
    }

    pub fn blocks_compiled(&self) -> u64 {
        self.blocks_compiled
    }

    pub fn compiled_count(&self) -> usize {
        self.compiled.len()
    }

    pub fn code_buffer_used(&self) -> usize {
        self.buffer.used()
    }

    pub fn code_buffer_capacity(&self) -> usize {
        self.buffer.capacity()
    }

    pub fn native_count(&self) -> u64 {
        self.translator.native_count
    }

    pub fn fallback_count(&self) -> u64 {
        self.translator.helper_count
    }

    /// Return the top N opcodes that fell back to helper_execute_one,
    /// sorted by frequency (highest first).
    /// Each entry: (opcode_key, count) where key = map<<8 | opcode.
    pub fn top_helper_opcodes(&self, n: usize) -> alloc::vec::Vec<(u16, u64)> {
        let mut entries: alloc::vec::Vec<(u16, u64)> = self.translator
            .helper_opcode_counts
            .iter()
            .map(|(&k, &v)| (k, v))
            .collect();
        entries.sort_by(|a, b| b.1.cmp(&a.1));
        entries.truncate(n);
        entries
    }
}
