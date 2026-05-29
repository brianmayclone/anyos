use alloc::{
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};

const IMAGE_DOS_SIGNATURE: u16 = 0x5A4D;
const IMAGE_NT_SIGNATURE: u32 = 0x0000_4550;
const IMAGE_FILE_MACHINE_AMD64: u16 = 0x8664;
const IMAGE_FILE_EXECUTABLE_IMAGE: u16 = 0x0002;
const IMAGE_FILE_LARGE_ADDRESS_AWARE: u16 = 0x0020;
const IMAGE_FILE_DLL: u16 = 0x2000;
const IMAGE_NT_OPTIONAL_HDR64_MAGIC: u16 = 0x20B;
const IMAGE_SUBSYSTEM_WINDOWS_CUI: u16 = 3;
const IMAGE_SCN_CNT_CODE: u32 = 0x0000_0020;
const IMAGE_SCN_CNT_INITIALIZED_DATA: u32 = 0x0000_0040;
const IMAGE_SCN_MEM_EXECUTE: u32 = 0x2000_0000;
const IMAGE_SCN_MEM_READ: u32 = 0x4000_0000;

const FILE_ALIGNMENT: u32 = 0x200;
const SECTION_ALIGNMENT: u32 = 0x1000;
const PE_OFFSET: usize = 0x80;
const OPTIONAL_HEADER_SIZE: usize = 0xF0;
const HEADER_SIZE: usize = 0x200;
const TEXT_RVA: u32 = 0x1000;
const TEXT_RAW: u32 = HEADER_SIZE as u32;
const EXPORT_DIR_INDEX: usize = 0;
const DATA_DIR_COUNT: usize = 16;
const WXE_PROCESS_BLOCK_BASE: u64 = 0x0000_7FFE_E000_0000;
const WXE_PEB_BASE: u64 = 0x0000_7FFE_E000_1000;
const WXE_CMDLINE_A_ADDR: u64 = 0x0000_7FFE_E000_3000;
const WXE_CMDLINE_W_ADDR: u64 = 0x0000_7FFE_E000_3800;
const WXE_ENV_A_ADDR: u64 = 0x0000_7FFE_E000_5000;
const WXE_ENV_W_ADDR: u64 = 0x0000_7FFE_E000_6000;
// CRT scratch area inside the process block (must match `task::loader::windows`).
const WXE_CRT_SCRATCH_BASE: u64 = WXE_PROCESS_BLOCK_BASE + 0x7000;
const WXE_ERRNO_ADDR: u64 = WXE_CRT_SCRATCH_BASE;
const WXE_ARGC_ADDR: u64 = WXE_CRT_SCRATCH_BASE + 0x08;
const WXE_ARGV_PTR_ADDR: u64 = WXE_CRT_SCRATCH_BASE + 0x10;
const WXE_IOB_ADDR: u64 = WXE_CRT_SCRATCH_BASE + 0x80;

#[derive(Clone)]
struct ExportDef {
    name: String,
    kind: ExportKind,
}

#[derive(Clone)]
enum ExportKind {
    Code(Vec<u8>),
    Forward(String),
}

pub fn build_profile_dll(dll_name: &str, manifests: &[&str], ordinal_seed: u32) -> Vec<u8> {
    let mut exports = collect_exports(dll_name, manifests);
    if exports.is_empty() {
        exports.push(ExportDef {
            name: String::from("WxeDllPresent"),
            kind: ExportKind::Code(return_u32(1)),
        });
    }
    exports.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));

    let image_base = dll_image_base(dll_name, ordinal_seed);
    let mut text = Vec::new();
    let mut function_rvas = Vec::new();
    for export in &exports {
        match &export.kind {
            ExportKind::Code(code) => {
                let off = text.len() as u32;
                text.extend_from_slice(code);
                function_rvas.push(TEXT_RVA + off);
            }
            ExportKind::Forward(_) => function_rvas.push(0),
        }
    }
    if text.is_empty() {
        text.push(0xC3);
    }

    let text_virtual_size = text.len() as u32;
    let text_raw_size = align_up_u32(text_virtual_size, FILE_ALIGNMENT);
    let edata_rva = TEXT_RVA + align_up_u32(text_virtual_size, SECTION_ALIGNMENT);
    let edata_raw = TEXT_RAW + text_raw_size;
    let edata = build_export_section(dll_name, &exports, edata_rva, &mut function_rvas);
    let edata_virtual_size = edata.len() as u32;
    let edata_raw_size = align_up_u32(edata_virtual_size, FILE_ALIGNMENT);
    let size_of_image = align_up_u32(edata_rva + edata_virtual_size, SECTION_ALIGNMENT);

    let mut image = vec![0u8; (edata_raw + edata_raw_size) as usize];
    write_headers(
        &mut image,
        dll_name,
        image_base,
        text_virtual_size,
        text_raw_size,
        edata_rva,
        edata_virtual_size,
        edata_raw,
        edata_raw_size,
        size_of_image,
    );
    copy_at(&mut image, TEXT_RAW as usize, &text);
    copy_at(&mut image, edata_raw as usize, &edata);
    image
}

