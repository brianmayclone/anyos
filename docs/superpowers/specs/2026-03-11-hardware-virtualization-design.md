# Hardware Virtualization for libcorevm

**Date:** 2026-03-11
**Status:** Draft
**Scope:** Replace software CPU emulation with Intel VT-x / AMD-V hardware virtualization

## 1. Overview

libcorevm is being redesigned from a pure-software x86 emulator to a hardware-virtualization-based hypervisor. All software instruction decoding, execution, JIT compilation, and software MMU are removed. The VM guest runs natively on the CPU via VMX (Intel) or SVM (AMD). If the host CPU lacks VT-x/AMD-V support, virtualization is unavailable — there is no fallback.

Three platform backends share a common `VmBackend` trait:

| Platform | Backend | Access Method | Feature Flag |
|----------|---------|---------------|--------------|
| anyOS | `AnyOsBackend` | Kernel syscalls → direct VMX/SVM | `anyos` (no_std) |
| Linux | `KvmBackend` | `/dev/kvm` ioctl | `linux` (std) |
| Windows | `WhpBackend` | Windows Hypervisor Platform API | `windows` (std) |

## 2. What Gets Removed

- `src/decoder.rs` — instruction decoder
- `src/executor/` — all 10 execution sub-modules (arith, control, data, flags, io, misc, mod, sse, string, system)
- `src/jit/` — decode cache, basic block detection, native code compiler
- `src/memory/paging.rs` — software page table walkers (2-level, PAE, 4-level)
- `src/fpu_state.rs` — software x87 FPU state
- `src/sse_state.rs` — software SSE register state
- `src/cpu.rs` — software fetch-decode-execute loop (replaced by new VM run-loop)
- SMC dirty tracking, decode cache invalidation, instruction counter logic
- All JIT-related FFI functions

## 3. What Gets Retained

All device models remain unchanged:

- **Interrupt controllers:** PIC (8259A), IOAPIC, LAPIC
- **Timers:** PIT (8254), CMOS/RTC
- **Storage:** IDE, AHCI (ICH9)
- **Network:** E1000
- **Display:** SVGA/VGA
- **Input:** PS/2 (keyboard + mouse)
- **Serial:** 16550 UART
- **Bus:** PCI configuration space
- **Power:** ACPI, APM
- **Misc:** fw_cfg, port 0x61 (speaker/NMI), debug port (0xE9)

Also retained:
- Interrupt routing chain (PIC → IOAPIC → LAPIC)
- PCI bus emulation and BAR management
- Guest RAM allocation (but mapped via EPT/NPT instead of software MMU)
- BIOS loading infrastructure

## 4. VmBackend Trait

```rust
/// Construction is not part of the trait (not object-safe).
/// Each backend provides `BackendType::new(ram_size) -> Result<Self, VmError>`.
pub trait VmBackend {
    fn destroy(&mut self);
    fn reset(&mut self) -> Result<(), VmError>;

    // Memory
    fn set_memory_region(&mut self, slot: u32, guest_phys: u64, size: u64, host_ptr: *mut u8) -> Result<(), VmError>;
    fn read_phys(&self, addr: u64, buf: &mut [u8]) -> Result<(), VmError>;
    fn write_phys(&mut self, addr: u64, buf: &[u8]) -> Result<(), VmError>;

    // vCPU
    fn create_vcpu(&mut self, id: u32) -> Result<(), VmError>;
    fn destroy_vcpu(&mut self, id: u32) -> Result<(), VmError>;
    fn run_vcpu(&mut self, id: u32) -> Result<VmExitReason, VmError>;
    fn get_vcpu_regs(&self, id: u32) -> Result<VcpuRegs, VmError>;
    fn set_vcpu_regs(&mut self, id: u32, regs: &VcpuRegs) -> Result<(), VmError>;
    fn get_vcpu_sregs(&self, id: u32) -> Result<VcpuSregs, VmError>;
    fn set_vcpu_sregs(&mut self, id: u32, sregs: &VcpuSregs) -> Result<(), VmError>;
    fn inject_interrupt(&mut self, id: u32, vector: u8) -> Result<(), VmError>;
    fn inject_exception(&mut self, id: u32, vector: u8, error_code: Option<u32>) -> Result<(), VmError>;
    fn inject_nmi(&mut self, id: u32) -> Result<(), VmError>;
    fn request_interrupt_window(&mut self, id: u32, enable: bool) -> Result<(), VmError>;
    fn set_cpuid(&mut self, entries: &[CpuidEntry]) -> Result<(), VmError>;
}
```

