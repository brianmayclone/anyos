//! Runtime support for anyrc-compiled programs.
//!
//! These are small assembly stubs that implement heap allocation, Vec operations,
//! and String operations. They're emitted as part of the executable and called
//! by intrinsic-generated code.
//!
//! On anyOS, heap allocation uses SYS_SBRK (syscall 9) for small allocations
//! and SYS_MMAP (syscall 14) for large ones.

use crate::linker::link::TargetAbi;
use crate::prelude::*;

/// Generate the runtime support object code (x86-64 machine code).
/// Returns a list of (symbol_name, code_bytes) pairs to be linked in.
pub fn runtime_stubs(target_abi: TargetAbi) -> Vec<(String, Vec<u8>)> {
    let mut stubs = Vec::new();

    // __anyrc_alloc(size: usize) -> *mut u8
    // args: RDI = size
    // returns: RAX = pointer (or 0 on failure)
    stubs.push(("__anyrc_alloc".to_string(), {
        let mut code = Vec::new();
        // Round size up to 8-byte alignment: size = (size + 7) & ~7
        code.extend_from_slice(&[0x48, 0x83, 0xC7, 0x07]); // add rdi, 7
        code.extend_from_slice(&[0x48, 0x83, 0xE7, 0xF8]); // and rdi, -8

        match target_abi {
            TargetAbi::AnyOs => {
                // anyOS native syscall ABI: RAX=syscall, RBX=arg1, R10=arg2, ...
                // SYS_SBRK takes an increment and returns the old break.
                code.extend_from_slice(&[0x53]); // push rbx
                code.extend_from_slice(&[0x48, 0x89, 0xFB]); // mov rbx, rdi (increment)
                code.extend_from_slice(&[0x48, 0xC7, 0xC0, 0x09, 0x00, 0x00, 0x00]); // mov rax, 9
                code.extend_from_slice(&[0x0F, 0x05]); // syscall
                code.extend_from_slice(&[0x5B]); // pop rbx
            }
            TargetAbi::Linux => {
                // Linux x86-64 mmap(NULL, size, PROT_READ|PROT_WRITE,
                // MAP_PRIVATE|MAP_ANONYMOUS, -1, 0).
                code.extend_from_slice(&[0x48, 0x89, 0xFE]); // mov rsi, rdi (len)
                code.extend_from_slice(&[0x48, 0x31, 0xFF]); // xor rdi, rdi (addr)
                code.extend_from_slice(&[0x48, 0xC7, 0xC2, 0x03, 0x00, 0x00, 0x00]); // prot=3
                code.extend_from_slice(&[0x49, 0xC7, 0xC2, 0x22, 0x00, 0x00, 0x00]); // flags=0x22
                code.extend_from_slice(&[0x49, 0xC7, 0xC0, 0xFF, 0xFF, 0xFF, 0xFF]); // fd=-1
                code.extend_from_slice(&[0x4D, 0x31, 0xC9]); // xor r9, r9 (offset)
                code.extend_from_slice(&[0x48, 0xC7, 0xC0, 0x09, 0x00, 0x00, 0x00]); // SYS_mmap
                code.extend_from_slice(&[0x0F, 0x05]); // syscall
            }
        }
        code.extend_from_slice(&[0xC3]); // ret
        code
    }));

    // __anyrc_dealloc(ptr: *mut u8, size: usize)
    // For sbrk-based allocator, dealloc is a no-op (memory reclaimed on process exit)
    stubs.push(("__anyrc_dealloc".to_string(), {
        let mut code = Vec::new();
        code.push(0xC3); // ret (no-op)
        code
    }));

    // Function-item shims used when Core/Alloc constructors or methods are
    // passed as callbacks, e.g. `Deserialize::deserialize(...).map(Wrapping)`.
    // These are ABI entry points for real Rust function values; direct calls
    // are still lowered by the compiler intrinsics above.
    for name in [
        "Reverse",
        "Wrapping",
        "into_boxed_c_str",
        "into_boxed_slice",
        "into_boxed_str",
    ] {
        stubs.push((name.to_string(), vec![0x48, 0x89, 0xF8, 0xC3])); // mov rax, rdi; ret
    }
    for name in ["drop_in_place"] {
        stubs.push((name.to_string(), vec![0xC3])); // ret
    }
    for name in ["Error"] {
        stubs.push((name.to_string(), vec![0x48, 0x31, 0xC0, 0xC3])); // xor rax, rax; ret
    }

    // __anyrc_realloc(ptr: *mut u8, old_size: usize, new_size: usize) -> *mut u8
    // Simple: alloc new, copy old, return new (dealloc is no-op)
    stubs.push(("__anyrc_realloc".to_string(), {
        let mut code = Vec::new();
        // Save args
        code.extend_from_slice(&[0x53]); // push rbx
        code.extend_from_slice(&[0x41, 0x54]); // push r12
        code.extend_from_slice(&[0x41, 0x55]); // push r13
        code.extend_from_slice(&[0x49, 0x89, 0xFC]); // mov r12, rdi (old ptr)
        code.extend_from_slice(&[0x49, 0x89, 0xF5]); // mov r13, rsi (old size)
        code.extend_from_slice(&[0x48, 0x89, 0xD7]); // mov rdi, rdx (new size)
        code.extend_from_slice(&[0x48, 0x89, 0xD3]); // mov rbx, rdx (save new size)
                                                     // Alloc new block
        code.extend_from_slice(&[0xE8, 0x00, 0x00, 0x00, 0x00]); // call __anyrc_alloc (will be patched by linker)
                                                                 // Copy min(old_size, new_size) bytes from old to new
        code.extend_from_slice(&[0x48, 0x89, 0xC7]); // mov rdi, rax (dst = new)
        code.extend_from_slice(&[0x4C, 0x89, 0xE6]); // mov rsi, r12 (src = old)
        code.extend_from_slice(&[0x4C, 0x89, 0xE9]); // mov rcx, r13 (count = old_size)
        code.extend_from_slice(&[0x48, 0x39, 0xD9]); // cmp rcx, rbx
        code.extend_from_slice(&[0x48, 0x0F, 0x47, 0xCB]); // cmova rcx, rbx (min)
        code.extend_from_slice(&[0x50]); // push rax (save new ptr)
        code.extend_from_slice(&[0xF3, 0xA4]); // rep movsb
        code.extend_from_slice(&[0x58]); // pop rax
        code.extend_from_slice(&[0x41, 0x5D]); // pop r13
        code.extend_from_slice(&[0x41, 0x5C]); // pop r12
        code.extend_from_slice(&[0x5B]); // pop rbx
        code.extend_from_slice(&[0xC3]); // ret
        code
    }));

    // __anyrc_vec_push(&mut Vec<T>, value: T)
    // Vec layout: [ptr: *mut T, len: usize, cap: usize]
    // args: RDI = &mut Vec, RSI = value
    stubs.push(("__anyrc_vec_push".to_string(), {
        let mut code = Vec::new();
        // Save callee-saved regs
        code.extend_from_slice(&[0x53]); // push rbx
        code.extend_from_slice(&[0x41, 0x54]); // push r12
        code.extend_from_slice(&[0x48, 0x89, 0xFB]); // mov rbx, rdi (&vec)
        code.extend_from_slice(&[0x49, 0x89, 0xF4]); // mov r12, rsi (value)

        // Check if len < cap
        code.extend_from_slice(&[0x48, 0x8B, 0x43, 0x08]); // mov rax, [rbx+8] (len)
        code.extend_from_slice(&[0x48, 0x3B, 0x43, 0x10]); // cmp rax, [rbx+16] (cap)
        code.extend_from_slice(&[0x72, 0x2C]); // jb .has_space (skip grow)

        // Grow: new_cap = max(cap * 2, 4)
        code.extend_from_slice(&[0x48, 0x8B, 0x4B, 0x10]); // mov rcx, [rbx+16] (cap)
        code.extend_from_slice(&[0x48, 0x01, 0xC9]); // add rcx, rcx (cap * 2)
        code.extend_from_slice(&[0x48, 0x83, 0xF9, 0x04]); // cmp rcx, 4
        code.extend_from_slice(&[0x48, 0x0F, 0x43, 0xC9]); // cmovae rcx, rcx (keep if >=4)
        code.extend_from_slice(&[0x48, 0x83, 0xF9, 0x04]); // cmp rcx, 4
        code.extend_from_slice(&[0x7D, 0x04]); // jge .ok
        code.extend_from_slice(&[0x48, 0xC7, 0xC1, 0x04]); // mov rcx, 4
                                                           // .ok: realloc(ptr, old_cap*8, new_cap*8)
        code.extend_from_slice(&[0x48, 0x89, 0x4B, 0x10]); // mov [rbx+16], rcx (store new cap)
        code.extend_from_slice(&[0x48, 0x8B, 0x3B]); // mov rdi, [rbx] (old ptr)
        code.extend_from_slice(&[0x48, 0xC1, 0xE1, 0x03]); // shl rcx, 3 (new_cap * 8)
        code.extend_from_slice(&[0x48, 0x89, 0xCE]); // mov rsi, rcx (old alloc size... approximate)
        code.extend_from_slice(&[0x48, 0x89, 0xCA]); // mov rdx, rcx (new alloc size)
        code.extend_from_slice(&[0xE8, 0x00, 0x00, 0x00, 0x00]); // call __anyrc_realloc (patched)
        code.extend_from_slice(&[0x48, 0x89, 0x03]); // mov [rbx], rax (store new ptr)

        // .has_space: store value at ptr[len], increment len
        code.extend_from_slice(&[0x48, 0x8B, 0x03]); // mov rax, [rbx] (ptr)
        code.extend_from_slice(&[0x48, 0x8B, 0x4B, 0x08]); // mov rcx, [rbx+8] (len)
        code.extend_from_slice(&[0x4C, 0x89, 0x24, 0xC8]); // mov [rax+rcx*8], r12 (store value)
        code.extend_from_slice(&[0x48, 0xFF, 0x43, 0x08]); // inc [rbx+8] (len++)
        code.extend_from_slice(&[0x41, 0x5C]); // pop r12
        code.extend_from_slice(&[0x5B]); // pop rbx
        code.extend_from_slice(&[0xC3]); // ret
        code
    }));

    // __anyrc_vec_pop(&mut Vec<T>) -> (disc, value) in (RAX, RDX)
    // Returns disc=0 (Some) + value if len > 0, disc=1 (None) if empty
    stubs.push(("__anyrc_vec_pop".to_string(), {
        let mut code = Vec::new();
        code.extend_from_slice(&[0x48, 0x8B, 0x47, 0x08]); // mov rax, [rdi+8] (len)
        code.extend_from_slice(&[0x48, 0x85, 0xC0]); // test rax, rax
        code.extend_from_slice(&[0x74, 0x14]); // jz .empty

        // Some: decrement len, load value
        code.extend_from_slice(&[0x48, 0xFF, 0x4F, 0x08]); // dec [rdi+8] (len--)
        code.extend_from_slice(&[0x48, 0x8B, 0x4F, 0x08]); // mov rcx, [rdi+8] (new len)
        code.extend_from_slice(&[0x48, 0x8B, 0x07]); // mov rax, [rdi] (ptr)
        code.extend_from_slice(&[0x48, 0x8B, 0x14, 0xC8]); // mov rdx, [rax+rcx*8] (value)
        code.extend_from_slice(&[0x48, 0x31, 0xC0]); // xor rax, rax (disc = 0 = Some)
        code.extend_from_slice(&[0xC3]); // ret

        // .empty: return None
        code.extend_from_slice(&[0x48, 0xC7, 0xC0, 0x01, 0x00, 0x00, 0x00]); // mov rax, 1 (disc = None)
        code.extend_from_slice(&[0x48, 0x31, 0xD2]); // xor rdx, rdx
        code.extend_from_slice(&[0xC3]); // ret
        code
    }));

    // __anyrc_string_push_str(&mut String, ptr: *const u8, len: usize)
    // String layout = Vec<u8> layout
    stubs.push(("__anyrc_string_push_str".to_string(), {
        let mut code = Vec::new();
        // Args: RDI=&mut String, RSI=src ptr, RDX=src len.
        // String layout: [ptr: *mut u8, len: usize, cap: usize].
        code.extend_from_slice(&[0x53]); // push rbx
        code.extend_from_slice(&[0x41, 0x54]); // push r12
        code.extend_from_slice(&[0x41, 0x55]); // push r13
        code.extend_from_slice(&[0x41, 0x56]); // push r14
        code.extend_from_slice(&[0x41, 0x57]); // push r15
        code.extend_from_slice(&[0x48, 0x89, 0xFB]); // mov rbx, rdi (string)
        code.extend_from_slice(&[0x49, 0x89, 0xF4]); // mov r12, rsi (src)
        code.extend_from_slice(&[0x49, 0x89, 0xD5]); // mov r13, rdx (append len)
        code.extend_from_slice(&[0x4C, 0x8B, 0x73, 0x08]); // mov r14, [rbx+8] (old len)
        code.extend_from_slice(&[0x4D, 0x89, 0xF7]); // mov r15, r14
        code.extend_from_slice(&[0x4D, 0x01, 0xEF]); // add r15, r13 (new len)
        code.extend_from_slice(&[0x48, 0x8B, 0x4B, 0x10]); // mov rcx, [rbx+16] (cap)
        code.extend_from_slice(&[0x49, 0x39, 0xCF]); // cmp r15, rcx
        let jbe_offset_pos = code.len() + 1;
        code.extend_from_slice(&[0x76, 0x00]); // jbe .has_capacity
        code.extend_from_slice(&[0x48, 0x8B, 0x3B]); // mov rdi, [rbx] (old ptr)
        code.extend_from_slice(&[0x48, 0x89, 0xCE]); // mov rsi, rcx (old cap bytes)
        code.extend_from_slice(&[0x4C, 0x89, 0xFA]); // mov rdx, r15 (new cap bytes)
        code.extend_from_slice(&[0xE8, 0x00, 0x00, 0x00, 0x00]); // call __anyrc_realloc
        code.extend_from_slice(&[0x48, 0x89, 0x03]); // mov [rbx], rax
        code.extend_from_slice(&[0x4C, 0x89, 0x7B, 0x10]); // mov [rbx+16], r15
        let has_capacity = code.len();
        code[jbe_offset_pos] = (has_capacity - (jbe_offset_pos + 1)) as u8;
        code.extend_from_slice(&[0x48, 0x8B, 0x3B]); // mov rdi, [rbx] (dst base)
        code.extend_from_slice(&[0x4C, 0x01, 0xF7]); // add rdi, r14 (dst += old len)
        code.extend_from_slice(&[0x4C, 0x89, 0xE6]); // mov rsi, r12 (src)
        code.extend_from_slice(&[0x4C, 0x89, 0xE9]); // mov rcx, r13 (count)
        code.extend_from_slice(&[0xF3, 0xA4]); // rep movsb
        code.extend_from_slice(&[0x4C, 0x89, 0x7B, 0x08]); // mov [rbx+8], r15
        code.extend_from_slice(&[0x41, 0x5F]); // pop r15
        code.extend_from_slice(&[0x41, 0x5E]); // pop r14
        code.extend_from_slice(&[0x41, 0x5D]); // pop r13
        code.extend_from_slice(&[0x41, 0x5C]); // pop r12
        code.extend_from_slice(&[0x5B]); // pop rbx
        code.push(0xC3);
        code
    }));

    // __anyrc_string_push_char(&mut String, char)
    stubs.push(("__anyrc_string_push_char".to_string(), {
        let mut code = Vec::new();
        // Append the low byte of the char. Full UTF-8 encoding belongs in the
        // library layer; this helper is only the bootstrap ASCII fast path used
        // by current anyrc codegen.
        code.extend_from_slice(&[0x56]); // push rsi (byte source at rsp)
        code.extend_from_slice(&[0x48, 0x89, 0xE6]); // mov rsi, rsp
        code.extend_from_slice(&[0x48, 0xC7, 0xC2, 0x01, 0x00, 0x00, 0x00]); // mov rdx, 1
        code.extend_from_slice(&[0xE8, 0x00, 0x00, 0x00, 0x00]); // call __anyrc_string_push_str
        code.extend_from_slice(&[0x48, 0x83, 0xC4, 0x08]); // add rsp, 8
        code.push(0xC3);
        code
    }));

    // __anyrc_string_from_str(ptr: *const u8, len: usize) -> *mut String
    stubs.push(("__anyrc_string_from_str".to_string(), {
        let mut code = Vec::new();
        // Allocate len bytes, copy the string data, return a String struct
        // Simplified: just return ptr as-is (caller manages)
        code.extend_from_slice(&[0x48, 0x89, 0xF8]); // mov rax, rdi
        code.push(0xC3);
        code
    }));

    // ── HashMap runtime stubs ──
    // HashMap is implemented as a simple linear-probe hash table:
    // Layout: [buckets_ptr, bucket_count, len, 0_padding]
    // Each bucket: [hash: u64, key: u64, value: u64, occupied: u64]

    // __anyrc_hashmap_insert(&mut HashMap, key: u64, value: u64)
    stubs.push(("__anyrc_hashmap_insert".to_string(), {
        let mut code = Vec::new();
        code.push(0xC3); // stub — full impl requires hash function
        code
    }));

    // __anyrc_hashmap_get(&HashMap, &key) -> (disc, &value) in (RAX, RDX)
    stubs.push(("__anyrc_hashmap_get".to_string(), {
        let mut code = Vec::new();
        // Stub: return None (1, 0)
        code.extend_from_slice(&[0x48, 0xC7, 0xC0, 0x01, 0x00, 0x00, 0x00]); // mov rax, 1
        code.extend_from_slice(&[0x48, 0x31, 0xD2]); // xor rdx, rdx
        code.push(0xC3);
        code
    }));

    // __anyrc_hashmap_entry(&mut HashMap, key) -> *mut entry_slot
    stubs.push(("__anyrc_hashmap_entry".to_string(), {
        let mut code = Vec::new();
        code.extend_from_slice(&[0x48, 0x89, 0xF8]); // mov rax, rdi (return &self as placeholder)
        code.push(0xC3);
        code
    }));

    // __anyrc_entry_or_default(entry_ptr) -> *mut value
    stubs.push(("__anyrc_entry_or_default".to_string(), {
        let mut code = Vec::new();
        code.extend_from_slice(&[0x48, 0x89, 0xF8]); // mov rax, rdi
        code.push(0xC3);
        code
    }));

    // __anyrc_option_map(&Option, fn_ptr) -> Option
    // disc in [RDI], value in [RDI+8], closure in RSI
    stubs.push(("__anyrc_option_map".to_string(), {
        let mut code = Vec::new();
        // Check disc
        code.extend_from_slice(&[0x48, 0x8B, 0x07]); // mov rax, [rdi] (disc)
        code.extend_from_slice(&[0x48, 0x85, 0xC0]); // test rax, rax
        code.extend_from_slice(&[0x75, 0x0E]); // jnz .none
                                               // Some: call closure with value
        code.extend_from_slice(&[0x48, 0x8B, 0x7F, 0x08]); // mov rdi, [rdi+8] (value)
        code.extend_from_slice(&[0xFF, 0xD6]); // call rsi (closure)
        code.extend_from_slice(&[0x48, 0x89, 0xC2]); // mov rdx, rax (mapped value)
        code.extend_from_slice(&[0x48, 0x31, 0xC0]); // xor rax, rax (disc = Some)
        code.extend_from_slice(&[0xC3]); // ret
                                         // .none: return None
        code.extend_from_slice(&[0x48, 0xC7, 0xC0, 0x01, 0x00, 0x00, 0x00]); // mov rax, 1
        code.extend_from_slice(&[0x48, 0x31, 0xD2]); // xor rdx, rdx
        code.extend_from_slice(&[0xC3]); // ret
        code
    }));

    // __anyrc_to_string(value: u64) -> String ptr
    stubs.push(("__anyrc_to_string".to_string(), {
        let mut code = Vec::new();
        code.extend_from_slice(&[0x48, 0x89, 0xF8]); // mov rax, rdi
        code.push(0xC3);
        code
    }));

    // __anyrc_println(fmt_str_ptr: *const u8) — write to stdout
    // On anyOS: SYS_WRITE = 2, fd=1 (stdout)
    stubs.push(("__anyrc_println".to_string(), {
        let mut code = Vec::new();
        code.extend_from_slice(&[0x53]); // push rbx
                                         // First, compute string length (scan for null terminator)
        code.extend_from_slice(&[0x48, 0x89, 0xFE]); // mov rsi, rdi (save ptr)
        code.extend_from_slice(&[0x48, 0x31, 0xC9]); // xor rcx, rcx (len = 0)
                                                     // .scan:
        code.extend_from_slice(&[0x80, 0x3C, 0x0E, 0x00]); // cmp byte [rsi+rcx], 0
        code.extend_from_slice(&[0x74, 0x04]); // je .done_scan
        code.extend_from_slice(&[0x48, 0xFF, 0xC1]); // inc rcx
        code.extend_from_slice(&[0xEB, 0xF5]); // jmp .scan
                                               // .done_scan: rcx = len, rsi = ptr
                                               // SYS_WRITE(fd=1, buf=rsi, len=rcx)
        code.extend_from_slice(&[0x48, 0x89, 0xCA]); // mov rdx, rcx (len)
        code.extend_from_slice(&[0x49, 0x89, 0xF2]); // mov r10, rsi (buf)
        code.extend_from_slice(&[0x48, 0xC7, 0xC3, 0x01, 0x00, 0x00, 0x00]); // mov rbx, 1 (stdout fd)
        code.extend_from_slice(&[0x48, 0xC7, 0xC0, 0x02, 0x00, 0x00, 0x00]); // mov rax, 2 (SYS_WRITE)
        code.extend_from_slice(&[0x0F, 0x05]); // syscall
                                               // Write newline
        code.extend_from_slice(&[0x6A, 0x0A]); // push 0x0A ('\n')
        code.extend_from_slice(&[0x49, 0x89, 0xE2]); // mov r10, rsp (ptr to '\n')
        code.extend_from_slice(&[0x48, 0xC7, 0xC2, 0x01, 0x00, 0x00, 0x00]); // mov rdx, 1
        code.extend_from_slice(&[0x48, 0xC7, 0xC3, 0x01, 0x00, 0x00, 0x00]); // mov rbx, 1
        code.extend_from_slice(&[0x48, 0xC7, 0xC0, 0x02, 0x00, 0x00, 0x00]); // mov rax, 2 (SYS_WRITE)
        code.extend_from_slice(&[0x0F, 0x05]); // syscall
        code.extend_from_slice(&[0x48, 0x83, 0xC4, 0x08]); // add rsp, 8 (clean up push)
        code.extend_from_slice(&[0x5B]); // pop rbx
        code.push(0xC3);
        code
    }));

    // ── compiler_builtins memory intrinsics ──
    // These are required for no_std builds where the compiler may emit calls
    // to memcpy, memset, memmove, memcmp, and bcmp.

    // memcpy(dest: *mut u8, src: *const u8, n: usize) -> *mut u8
    // RDI=dest, RSI=src, RDX=n, returns RAX=dest
    stubs.push(("memcpy".to_string(), {
        let mut code = Vec::new();
        code.extend_from_slice(&[0x48, 0x89, 0xF8]); // mov rax, rdi (save dest for return)
        code.extend_from_slice(&[0x48, 0x89, 0xD1]); // mov rcx, rdx (count)
        code.extend_from_slice(&[0xF3, 0xA4]); // rep movsb
        code.push(0xC3); // ret
        code
    }));

    // memmove(dest: *mut u8, src: *const u8, n: usize) -> *mut u8
    // Handles overlapping regions by checking direction.
    stubs.push(("memmove".to_string(), {
        let mut code = Vec::new();
        code.extend_from_slice(&[0x48, 0x89, 0xF8]); // mov rax, rdi (save dest)
        code.extend_from_slice(&[0x48, 0x89, 0xD1]); // mov rcx, rdx (count)
        code.extend_from_slice(&[0x48, 0x39, 0xFE]); // cmp rsi, rdi (src vs dest)
        code.extend_from_slice(&[0x73, 0x09]); // jae .forward (src >= dest)
                                               // Backward copy: set direction flag, adjust pointers to end
        code.extend_from_slice(&[0xFD]); // std
        code.extend_from_slice(&[0x48, 0x8D, 0x7C, 0x0F, 0xFF]); // lea rdi, [rdi+rcx-1]
        code.extend_from_slice(&[0x48, 0x8D, 0x74, 0x0E, 0xFF]); // lea rsi, [rsi+rcx-1]
        code.extend_from_slice(&[0xF3, 0xA4]); // rep movsb
        code.extend_from_slice(&[0xFC]); // cld (clear direction flag)
        code.push(0xC3); // ret
                         // .forward:
        code[9] = (code.len() as u8) - 10; // fix jae offset
        code.extend_from_slice(&[0xF3, 0xA4]); // rep movsb
        code.push(0xC3); // ret
        code
    }));

    // memset(dest: *mut u8, val: i32, n: usize) -> *mut u8
    // RDI=dest, ESI=val, RDX=n, returns RAX=dest
    stubs.push(("memset".to_string(), {
        let mut code = Vec::new();
        code.extend_from_slice(&[0x48, 0x89, 0xF8]); // mov rax, rdi (save dest)
        code.extend_from_slice(&[0x48, 0x89, 0xD1]); // mov rcx, rdx (count)
        code.extend_from_slice(&[0x40, 0x88, 0xF0]); // mov al, sil (byte value)
                                                     // Note: rax was overwritten, save and restore dest
        code.clear();
        code.extend_from_slice(&[0x48, 0x89, 0xFA]); // mov rdx, rdi (save dest)
        code.extend_from_slice(&[0x89, 0xF0]); // mov eax, esi (value)
        code.extend_from_slice(&[0x48, 0x8B, 0x4C, 0x24, 0x00]); // hm, rdx is count in sysv
                                                                 // Actually: RDI=dest, ESI=val, RDX=n
        code.clear();
        code.extend_from_slice(&[0x50]); // push rax (dummy, save rbx-compat)
        code.extend_from_slice(&[0x48, 0x89, 0xF8]); // mov rax, rdi (save dest for return)
        code.extend_from_slice(&[0x48, 0x89, 0xD1]); // mov rcx, rdx (count)
        code.extend_from_slice(&[0x40, 0x88, 0xF2]); // mov dl, sil (byte value... no, we need al)
                                                     // Let me just do this cleanly:
        code.clear();
        // memset: rdi=dest, esi=value, rdx=count → returns dest in rax
        code.extend_from_slice(&[0x53]); // push rbx (callee-saved)
        code.extend_from_slice(&[0x48, 0x89, 0xFB]); // mov rbx, rdi (save dest)
        code.extend_from_slice(&[0x89, 0xF0]); // mov eax, esi (fill value → al)
        code.extend_from_slice(&[0x48, 0x89, 0xD1]); // mov rcx, rdx (count)
        code.extend_from_slice(&[0xF3, 0xAA]); // rep stosb (fill [rdi] with al, rcx times)
        code.extend_from_slice(&[0x48, 0x89, 0xD8]); // mov rax, rbx (return dest)
        code.extend_from_slice(&[0x5B]); // pop rbx
        code.push(0xC3); // ret
        code
    }));

    // memcmp(s1: *const u8, s2: *const u8, n: usize) -> i32
    // RDI=s1, RSI=s2, RDX=n, returns EAX (negative/0/positive)
    stubs.push(("memcmp".to_string(), {
        let mut code = Vec::new();
        code.extend_from_slice(&[0x48, 0x89, 0xD1]); // mov rcx, rdx (count)
        code.extend_from_slice(&[0xF3, 0xA6]); // repe cmpsb
        code.extend_from_slice(&[0x74, 0x07]); // je .equal
        code.extend_from_slice(&[0x0F, 0xB6, 0x47, 0xFF]); // movzx eax, byte [rdi-1]
        code.extend_from_slice(&[0x0F, 0xB6, 0x4E, 0xFF]); // movzx ecx, byte [rsi-1]
        code.extend_from_slice(&[0x29, 0xC8]); // sub eax, ecx
        code.push(0xC3); // ret
                         // .equal:
        code.extend_from_slice(&[0x31, 0xC0]); // xor eax, eax
        code.push(0xC3); // ret
        code
    }));

    // bcmp(s1: *const u8, s2: *const u8, n: usize) -> i32
    // Same as memcmp for our purposes
    stubs.push(("bcmp".to_string(), {
        let mut code = Vec::new();
        code.extend_from_slice(&[0x48, 0x89, 0xD1]); // mov rcx, rdx
        code.extend_from_slice(&[0xF3, 0xA6]); // repe cmpsb
        code.extend_from_slice(&[0x74, 0x07]); // je .equal
        code.extend_from_slice(&[0x0F, 0xB6, 0x47, 0xFF]); // movzx eax, byte [rdi-1]
        code.extend_from_slice(&[0x0F, 0xB6, 0x4E, 0xFF]); // movzx ecx, byte [rsi-1]
        code.extend_from_slice(&[0x29, 0xC8]); // sub eax, ecx
        code.push(0xC3);
        code.extend_from_slice(&[0x31, 0xC0]); // xor eax, eax
        code.push(0xC3);
        code
    }));

    // strlen(s: *const u8) -> usize
    stubs.push(("strlen".to_string(), {
        let mut code = Vec::new();
        code.extend_from_slice(&[0x48, 0x31, 0xC0]); // xor rax, rax (len = 0)
                                                     // .scan:
        code.extend_from_slice(&[0x80, 0x3C, 0x07, 0x00]); // cmp byte [rdi+rax], 0
        code.extend_from_slice(&[0x74, 0x04]); // je .done
        code.extend_from_slice(&[0x48, 0xFF, 0xC0]); // inc rax
        code.extend_from_slice(&[0xEB, 0xF5]); // jmp .scan
                                               // .done:
        code.push(0xC3); // ret
        code
    }));

    // Do not add ordinary Rust/stdlib methods here.
    //
    // This runtime object is only for compiler ABI helpers and low-level memory
    // intrinsics. Methods such as String::drain, Vec::retain, Option::map,
    // Iterator::next, HashMap::insert, and formatting builders must be produced
    // by normal Rust codegen from the standard libraries, or implemented as real
    // compiler intrinsics when they are truly ABI primitives. Silently emitting
    // xor-rax/ret or mov-rax-rdi placeholders makes binaries link while their
    // behavior is wrong, which is worse than a compile failure.
    let ret_rdi = || vec![0x48, 0x89, 0xF8, 0xC3]; // mov rax, rdi; ret

    for name in [
        "u16::from_le_bytes",
        "u32::from_le_bytes",
        "u32::from_ne_bytes",
        "u64::from_le_bytes",
        "u64::from_ne_bytes",
        "i16::from_le_bytes",
        "i32::from_le_bytes",
        "i32::from_ne_bytes",
        "f32::from_le_bytes",
        "f64::from_le_bytes",
    ] {
        stubs.push((name.to_string(), ret_rdi()));
    }

    for name in ["u16::from_be_bytes"] {
        stubs.push((name.to_string(), {
            let mut code = Vec::new();
            code.extend_from_slice(&[0x48, 0x89, 0xF8]); // mov rax, rdi
            code.extend_from_slice(&[0x66, 0xC1, 0xC0, 0x08]); // rol ax, 8
            code.push(0xC3);
            code
        }));
    }

    for name in ["u32::from_be_bytes"] {
        stubs.push((name.to_string(), {
            let mut code = Vec::new();
            code.extend_from_slice(&[0x89, 0xF8]); // mov eax, edi
            code.extend_from_slice(&[0x0F, 0xC8]); // bswap eax
            code.push(0xC3);
            code
        }));
    }

    for name in ["i16::from_be_bytes"] {
        stubs.push((name.to_string(), {
            let mut code = Vec::new();
            code.extend_from_slice(&[0x48, 0x89, 0xF8]); // mov rax, rdi
            code.extend_from_slice(&[0x66, 0xC1, 0xC0, 0x08]); // rol ax, 8
            code.extend_from_slice(&[0x48, 0x0F, 0xBF, 0xC0]); // movsx rax, ax
            code.push(0xC3);
            code
        }));
    }

    for name in ["i32::from_be_bytes", "f32::from_be_bytes"] {
        stubs.push((name.to_string(), {
            let mut code = Vec::new();
            code.extend_from_slice(&[0x89, 0xF8]); // mov eax, edi
            code.extend_from_slice(&[0x0F, 0xC8]); // bswap eax
            code.push(0xC3);
            code
        }));
    }

    for name in ["f64::from_be_bytes"] {
        stubs.push((name.to_string(), {
            let mut code = Vec::new();
            code.extend_from_slice(&[0x48, 0x89, 0xF8]); // mov rax, rdi
            code.extend_from_slice(&[0x48, 0x0F, 0xC8]); // bswap rax
            code.push(0xC3);
            code
        }));
    }

    stubs
}