fn collect_exports(dll_name: &str, manifests: &[&str]) -> Vec<ExportDef> {
    let mut exports = Vec::new();
    for manifest in manifests {
        for line in manifest.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((left, route)) = split_once(line, '=') else {
                continue;
            };
            let Some((dll, name)) = split_once(left, '!') else {
                continue;
            };
            if !dll.eq_ignore_ascii_case(dll_name) {
                continue;
            }
            exports.push(ExportDef {
                name: name.to_string(),
                kind: export_kind(dll_name, name, route),
            });
        }
    }
    exports
}

fn export_kind(dll_name: &str, name: &str, route: &str) -> ExportKind {
    if let Some(target) = forwarder_target(route) {
        return ExportKind::Forward(target);
    }
    if let Some(id) = nt_service_id(route) {
        return ExportKind::Code(nt_syscall_stub(id));
    }

    let code = match route {
        "win32:process:exit" => exit_process_stub(),
        "win32:pseudo:process" => return_i64(-1),
        "win32:pseudo:thread" => return_i64(-2),
        "win32:id:process" | "win32:id:thread" => return_u32(1),
        "win32:process:cmdline-a" => return_u64(WXE_CMDLINE_A_ADDR),
        "win32:process:cmdline-w" => return_u64(WXE_CMDLINE_W_ADDR),
        "win32:env:block-a" => return_u64(WXE_ENV_A_ADDR),
        "win32:env:block-w" => return_u64(WXE_ENV_W_ADDR),
        "win32:env:free" => return_u32(1),
        "win32:module:handle-a" => raw_syscall_stub(0x0100),
        "win32:module:handle-w" => raw_syscall_stub(0x0101),
        "win32:module:proc" => raw_syscall_stub(0x0102),
        "win32:module:load-a" => raw_syscall_stub(0x0103),
        "win32:module:load-w" | "win32:module:load-ex-w" => raw_syscall_stub(0x0104),
        "win32:console:std-handle" => sign_extend_ecx_stub(),
        "win32:console:set-std-handle" => return_u32(1),
        "win32:console:get-mode" | "win32:console:set-mode" => return_u32(1),
        "win32:file:create-a" => raw_syscall_stub(0x0105),
        "win32:file:create-w" => raw_syscall_stub(0x0106),
        "win32:console:read-a" | "win32:console:read-w" => nt_io_file_stub(4),
        "win32:console:write-a" | "win32:console:write-w" => nt_io_file_stub(5),
        "win32:file:read" => nt_io_file_stub(4),
        "win32:file:write" => nt_io_file_stub(5),
        "win32:file:attributes-a" => raw_syscall_stub(0x0107),
        "win32:file:attributes-w" => raw_syscall_stub(0x0108),
        "win32:file:size-ex" => raw_syscall_stub(0x0109),
        "win32:file:pointer-ex" => raw_syscall_stub(0x010a),
        "win32:file:type" => return_u32(2),
        "win32:handle:close" => nt_status_bool_stub(3),
        "win32:time:sleep" => sleep_stub(),
        "win32:time:qpc" => qpc_stub(),
        "win32:time:qpf" => qpf_stub(),
        "win32:time:filetime" => system_time_stub(),
        "win32:memory:alloc" => virtual_alloc_stub(),
        "win32:memory:free" => virtual_free_stub(),
        "win32:memory:protect" => virtual_protect_stub(),
        "win32:heap:create" | "win32:heap:process" => return_u32(1),
        "win32:heap:destroy" => return_u32(1),
        // HeapAlloc(heap, flags, size): size is the 3rd Win64 arg (r8).
        "win32:heap:alloc" => priv_syscall_from_r8(0x0200),
        // HeapReAlloc(heap, flags, ptr, size): ptr=r8 -> a1, size=r9 -> a2.
        "win32:heap:realloc" => heap_realloc_win_stub(),
        // HeapFree(heap, flags, ptr): ptr=r8.
        "win32:heap:free" => priv_syscall_from_r8(0x0201),
        // HeapSize(heap, flags, ptr): ptr=r8.
        "win32:heap:size" => priv_syscall_from_r8(0x0203),
        // CRT heap: malloc(size)/free(ptr)/realloc(ptr,size)/calloc(n,size).
        "ucrt:malloc" => priv_syscall_stub(0x0200),
        "ucrt:free" => priv_syscall_stub(0x0201),
        "ucrt:realloc" => priv_syscall_stub(0x0202),
        "ucrt:calloc" => calloc_stub(),
        "ucrt:memcpy" => priv_syscall_stub(0x0250),
        "ucrt:memmove" => priv_syscall_stub(0x0251),
        "ucrt:memset" => priv_syscall_stub(0x0252),
        "ucrt:memcmp" => priv_syscall_stub(0x0253),
        "ucrt:initterm" => initterm_stub(false),
        "ucrt:initterm-e" => initterm_stub(true),
        "ucrt:errno-ptr" => return_u64(WXE_ERRNO_ADDR),
        "ucrt:argc-ptr" => return_u64(WXE_ARGC_ADDR),
        "ucrt:argv-ptr" => return_u64(WXE_ARGV_PTR_ADDR),
        "ucrt:narrow-env-get" => return_u64(WXE_ENV_A_ADDR),
        "ucrt:iob" => return_u64(WXE_IOB_ADDR),
        "ucrt:vswprintf" => priv_syscall_stub(0x0230),
        "ucrt:exit" => exit_process_stub(),
        "win32:env:get-w" => priv_syscall_stub(0x0221),
        "win32:env:set-w" => priv_syscall_stub(0x0222),
        "win32:env:expand-w" => priv_syscall_stub(0x0224),
        "win32:module:filename-w" => priv_syscall_stub(0x0223),
        "win32:console:write-con-w" => priv_syscall_stub(0x0210),
        "win32:console:read-con-w" => priv_syscall_stub(0x0211),
        "win32:console:write-con-a" => priv_syscall_stub(0x0212),
        "win32:console:screen-info" => priv_syscall_stub(0x0213),
        "win32:console:output-cp" => return_u32(65001),
        "win32:console:ctrl-handler" => return_u32(1),
        "win32:console:window" => return_u32(0),
        "win32:startup-info-w" => priv_syscall_stub(0x0220),
        "win32:dir:get-w" => priv_syscall_stub(0x0225),
        "win32:dir:set-w" => priv_syscall_stub(0x0226),
        "win32:osf-handle" => priv_syscall_stub(0x0240),
        "win32:acp" => return_u32(65001),
        "stub:one" => return_u32(1),
        "stub:zero" => return_u32(0),
        "anyui:dialog:message-box-a" | "anyui:dialog:message-box-w" => return_u32(1),
        "wxe-ui:pseudo:desktop-window" => return_u32(1),
        _ if dll_name.eq_ignore_ascii_case("ntdll.dll") && name == "RtlGetCurrentPeb" => {
            return_u64(WXE_PEB_BASE)
        }
        // RtlAllocateHeap(heap, flags, size): size is the 3rd arg (r8).
        _ if dll_name.eq_ignore_ascii_case("ntdll.dll") && name == "RtlAllocateHeap" => {
            priv_syscall_from_r8(0x0200)
        }
        _ if dll_name.eq_ignore_ascii_case("ntdll.dll") && name == "RtlReAllocateHeap" => {
            heap_realloc_win_stub()
        }
        _ if dll_name.eq_ignore_ascii_case("ntdll.dll") && name == "RtlFreeHeap" => {
            priv_syscall_from_r8(0x0201)
        }
        _ if dll_name.eq_ignore_ascii_case("ntdll.dll") && name.starts_with("Rtl") => return_u32(0),
        _ => return_u32(0),
    };
    ExportKind::Code(code)
}