The core library and all device models remain `no_std`. Only the Linux (`KvmBackend`) and Windows (`WhpBackend`) backends require `std`, enabled via feature flags.

## 5. New FFI API

Replaces the previous 58 functions with ~20 virtualization-oriented functions.

### 5.1 Data Structures

```c
// All general-purpose registers + RIP + RFLAGS
struct VcpuRegs {
    uint64_t rax, rbx, rcx, rdx, rsi, rdi, rbp, rsp;
    uint64_t r8, r9, r10, r11, r12, r13, r14, r15;
    uint64_t rip, rflags;
};

// Segment register descriptor
struct SegmentReg {
    uint64_t base;
    uint32_t limit;
    uint16_t selector;
    uint8_t  type_;
    uint8_t  present;
    uint8_t  dpl;
    uint8_t  db;
    uint8_t  s;
    uint8_t  l;
    uint8_t  g;
    uint8_t  avl;
};

// CPUID entry for guest feature masking
struct CpuidEntry {
    uint32_t function;
    uint32_t index;
    uint32_t eax, ebx, ecx, edx;
};

// System registers: segments, control regs, descriptor tables
struct VcpuSregs {
    SegmentReg cs, ds, es, fs, gs, ss, tr, ldt;
    struct { uint64_t base; uint16_t limit; } gdt, idt;
    uint64_t cr0, cr2, cr3, cr4, efer;
};

// VM-Exit reason returned by corevm_run_vcpu
// For IoIn/MmioRead: client writes response into the shared data buffer,
// backend writes it back to guest register state before next VM-entry.
enum VmExitReason {
    IoIn       { uint16_t port; uint8_t size; uint8_t *data; },   // write response here
    IoOut      { uint16_t port; uint8_t size; uint32_t data; },
    MmioRead   { uint64_t addr; uint8_t size; uint8_t *data; },   // write response here
    MmioWrite  { uint64_t addr; uint8_t size; uint64_t data; },
    MsrRead    { uint32_t index; uint64_t *value; },               // write response here
    MsrWrite   { uint32_t index; uint64_t value; },
    CpuidExit  { uint32_t function; uint32_t index; uint32_t *eax; uint32_t *ebx; uint32_t *ecx; uint32_t *edx; },
    Halted,
    InterruptWindow,
    Shutdown,           // Triple fault
    Debug,              // Single-step / hw breakpoint
    Error,
};
```

### 5.2 Functions

**VM Lifecycle:**
```c
uint64_t corevm_create(uint32_t ram_mb);
void     corevm_destroy(uint64_t handle);
int      corevm_reset(uint64_t handle);
```

**vCPU Management:**
```c
int corevm_create_vcpu(uint64_t handle, uint32_t vcpu_id);
int corevm_destroy_vcpu(uint64_t handle, uint32_t vcpu_id);
int corevm_run_vcpu(uint64_t handle, uint32_t vcpu_id, VmExitReason *exit);
int corevm_get_vcpu_regs(uint64_t handle, uint32_t vcpu_id, VcpuRegs *regs);
int corevm_set_vcpu_regs(uint64_t handle, uint32_t vcpu_id, const VcpuRegs *regs);
int corevm_get_vcpu_sregs(uint64_t handle, uint32_t vcpu_id, VcpuSregs *sregs);
int corevm_set_vcpu_sregs(uint64_t handle, uint32_t vcpu_id, const VcpuSregs *sregs);
int corevm_inject_interrupt(uint64_t handle, uint32_t vcpu_id, uint8_t vector);
int corevm_inject_exception(uint64_t handle, uint32_t vcpu_id, uint8_t vector, int has_error_code, uint32_t error_code);
int corevm_inject_nmi(uint64_t handle, uint32_t vcpu_id);
int corevm_request_interrupt_window(uint64_t handle, uint32_t vcpu_id, int enable);
int corevm_set_cpuid(uint64_t handle, const CpuidEntry *entries, uint32_t count);
```

**Memory:**
```c
int  corevm_set_memory_region(uint64_t handle, uint32_t slot, uint64_t guest_phys, uint64_t size, void *host_ptr);
int  corevm_read_phys(uint64_t handle, uint64_t addr, void *buf, uint32_t len);
int  corevm_write_phys(uint64_t handle, uint64_t addr, const void *buf, uint32_t len);
int  corevm_load_binary(uint64_t handle, uint64_t guest_phys, const void *data, uint32_t len);
```

**Devices (retained, signatures unchanged):**
```c
int  corevm_setup_standard_devices(uint64_t handle);
int  corevm_setup_e1000(uint64_t handle, ...);
int  corevm_setup_ahci(uint64_t handle);
int  corevm_ahci_attach_disk(uint64_t handle, uint32_t port, const char *path);
int  corevm_ahci_attach_cdrom(uint64_t handle, uint32_t port, const char *path);
```

**I/O Exit Dispatch:**
```c
int corevm_handle_io_exit(uint64_t handle, uint16_t port, uint8_t direction, uint8_t size, void *data);
int corevm_handle_mmio_exit(uint64_t handle, uint64_t addr, uint8_t direction, uint8_t size, void *data);
```

**Query:**
```c
int corevm_has_hw_support(void);  // Returns 1 if VT-x/AMD-V/KVM/WHP available
```

## 6. VM-Exit Handling Flow

The client run-loop replaces the old batch-instruction model:

```
loop {
    // Check if devices have pending interrupts or exceptions
    if let Some(pending) = device_manager.pending_event() {
        match pending {
            IrqEvent::Interrupt(vector) => corevm_inject_interrupt(vm, 0, vector),
            IrqEvent::Exception(vector, err) => corevm_inject_exception(vm, 0, vector, err),
            IrqEvent::Nmi => corevm_inject_nmi(vm, 0),
        }
    }

    // Enter guest
    let exit = corevm_run_vcpu(vm, 0);

    match exit {
        IoOut { port, size, data } => {
            corevm_handle_io_exit(vm, port, IO_OUT, size, &data);
        }
        IoIn { port, size, data } => {
            // handle_io_exit writes result into data buffer;
            // backend copies it to guest EAX before next VM-entry
            corevm_handle_io_exit(vm, port, IO_IN, size, data);
        }
        MmioWrite { addr, size, data } => {
            corevm_handle_mmio_exit(vm, addr, IO_OUT, size, &data);
        }
        MmioRead { addr, size, data } => {
            // handle_mmio_exit writes result into data buffer;
            // backend copies it to guest destination register before next VM-entry
            corevm_handle_mmio_exit(vm, addr, IO_IN, size, data);
        }
        MsrRead { index, value } => {
            *value = handle_msr_read(vm, index);
        }
        MsrWrite { index, value } => {
            handle_msr_write(vm, index, value);
        }
        CpuidExit { function, index, eax, ebx, ecx, edx } => {
            cpuid_filter(function, index, eax, ebx, ecx, edx);
        }
        InterruptWindow => {
            // Guest is ready for interrupt injection, loop back
        }
        Halted => {
            // Wait for next interrupt, then inject and resume
            wait_for_device_irq();
        }
        Shutdown => break,  // Triple fault
        Error => break,
    }
}
```