fn forwarder_target(route: &str) -> Option<String> {
    let target = route.strip_prefix("forward:")?;
    let (dll, export) = split_once(target, '!')?;
    let dll = dll.strip_suffix(".dll").unwrap_or(dll);
    Some(format!("{}.{}", dll, export))
}

fn nt_service_id(route: &str) -> Option<u32> {
    let hex = route.strip_prefix("nt:0x")?;
    u32::from_str_radix(hex, 16).ok()
}

fn nt_syscall_stub(id: u32) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0x4C, 0x8B, 0xD1]); // mov r10, rcx
    out.push(0xB8); // mov eax, imm32
    out.extend_from_slice(&id.to_le_bytes());
    out.extend_from_slice(&[0x0F, 0x05, 0xC3]); // syscall; ret
    out
}

fn raw_syscall_stub(id: u32) -> Vec<u8> {
    let mut out = Vec::new();
    append_syscall(&mut out, id);
    out.push(0xC3);
    out
}

/// `mov r10, rcx; mov eax, id; syscall; ret` — maps the 1st Win64 arg (rcx) to
/// service arg a1. The 2nd/3rd/4th Win64 args (rdx/r8/r9) already line up with
/// a2/a3/a4, and stack args are read by the kernel via `stack_arg`.
fn priv_syscall_stub(id: u32) -> Vec<u8> {
    raw_syscall_stub(id)
}