MMIO regions (LAPIC at 0xFEE00000, IOAPIC at 0xFEC00000, device BARs) are left unmapped in EPT/NPT so that accesses cause VM-Exits.

## 7. Platform Backends

### 7.1 KVM Backend (Linux)

Uses `/dev/kvm` via `ioctl()`:

| Operation | ioctl |
|-----------|-------|
| Create VM | `KVM_CREATE_VM` |
| Set memory | `KVM_SET_USER_MEMORY_REGION` |
| Create vCPU | `KVM_CREATE_VCPU` |
| Run vCPU | `KVM_RUN` (shared `kvm_run` page) |
| Get/set regs | `KVM_GET_REGS` / `KVM_SET_REGS` |
| Get/set sregs | `KVM_GET_SREGS` / `KVM_SET_SREGS` |
| Inject IRQ | `KVM_INTERRUPT` |

Guest RAM is allocated via `mmap(MAP_ANONYMOUS)` and registered with `KVM_SET_USER_MEMORY_REGION`. The `kvm_run` shared page contains exit reason, I/O details, and MMIO data.

### 7.2 WHP Backend (Windows)

Uses Windows Hypervisor Platform API:

| Operation | WHP Function |
|-----------|-------------|
| Create partition | `WHvCreatePartition` + `WHvSetupPartition` |
| Set memory | `WHvMapGpaRange` |
| Create vCPU | `WHvCreateVirtualProcessor` |
| Run vCPU | `WHvRunVirtualProcessor` |
| Get/set regs | `WHvGetVirtualProcessorRegisters` / `WHvSetVirtualProcessorRegisters` |
| Inject IRQ | `WHvRequestInterrupt` |

Partition properties: `WHvPartitionPropertyCodeProcessorCount = 1`, enable extended VM exits for I/O and MMIO.

### 7.3 anyOS Backend

Uses new kernel syscalls (see Section 8). The anyOS backend directly maps the `VmBackend` trait calls to syscalls. Guest RAM is allocated in userspace and registered via `sys_vm_set_memory`.

## 8. anyOS Kernel: Virtualization Module

New module at `kernel/src/arch/x86/virt/`.

### 8.1 File Structure

```
kernel/src/arch/x86/virt/
├── mod.rs          — Feature detection, VirtType enum (Vmx/Svm/None)
├── vmx.rs          — Intel VT-x: VMXON, VMCS, VM-Entry/Exit
├── svm.rs          — AMD-V: SVME, VMCB, VMRUN
├── ept.rs          — EPT (Intel) / NPT (AMD) page table management
└── syscalls.rs     — Syscall handlers for VM operations
```

### 8.2 Feature Detection (`mod.rs`)

At boot:
1. Check `CPUID.1:ECX[5]` for VMX (Intel)
2. Check `CPUID.0x80000001:ECX[2]` for SVM (AMD)
3. Store result in global `VIRT_TYPE: VirtType`
4. If VMX: verify `IA32_FEATURE_CONTROL` MSR allows VMXON
5. If SVM: verify `VM_CR` MSR allows SVM

### 8.3 Intel VMX (`vmx.rs`)

**Per-CPU Init (called during SMP startup):**
1. Set CR4.VMXE (bit 13)
2. Allocate 4KB-aligned VMXON region, write VMCS revision ID
3. Execute `VMXON`

**Per-VM VMCS Setup:**
1. Allocate 4KB-aligned VMCS, `VMCLEAR`, `VMPTRLD`
2. `VMWRITE` guest state: RIP, RSP, RFLAGS, segments (Real Mode defaults: CS.base=0xF000, IP=0xFFF0)
3. `VMWRITE` host state: CR3 (kernel page table), RIP (= `vmexit_handler`), RSP (kernel stack), segments
4. Pin-based controls: External-interrupt exiting
5. Primary proc-based controls: HLT exiting, I/O exiting (unconditional), use MSR bitmaps
6. Secondary proc-based controls: Enable EPT, unrestricted guest (for Real Mode)
7. VM-entry controls: 64-bit host
8. VM-exit controls: 64-bit host, save/load EFER

**VM-Exit Handler (`vmexit_handler`, naked asm):**
1. Push all guest GPRs to stack
2. Read exit reason from VMCS (`VMREAD`)
3. Translate to `VmExitReason`
4. Store in per-vCPU struct
5. Restore kernel GPRs
6. Return to syscall handler

**VM-Entry:**
1. Load guest GPRs from per-vCPU struct
2. If first entry: `VMLAUNCH`; else: `VMRESUME`
3. On VM-Exit: jumps to `vmexit_handler`

### 8.4 AMD SVM (`svm.rs`)

**Per-CPU Init:**
1. Set EFER.SVME (bit 12) via `wrmsr`
2. Allocate Host Save Area (4KB), write to `VM_HSAVE_PA` MSR

**Per-VM VMCB Setup:**
1. Allocate 4KB-aligned VMCB
2. Control area: intercept I/O, HLT, MSR reads/writes
3. State save area: guest RIP, RSP, RFLAGS, segments, CR0/3/4, EFER
4. Enable Nested Paging (NPT): set control bit, load nCR3

**VMRUN:**
1. Load VMCB physical address into RAX
2. `VMRUN` — hardware saves host state, loads guest state, executes guest
3. On intercept: hardware saves guest state, loads host state, execution continues after `VMRUN`
4. Read intercept code from VMCB control area

### 8.5 EPT / NPT Page Tables (`ept.rs`)

Page table management for both Intel EPT and AMD NPT. Both use 4-level page tables mapping Guest Physical Address (GPA) → Host Physical Address (HPA), but with different entry formats.

**Intel EPT entry format (per SDM Vol 3, Table 29-1):**
```
Bits [2:0]   — Read / Write / Execute
Bits [5:3]   — EPT memory type (leaf entries only: 0=UC, 6=WB)
Bit  [6]     — Ignore PAT (leaf only)
Bit  [7]     — Page size (1=2MB/1GB large page)
Bit  [8]     — Accessed
Bit  [9]     — Dirty (leaf only)
Bits [11:10] — Ignored
Bits [51:12] — Physical address
```

**AMD NPT entry format (standard AMD64 page table format):**
```
Bit  [0]     — Present
Bit  [1]     — Read/Write
Bit  [2]     — User/Supervisor
Bits [4:3]   — PWT, PCD
Bit  [5]     — Accessed
Bit  [6]     — Dirty (leaf only)
Bit  [7]     — Page size
Bits [51:12] — Physical address
Bit  [63]    — No-Execute
```

The `ept.rs` module provides a `GuestPageTable` trait with separate `EptPageTable` and `NptPageTable` implementations to handle these format differences.

Operations:
- `map_page(gpa, hpa, perms)` — Add mapping
- `unmap_page(gpa)` — Remove mapping (used for MMIO holes)
- `map_range(gpa_start, hpa_start, size, perms)` — Bulk mapping for RAM regions

MMIO regions are explicitly unmapped so guest accesses cause EPT violations (Intel) / NPT faults (AMD), which trigger VM-Exits.

### 8.6 Syscalls (`syscalls.rs`)