/// `mov r10, r8; mov eax, id; syscall; ret` — for Win32 calls whose meaningful
/// argument is the 3rd one (e.g. HeapAlloc/HeapFree/HeapSize: the pointer/size).
fn priv_syscall_from_r8(id: u32) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0x4D, 0x89, 0xC2]); // mov r10, r8
    out.push(0xB8);
    out.extend_from_slice(&id.to_le_bytes());
    out.extend_from_slice(&[0x0F, 0x05, 0xC3]); // syscall; ret
    out
}

/// HeapReAlloc(heap, flags, ptr=r8, size=r9): a1=ptr, a2=size.
fn heap_realloc_win_stub() -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0x4D, 0x89, 0xC2]); // mov r10, r8  (ptr)
    out.extend_from_slice(&[0x4C, 0x89, 0xCA]); // mov rdx, r9  (size)
    out.push(0xB8);
    out.extend_from_slice(&0x0202u32.to_le_bytes());
    out.extend_from_slice(&[0x0F, 0x05, 0xC3]);
    out
}

/// calloc(n=rcx, size=rdx): pass the product as a1 (mmap zeroes the block).
fn calloc_stub() -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0x48, 0x89, 0xC8]); // mov rax, rcx
    out.extend_from_slice(&[0x48, 0x0F, 0xAF, 0xC2]); // imul rax, rdx
    out.extend_from_slice(&[0x49, 0x89, 0xC2]); // mov r10, rax
    out.push(0xB8);
    out.extend_from_slice(&0x0200u32.to_le_bytes());
    out.extend_from_slice(&[0x0F, 0x05, 0xC3]);
    out
}

/// _initterm(first=rcx, last=rdx) / _initterm_e: call each non-null function
/// pointer in `[first, last)`. The `_e` form stops and returns the first
/// non-zero result. Hand-assembled; preserves rbx/rsi and keeps 16-byte stack
/// alignment across each `call`.
fn initterm_stub(check_e: bool) -> Vec<u8> {
    if check_e {
        // See kernel-side notes; offsets verified by hand (48 bytes).
        Vec::from([
            0x53, 0x56, 0x57, // push rbx; push rsi; push rdi
            0x48, 0x89, 0xCB, // mov rbx, rcx  (cursor = first)
            0x48, 0x89, 0xD6, // mov rsi, rdx  (end = last)
            0x48, 0x39, 0xF3, // .loop: cmp rbx, rsi
            0x73, 0x1C, // jae .done0
            0x48, 0x8B, 0x03, // mov rax, [rbx]
            0x48, 0x85, 0xC0, // test rax, rax
            0x74, 0x0E, // jz .skip
            0x48, 0x83, 0xEC, 0x20, // sub rsp, 0x20
            0xFF, 0xD0, // call rax
            0x48, 0x83, 0xC4, 0x20, // add rsp, 0x20
            0x85, 0xC0, // test eax, eax
            0x75, 0x08, // jnz .ret
            0x48, 0x83, 0xC3, 0x08, // .skip: add rbx, 8
            0xEB, 0xDF, // jmp .loop
            0x31, 0xC0, // .done0: xor eax, eax
            0x5F, 0x5E, 0x5B, 0xC3, // .ret: pop rdi; pop rsi; pop rbx; ret
        ])
    } else {
        // 42 bytes.
        Vec::from([
            0x53, 0x56, 0x57, // push rbx; push rsi; push rdi
            0x48, 0x89, 0xCB, // mov rbx, rcx
            0x48, 0x89, 0xD6, // mov rsi, rdx
            0x48, 0x39, 0xF3, // .loop: cmp rbx, rsi
            0x73, 0x18, // jae .done
            0x48, 0x8B, 0x03, // mov rax, [rbx]
            0x48, 0x85, 0xC0, // test rax, rax
            0x74, 0x0A, // jz .skip
            0x48, 0x83, 0xEC, 0x20, // sub rsp, 0x20
            0xFF, 0xD0, // call rax
            0x48, 0x83, 0xC4, 0x20, // add rsp, 0x20
            0x48, 0x83, 0xC3, 0x08, // .skip: add rbx, 8
            0xEB, 0xE3, // jmp .loop
            0x5F, 0x5E, 0x5B, 0xC3, // .done: pop rdi; pop rsi; pop rbx; ret
        ])
    }
}

fn exit_process_stub() -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0x48, 0x89, 0xCA]); // mov rdx, rcx
    out.extend_from_slice(&[0x48, 0xC7, 0xC1, 0xFF, 0xFF, 0xFF, 0xFF]); // mov rcx, -1
    out.extend_from_slice(&[0x4C, 0x8B, 0xD1]); // mov r10, rcx
    out.push(0xB8);
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&[0x0F, 0x05, 0xC3]);
    out
}