| Syscall | Parameters | Description |
|---------|-----------|-------------|
| `sys_vm_create` | — | Allocate VM struct, init EPT/NPT root |
| `sys_vm_destroy` | vm_id | Tear down EPT/NPT, free VMCS/VMCB |
| `sys_vm_set_memory` | vm_id, slot, gpa, size, uva | Translate UVA→HPA, create EPT/NPT mappings |
| `sys_vcpu_create` | vm_id, vcpu_id | Allocate VMCS/VMCB, init guest state |
| `sys_vcpu_run` | vm_id, vcpu_id, *exit_info | VMLAUNCH/VMRESUME or VMRUN, copy exit reason to userspace |
| `sys_vcpu_get_regs` | vm_id, vcpu_id, *regs | Read GPRs from VMCS/VMCB |
| `sys_vcpu_set_regs` | vm_id, vcpu_id, *regs | Write GPRs to VMCS/VMCB |
| `sys_vcpu_get_sregs` | vm_id, vcpu_id, *sregs | Read segment/control regs |
| `sys_vcpu_set_sregs` | vm_id, vcpu_id, *sregs | Write segment/control regs |
| `sys_vcpu_inject_irq` | vm_id, vcpu_id, vector | Inject external interrupt |
| `sys_vcpu_inject_exception` | vm_id, vcpu_id, vector, has_err, err_code | Inject exception (with optional error code) |
| `sys_vcpu_inject_nmi` | vm_id, vcpu_id | Inject NMI |
| `sys_vm_set_cpuid` | vm_id, *entries, count | Set CPUID filter table |

## 9. Client Adaptations

### 9.1 libcorevm_client

Complete rewrite to match new FFI API. New types:
- `VmHandle` — wraps `u64`, RAII (calls `corevm_destroy` on drop)
- `VcpuHandle` — wraps `(&VmHandle, u32 vcpu_id)` (borrows VM, does not own it)
- `VcpuRegs`, `VcpuSregs`, `VmExitReason` — Rust mirrors of C structs

### 9.2 bin/vmd

Adapt run-loop from batch-instruction to VM-Exit model:
- Remove instruction-count-based batching
- Replace with: `run_vcpu` → match exit → handle I/O/MMIO → loop
- IPC remains unchanged (command/status pipes, SHM framebuffer)
- Remove JIT toggle from command handling

### 9.3 apps/vmmanager (anyOS)

- Update VM config: remove JIT toggle
- Adapt to new `libcorevm_client` API
- Rest unchanged (UI, IPC, SHM)

### 9.4 corevm/vmmanager (Linux/Windows)

- Switch from `host_test` feature to `linux` / `windows` feature
- Adapt to new API (create_vcpu, run_vcpu exit loop)
- Remove JIT-related UI elements

## 10. Build Configuration

```toml
# libs/libcorevm/Cargo.toml
[features]
default = []
anyos = []           # no_std, direct VMX/SVM via kernel syscalls
linux = ["std"]      # KVM backend
windows = ["std"]    # WHP backend
std = []

[lib]
crate-type = ["staticlib", "rlib"]
```

Only one platform feature is active per build. The `VmBackend` implementation is selected at compile time via `#[cfg(feature = "...")]`.

## 11. SMP Readiness

While initially single-vCPU, all data structures support multiple vCPUs:

```rust
struct Vm {
    backend: Box<dyn VmBackend>,
    vcpus: Vec<Vcpu>,          // index = vcpu_id
    devices: DeviceManager,
    ram: GuestMemory,
}
```

Future SMP: each vCPU gets its own thread calling `run_vcpu(vm, vcpu_id)`. Device access synchronized via mutex. LAPIC per-vCPU, IOAPIC routes to specific LAPIC by destination ID.

## 12. Error Handling

```rust
pub enum VmError {
    NoHardwareSupport,      // CPU lacks VT-x/AMD-V
    VmxInitFailed,          // VMXON failed (e.g., locked by BIOS)
    SvmInitFailed,          // SVM enable failed
    InvalidVcpuId,
    MemoryMapFailed,
    VmEntryFailed(u32),     // VM-entry failure reason code
    BackendError(i32),      // OS-specific error code
}
```

`corevm_has_hw_support()` allows clients to check before attempting VM creation and show a user-friendly error message.

## 13. CPUID Filtering

Guest CPUID results must be controlled to hide features the device model doesn't support (e.g., XSAVE, AVX-512, BMI). At VM creation, libcorevm builds a default CPUID table based on host CPUID with unsafe features masked out. Clients can override via `corevm_set_cpuid()`.

Per backend:
- **KVM:** `KVM_SET_CPUID2` ioctl with array of `kvm_cpuid_entry2`
- **WHP:** `WHvSetPartitionProperty` with `WHvPartitionPropertyCodeCpuidResultList`
- **anyOS:** VMCS/VMCB intercept CPUID; kernel returns filtered results from per-VM table

Default masks applied (matching previous libcorevm behavior):
- Remove XSAVE/OSXSAVE (CPUID.1:ECX bits 26-27)
- Remove BMI1/BMI2 (CPUID.7:EBX bits 3,8)
- Remove AVX-512 family
- Keep SSE/SSE2/SSE3/SSSE3/SSE4.1/SSE4.2
- Advertise RDRAND (CPUID.1:ECX bit 30) — handled by hardware natively

## 14. MSR Interception

MSR bitmap controls which MSR reads/writes cause VM-Exits. By default, most MSRs are handled by hardware (passthrough). Intercepted MSRs:

- **IA32_APIC_BASE (0x1B):** Tracks LAPIC base address changes for MMIO remapping
- **x2APIC MSRs (0x800-0x8FF):** Routed to software LAPIC model if x2APIC mode active
- **IA32_TSC_DEADLINE (0x6E0):** Routed to LAPIC timer model

All other MSRs (EFER, STAR, LSTAR, etc.) are handled natively by VMX/SVM hardware.

Per backend:
- **KVM:** `KVM_SET_MSR_FILTER` or per-vCPU `KVM_SET_MSRS`
- **WHP:** MSR access causes `WHvRunVpExitReasonX64MsrAccess` exit
- **anyOS:** MSR bitmap in VMCS/VMCB controls which MSRs trap

## 15. TSC Handling

With hardware virtualization, the guest TSC runs natively on the physical CPU. This is correct behavior — no offset or scaling needed by default. The LAPIC timer model already uses TSC-based timing (see MEMORY.md bug #16 fix), which maps directly to hardware TSC.

If needed in the future (e.g., live migration, TSC frequency mismatch), backends support TSC offsetting:
- **VMX:** VMCS TSC-offset field
- **SVM:** VMCB TSC_OFFSET
- **KVM:** `KVM_SET_TSC_KHZ`
- **WHP:** `WHvX64RegisterTsc`

## 16. Allocation Strategy

The `no_std` core uses `alloc` (Vec, Box) which requires a global allocator. This is already available in anyOS userspace via the existing allocator. The `anyos` feature backend uses the same allocation path as existing libcorevm code.

For the `Vm` struct with `Box<dyn VmBackend>`: on anyOS, the concrete backend is selected at compile time via `#[cfg]`, so dynamic dispatch is replaced with a type alias (`type Backend = AnyOsBackend`) avoiding the need for `dyn`. On Linux/Windows with `std`, `Box<dyn VmBackend>` works normally.

## 17. FPU / SSE / XSAVE State

With hardware virtualization, FPU/SSE state is managed transparently by the hardware:
- **VMX:** VMCS controls auto-save/restore of x87, SSE, and XSAVE state on VM-entry/exit
- **SVM:** VMCB has dedicated save area; VMRUN/VMEXIT auto-swap FPU context
- **KVM/WHP:** handle FPU context switching internally

No explicit FPU register accessors are needed in the initial API since the hardware handles save/restore. If future features require FPU state inspection (debugging, migration), accessors can be added:
```c
int corevm_get_fpu_state(uint64_t handle, uint32_t vcpu_id, void *fxsave_area, uint32_t size);
int corevm_set_fpu_state(uint64_t handle, uint32_t vcpu_id, const void *fxsave_area, uint32_t size);
```
These would map to `KVM_GET_FPU`/`KVM_SET_FPU`, `WHvGetVirtualProcessorRegisters` (FP registers), or direct VMCS/VMCB access on anyOS.