fn qpc_stub() -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0x4C, 0x8B, 0xD1]); // mov r10, rcx
    out.extend_from_slice(&[0x31, 0xD2]); // xor edx, edx
    out.push(0xB8);
    out.extend_from_slice(&8u32.to_le_bytes());
    out.extend_from_slice(&[0x0F, 0x05]); // syscall
    out.extend_from_slice(&[0x85, 0xC0]); // test eax, eax
    out.extend_from_slice(&[0x0F, 0x94, 0xC0]); // sete al
    out.extend_from_slice(&[0x0F, 0xB6, 0xC0, 0xC3]); // movzx eax, al; ret
    out
}

fn qpf_stub() -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0x48, 0xB8]);
    out.extend_from_slice(&1000u64.to_le_bytes());
    out.extend_from_slice(&[0x48, 0x89, 0x01]); // mov [rcx], rax
    out.extend_from_slice(&[0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3]);
    out
}

fn system_time_stub() -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0x4C, 0x8B, 0xD1]);
    out.push(0xB8);
    out.extend_from_slice(&7u32.to_le_bytes());
    out.extend_from_slice(&[0x0F, 0x05, 0xC3]);
    out
}

fn sign_extend_ecx_stub() -> Vec<u8> {
    Vec::from([0x48, 0x63, 0xC1, 0xC3])
}

fn nt_status_bool_stub(id: u32) -> Vec<u8> {
    let mut out = nt_syscall_stub_without_ret(id);
    append_status_bool_return(&mut out, 0);
    out
}

fn nt_io_file_stub(id: u32) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0x48, 0x83, 0xEC, 0x68]); // sub rsp, 0x68
    out.extend_from_slice(&[0x4C, 0x89, 0x4C, 0x24, 0x60]); // save lpNumberOfBytes*
    out.extend_from_slice(&[0x48, 0x89, 0x54, 0x24, 0x30]); // arg6 buffer
    out.extend_from_slice(&[0x4C, 0x89, 0x44, 0x24, 0x38]); // arg7 length
    out.extend_from_slice(&[0x48, 0xC7, 0x44, 0x24, 0x40, 0, 0, 0, 0]); // arg8 byte offset
    out.extend_from_slice(&[0x48, 0xC7, 0x44, 0x24, 0x48, 0, 0, 0, 0]); // arg9 key
    out.extend_from_slice(&[0x48, 0x8D, 0x44, 0x24, 0x50]); // lea rax, io_status
    out.extend_from_slice(&[0x48, 0x89, 0x44, 0x24, 0x28]); // arg5 io_status
    out.extend_from_slice(&[0x31, 0xC0]); // xor eax, eax
    out.extend_from_slice(&[0x48, 0x89, 0x44, 0x24, 0x50]);
    out.extend_from_slice(&[0x48, 0x89, 0x44, 0x24, 0x58]);
    out.extend_from_slice(&[0x31, 0xD2]); // xor edx, edx
    out.extend_from_slice(&[0x45, 0x31, 0xC0]); // xor r8d, r8d
    out.extend_from_slice(&[0x45, 0x31, 0xC9]); // xor r9d, r9d
    out.extend_from_slice(&[0x4C, 0x8B, 0xD1]); // mov r10, rcx
    out.push(0xB8);
    out.extend_from_slice(&id.to_le_bytes());
    out.extend_from_slice(&[0x0F, 0x05]); // syscall
    out.extend_from_slice(&[0x85, 0xC0]); // test eax, eax
    let fail_jne = out.len();
    out.extend_from_slice(&[0x75, 0]); // jne fail
    out.extend_from_slice(&[0x48, 0x8B, 0x4C, 0x24, 0x60]); // load lpNumberOfBytes*
    out.extend_from_slice(&[0x48, 0x85, 0xC9]); // test rcx, rcx
    let success_je = out.len();
    out.extend_from_slice(&[0x74, 0]); // je success
    out.extend_from_slice(&[0x48, 0x8B, 0x44, 0x24, 0x58]); // load IO_STATUS.Information
    out.extend_from_slice(&[0x89, 0x01]); // mov [rcx], eax
    let success_pos = out.len();
    out.extend_from_slice(&[0xB8, 1, 0, 0, 0]); // mov eax, 1
    out.extend_from_slice(&[0x48, 0x83, 0xC4, 0x68, 0xC3]); // add rsp, 0x68; ret
    let fail_pos = out.len();
    out.extend_from_slice(&[0x31, 0xC0]); // xor eax, eax
    out.extend_from_slice(&[0x48, 0x83, 0xC4, 0x68, 0xC3]);
    patch_rel8(&mut out, fail_jne, fail_pos);
    patch_rel8(&mut out, success_je, success_pos);
    out
}

fn sleep_stub() -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0x48, 0x83, 0xEC, 0x18]); // sub rsp, 0x18
    out.extend_from_slice(&[0x48, 0x89, 0xC8]); // mov rax, rcx
    out.extend_from_slice(&[0x48, 0x69, 0xC0, 0x10, 0x27, 0, 0]); // imul rax, rax, 10000
    out.extend_from_slice(&[0x48, 0xF7, 0xD8]); // neg rax
    out.extend_from_slice(&[0x48, 0x89, 0x44, 0x24, 0x08]); // store interval
    out.extend_from_slice(&[0x31, 0xC9]); // xor ecx, ecx
    out.extend_from_slice(&[0x48, 0x8D, 0x54, 0x24, 0x08]); // lea rdx, interval
    out.extend_from_slice(&[0x4C, 0x8B, 0xD1]); // mov r10, rcx
    out.push(0xB8);
    out.extend_from_slice(&6u32.to_le_bytes());
    out.extend_from_slice(&[0x0F, 0x05]); // syscall
    out.extend_from_slice(&[0x48, 0x83, 0xC4, 0x18, 0xC3]);
    out
}

fn virtual_alloc_stub() -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0x48, 0x83, 0xEC, 0x58]); // sub rsp, 0x58
    out.extend_from_slice(&[0x48, 0x89, 0x4C, 0x24, 0x40]); // local base
    out.extend_from_slice(&[0x48, 0x89, 0x54, 0x24, 0x48]); // local size
    out.extend_from_slice(&[0x4C, 0x89, 0x44, 0x24, 0x28]); // arg5 allocation type
    out.extend_from_slice(&[0x4C, 0x89, 0x4C, 0x24, 0x30]); // arg6 protect
    out.extend_from_slice(&[0x48, 0xC7, 0xC1, 0xFF, 0xFF, 0xFF, 0xFF]); // process -1
    out.extend_from_slice(&[0x48, 0x8D, 0x54, 0x24, 0x40]); // base ptr
    out.extend_from_slice(&[0x45, 0x31, 0xC0]); // zero bits
    out.extend_from_slice(&[0x4C, 0x8D, 0x4C, 0x24, 0x48]); // size ptr
    append_syscall(&mut out, 9);
    out.extend_from_slice(&[0x85, 0xC0]);
    let fail_jne = out.len();
    out.extend_from_slice(&[0x75, 0]);
    out.extend_from_slice(&[0x48, 0x8B, 0x44, 0x24, 0x40]); // return base
    out.extend_from_slice(&[0x48, 0x83, 0xC4, 0x58, 0xC3]);
    let fail_pos = out.len();
    out.extend_from_slice(&[0x31, 0xC0]);
    out.extend_from_slice(&[0x48, 0x83, 0xC4, 0x58, 0xC3]);
    patch_rel8(&mut out, fail_jne, fail_pos);
    out
}

fn virtual_free_stub() -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0x48, 0x83, 0xEC, 0x58]);
    out.extend_from_slice(&[0x48, 0x89, 0x4C, 0x24, 0x40]); // base local
    out.extend_from_slice(&[0x48, 0x89, 0x54, 0x24, 0x48]); // size local
    out.extend_from_slice(&[0x4D, 0x89, 0xC1]); // mov r9, r8
    out.extend_from_slice(&[0x48, 0xC7, 0xC1, 0xFF, 0xFF, 0xFF, 0xFF]);
    out.extend_from_slice(&[0x48, 0x8D, 0x54, 0x24, 0x40]);
    out.extend_from_slice(&[0x4C, 0x8D, 0x44, 0x24, 0x48]);
    append_syscall(&mut out, 10);
    append_status_bool_return(&mut out, 0x58);
    out
}

fn virtual_protect_stub() -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0x48, 0x83, 0xEC, 0x58]);
    out.extend_from_slice(&[0x48, 0x89, 0x4C, 0x24, 0x40]); // base local
    out.extend_from_slice(&[0x48, 0x89, 0x54, 0x24, 0x48]); // size local
    out.extend_from_slice(&[0x4C, 0x89, 0x4C, 0x24, 0x28]); // old protect stack arg
    out.extend_from_slice(&[0x4D, 0x89, 0xC1]); // new protect to r9
    out.extend_from_slice(&[0x48, 0xC7, 0xC1, 0xFF, 0xFF, 0xFF, 0xFF]);
    out.extend_from_slice(&[0x48, 0x8D, 0x54, 0x24, 0x40]);
    out.extend_from_slice(&[0x4C, 0x8D, 0x44, 0x24, 0x48]);
    append_syscall(&mut out, 11);
    append_status_bool_return(&mut out, 0x58);
    out
}

fn nt_syscall_stub_without_ret(id: u32) -> Vec<u8> {
    let mut out = Vec::new();
    append_syscall(&mut out, id);
    out
}

fn append_syscall(out: &mut Vec<u8>, id: u32) {
    out.extend_from_slice(&[0x4C, 0x8B, 0xD1]); // mov r10, rcx
    out.push(0xB8);
    out.extend_from_slice(&id.to_le_bytes());
    out.extend_from_slice(&[0x0F, 0x05]); // syscall
}

fn append_status_bool_return(out: &mut Vec<u8>, stack_size: u8) {
    out.extend_from_slice(&[0x85, 0xC0]); // test eax, eax
    let fail_jne = out.len();
    out.extend_from_slice(&[0x75, 0]); // jne fail
    out.extend_from_slice(&[0xB8, 1, 0, 0, 0]); // mov eax, 1
    if stack_size != 0 {
        out.extend_from_slice(&[0x48, 0x83, 0xC4, stack_size]);
    }
    out.push(0xC3);
    let fail_pos = out.len();
    out.extend_from_slice(&[0x31, 0xC0]); // xor eax, eax
    if stack_size != 0 {
        out.extend_from_slice(&[0x48, 0x83, 0xC4, stack_size]);
    }
    out.push(0xC3);
    patch_rel8(out, fail_jne, fail_pos);
}

fn patch_rel8(out: &mut [u8], branch_pos: usize, target_pos: usize) {
    let rel = target_pos as isize - (branch_pos + 2) as isize;
    out[branch_pos + 1] = rel as i8 as u8;
}

fn return_u32(value: u32) -> Vec<u8> {
    if value == 0 {
        Vec::from([0x31, 0xC0, 0xC3])
    } else {
        let mut out = Vec::new();
        out.push(0xB8);
        out.extend_from_slice(&value.to_le_bytes());
        out.push(0xC3);
        out
    }
}

fn return_i64(value: i64) -> Vec<u8> {
    return_u64(value as u64)
}

fn return_u64(value: u64) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0x48, 0xB8]);
    out.extend_from_slice(&value.to_le_bytes());
    out.push(0xC3);
    out
}

fn build_export_section(
    dll_name: &str,
    exports: &[ExportDef],
    edata_rva: u32,
    function_rvas: &mut [u32],
) -> Vec<u8> {
    let n = exports.len();
    let eat_off = 40usize;
    let name_ptrs_off = eat_off + n * 4;
    let ordinals_off = name_ptrs_off + n * 4;
    let mut edata = vec![0u8; ordinals_off + n * 2];

    let dll_name_off = push_cstr(&mut edata, dll_name);
    let mut name_offsets = Vec::new();
    for export in exports {
        name_offsets.push(push_cstr(&mut edata, &export.name));
    }
    for (idx, export) in exports.iter().enumerate() {
        if let ExportKind::Forward(target) = &export.kind {
            let off = push_cstr(&mut edata, target);
            function_rvas[idx] = edata_rva + off as u32;
        }
    }

    put_u32(&mut edata, 12, edata_rva + dll_name_off as u32);
    put_u32(&mut edata, 16, 1);
    put_u32(&mut edata, 20, n as u32);
    put_u32(&mut edata, 24, n as u32);
    put_u32(&mut edata, 28, edata_rva + eat_off as u32);
    put_u32(&mut edata, 32, edata_rva + name_ptrs_off as u32);
    put_u32(&mut edata, 36, edata_rva + ordinals_off as u32);

    for (idx, rva) in function_rvas.iter().enumerate() {
        put_u32(&mut edata, eat_off + idx * 4, *rva);
        put_u32(
            &mut edata,
            name_ptrs_off + idx * 4,
            edata_rva + name_offsets[idx] as u32,
        );
        put_u16(&mut edata, ordinals_off + idx * 2, idx as u16);
    }

    edata
}

fn write_headers(
    out: &mut [u8],
    dll_name: &str,
    image_base: u64,
    text_virtual_size: u32,
    text_raw_size: u32,
    edata_rva: u32,
    edata_virtual_size: u32,
    edata_raw: u32,
    edata_raw_size: u32,
    size_of_image: u32,
) {
    put_u16(out, 0, IMAGE_DOS_SIGNATURE);
    put_u32(out, 0x3C, PE_OFFSET as u32);

    let pe = PE_OFFSET;
    put_u32(out, pe, IMAGE_NT_SIGNATURE);
    let coff = pe + 4;
    put_u16(out, coff, IMAGE_FILE_MACHINE_AMD64);
    put_u16(out, coff + 2, 2);
    put_u16(out, coff + 16, OPTIONAL_HEADER_SIZE as u16);
    put_u16(
        out,
        coff + 18,
        IMAGE_FILE_EXECUTABLE_IMAGE | IMAGE_FILE_LARGE_ADDRESS_AWARE | IMAGE_FILE_DLL,
    );

    let opt = coff + 20;
    put_u16(out, opt, IMAGE_NT_OPTIONAL_HDR64_MAGIC);
    out[opt + 2] = 1;
    put_u32(out, opt + 4, text_raw_size);
    put_u32(out, opt + 8, edata_raw_size);
    put_u32(out, opt + 20, TEXT_RVA);
    put_u64(out, opt + 24, image_base);
    put_u32(out, opt + 32, SECTION_ALIGNMENT);
    put_u32(out, opt + 36, FILE_ALIGNMENT);
    put_u16(out, opt + 40, 10);
    put_u16(out, opt + 48, 10);
    put_u32(out, opt + 56, size_of_image);
    put_u32(out, opt + 60, HEADER_SIZE as u32);
    put_u16(out, opt + 68, IMAGE_SUBSYSTEM_WINDOWS_CUI);
    put_u16(out, opt + 70, 0x0100); // NX compatible
    put_u64(out, opt + 72, 0x100000);
    put_u64(out, opt + 80, 0x1000);
    put_u64(out, opt + 88, 0x100000);
    put_u64(out, opt + 96, 0x1000);
    put_u32(out, opt + 108, DATA_DIR_COUNT as u32);
    let export_dir = opt + 112 + EXPORT_DIR_INDEX * 8;
    put_u32(out, export_dir, edata_rva);
    put_u32(out, export_dir + 4, edata_virtual_size);

    let section = opt + OPTIONAL_HEADER_SIZE;
    write_section_name(out, section, ".text");
    put_u32(out, section + 8, text_virtual_size);
    put_u32(out, section + 12, TEXT_RVA);
    put_u32(out, section + 16, text_raw_size);
    put_u32(out, section + 20, TEXT_RAW);
    put_u32(
        out,
        section + 36,
        IMAGE_SCN_CNT_CODE | IMAGE_SCN_MEM_EXECUTE | IMAGE_SCN_MEM_READ,
    );

    let section = section + 40;
    write_section_name(out, section, ".edata");
    put_u32(out, section + 8, edata_virtual_size);
    put_u32(out, section + 12, edata_rva);
    put_u32(out, section + 16, edata_raw_size);
    put_u32(out, section + 20, edata_raw);
    put_u32(
        out,
        section + 36,
        IMAGE_SCN_CNT_INITIALIZED_DATA | IMAGE_SCN_MEM_READ,
    );

    let banner = format!("WXE generated {}\r\n", dll_name);
    let start = 0x40usize;
    let end = core::cmp::min(start + banner.len(), PE_OFFSET - 1);
    copy_at(out, start, &banner.as_bytes()[..end - start]);
}

fn dll_image_base(name: &str, seed: u32) -> u64 {
    let mut hash = seed.wrapping_add(0x5745_5845);
    for byte in name.bytes() {
        hash = hash.rotate_left(5) ^ (byte.to_ascii_lowercase() as u32);
    }
    let slot = if seed == 0 {
        (hash as u64 & 0x0FFF).max(1)
    } else {
        (seed as u64 & 0x0FFF).max(1)
    };
    0x0000_0001_8000_0000u64 + slot * 0x0100_0000
}

fn push_cstr(buf: &mut Vec<u8>, value: &str) -> usize {
    let off = buf.len();
    buf.extend_from_slice(value.as_bytes());
    buf.push(0);
    off
}

fn write_section_name(out: &mut [u8], off: usize, name: &str) {
    let bytes = name.as_bytes();
    let len = core::cmp::min(bytes.len(), 8);
    copy_at(out, off, &bytes[..len]);
}

fn split_once(value: &str, needle: char) -> Option<(&str, &str)> {
    let idx = value.find(needle)?;
    Some((&value[..idx], &value[idx + needle.len_utf8()..]))
}

fn copy_at(out: &mut [u8], off: usize, data: &[u8]) {
    let end = off + data.len();
    out[off..end].copy_from_slice(data);
}

fn align_up_u32(value: u32, align: u32) -> u32 {
    (value + align - 1) & !(align - 1)
}

fn put_u16(out: &mut [u8], off: usize, value: u16) {
    copy_at(out, off, &value.to_le_bytes());
}

fn put_u32(out: &mut [u8], off: usize, value: u32) {
    copy_at(out, off, &value.to_le_bytes());
}

fn put_u64(out: &mut [u8], off: usize, value: u64) {
    copy_at(out, off, &value.to_le_bytes());
}
