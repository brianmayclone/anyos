use libcorevm::cpu::{Cpu, Mode};
use libcorevm::decoder::{CpuMode, Decoder};
use libcorevm::executor;
use libcorevm::flags::RFLAGS_FIXED;
use libcorevm::flags;
use libcorevm::interrupts::InterruptController;
use libcorevm::io::IoDispatch;
use libcorevm::memory::{GuestMemory, MemoryBus, Mmu};
use libcorevm::registers::{
    GprIndex, MSR_IA32_APIC_BASE, MSR_IA32_SYSENTER_CS, MSR_IA32_SYSENTER_EIP,
    MSR_IA32_SYSENTER_ESP, MSR_TSC, SegReg, CR0_PE,
};
use libcorevm::registers::CR0_PG;
use libcorevm::{corevm_create_ex, corevm_destroy, corevm_get_vcpu_count};

fn run_one(cpu: &mut Cpu, mmu: &mut Mmu, mem: &mut GuestMemory, bytes: &[u8]) {
    mem.load_at(0, bytes);
    cpu.regs.rip = 0;
    let inst = cpu.decoder.decode(mem, 0).expect("decode");
    let mut io = IoDispatch::new();
    let mut ints = InterruptController::new();
    executor::execute(cpu, &inst, mem, mmu, &mut io, &mut ints).expect("execute");
}

fn lane_from_bytes(bytes: [u8; 8]) -> u64 {
    u64::from_le_bytes(bytes)
}

fn crc32_update(mut crc: u32, val: u64, bytes: usize) -> u32 {
    for i in 0..bytes {
        crc ^= ((val >> (i * 8)) as u8) as u32;
        for _ in 0..8 {
            if (crc & 1) != 0 {
                crc = (crc >> 1) ^ 0x82F6_3B78;
            } else {
                crc >>= 1;
            }
        }
    }
    crc
}

fn make_seg_desc(base: u32, limit: u32, access: u8, flags: u8) -> u64 {
    let limit_low = (limit & 0xFFFF) as u64;
    let limit_high = ((limit >> 16) & 0x0F) as u64;
    let base_low = (base & 0xFFFF) as u64;
    let base_mid = ((base >> 16) & 0xFF) as u64;
    let base_high = ((base >> 24) & 0xFF) as u64;
    limit_low
        | (base_low << 16)
        | ((base_mid as u64) << 32)
        | ((access as u64) << 40)
        | (limit_high << 48)
        | (((flags & 0x0F) as u64) << 52)
        | (base_high << 56)
}

fn make_idt32_interrupt_gate(offset: u32, selector: u16) -> u64 {
    let off_lo = (offset & 0xFFFF) as u64;
    let off_hi = ((offset >> 16) & 0xFFFF) as u64;
    // type_attr: P=1, DPL=0, type=0xE (32-bit interrupt gate)
    let type_attr = 0x8Eu64;
    off_lo | ((selector as u64) << 16) | (type_attr << 40) | (off_hi << 48)
}

fn make_idt16_interrupt_gate(offset: u16, selector: u16) -> u64 {
    let off_lo = offset as u64;
    let off_hi = 0u64;
    // type_attr: P=1, DPL=0, type=0x6 (16-bit interrupt gate)
    let type_attr = 0x86u64;
    off_lo | ((selector as u64) << 16) | (type_attr << 40) | (off_hi << 48)
}

#[test]
fn cpuid_leaf1_reports_boot_critical_features() {
    let mut cpu = Cpu::new();
    cpu.mode = Mode::ProtectedMode;
    cpu.decoder = Decoder::new(CpuMode::Protected32);
    cpu.regs.write_gpr32(GprIndex::Rax as u8, 1);

    let mut mmu = Mmu::new();
    let mut mem = GuestMemory::new(0x4000);
    run_one(&mut cpu, &mut mmu, &mut mem, &[0x0F, 0xA2]); // CPUID

    let ecx = cpu.regs.read_gpr32(GprIndex::Rcx as u8);
    assert_ne!(ecx & (1 << 13), 0, "CMPXCHG16B missing");
    assert_ne!(ecx & (1 << 9), 0, "SSSE3 missing");
}

#[test]
fn cpuid_leaf1_reports_configured_logical_cpu_count() {
    let mut cpu = Cpu::new();
    cpu.configure_topology(0, 4);
    cpu.mode = Mode::ProtectedMode;
    cpu.decoder = Decoder::new(CpuMode::Protected32);
    cpu.regs.write_gpr32(GprIndex::Rax as u8, 1);

    let mut mmu = Mmu::new();
    let mut mem = GuestMemory::new(0x4000);
    run_one(&mut cpu, &mut mmu, &mut mem, &[0x0F, 0xA2]); // CPUID

    let ebx = cpu.regs.read_gpr32(GprIndex::Rbx as u8);
    let logical = (ebx >> 16) & 0xFF;
    let edx = cpu.regs.read_gpr32(GprIndex::Rdx as u8);
    assert_eq!(logical, 4);
    assert_ne!(edx & (1 << 28), 0, "HTT bit should be set for SMP topology");
}

#[test]
fn corevm_create_ex_persists_vcpu_count() {
    let h = corevm_create_ex(64, 4);
    assert_ne!(h, 0);
    let cores = corevm_get_vcpu_count(h);
    assert_eq!(cores, 4);
    corevm_destroy(h);
}

#[test]
fn ia32_apic_base_msr_is_initialized() {
    let cpu = Cpu::new();
    assert_eq!(cpu.regs.read_msr(MSR_IA32_APIC_BASE), 0xFEE0_0900);
}

#[test]
fn xsetbv_then_xgetbv_roundtrip() {
    let mut cpu = Cpu::new();
    cpu.mode = Mode::LongMode;
    cpu.decoder = Decoder::new(CpuMode::Long64);
    cpu.regs.cpl = 0;

    let mut mmu = Mmu::new();
    let mut mem = GuestMemory::new(0x4000);

    cpu.regs.write_gpr32(GprIndex::Rcx as u8, 0);
    cpu.regs.write_gpr32(GprIndex::Rax as u8, 0x3);
    cpu.regs.write_gpr32(GprIndex::Rdx as u8, 0);
    run_one(&mut cpu, &mut mmu, &mut mem, &[0x0F, 0x01, 0xD1]); // XSETBV

    cpu.regs.write_gpr32(GprIndex::Rcx as u8, 0);
    run_one(&mut cpu, &mut mmu, &mut mem, &[0x0F, 0x01, 0xD0]); // XGETBV
    assert_eq!(cpu.regs.read_gpr32(GprIndex::Rax as u8), 0x3);
}

#[test]
fn rdtscp_reads_tsc_and_advances() {
    let mut cpu = Cpu::new();
    cpu.mode = Mode::LongMode;
    cpu.decoder = Decoder::new(CpuMode::Long64);
    cpu.regs.write_msr(MSR_TSC, 0x1122_3344_5566_7788);

    let mut mmu = Mmu::new();
    let mut mem = GuestMemory::new(0x4000);
    run_one(&mut cpu, &mut mmu, &mut mem, &[0x0F, 0x01, 0xF9]); // RDTSCP

    assert_eq!(cpu.regs.read_gpr32(GprIndex::Rax as u8), 0x5566_7788);
    assert_eq!(cpu.regs.read_gpr32(GprIndex::Rdx as u8), 0x1122_3344);
}

#[test]
fn cmpxchg8b_updates_memory_on_match() {
    let mut cpu = Cpu::new();
    cpu.mode = Mode::ProtectedMode;
    cpu.decoder = Decoder::new(CpuMode::Protected32);

    let mut mmu = Mmu::new();
    let mut mem = GuestMemory::new(0x4000);
    mem.load_at(0x100, &0xAABB_CCDD_1122_3344u64.to_le_bytes());

    cpu.regs.write_gpr32(GprIndex::Rsi as u8, 0x100);
    cpu.regs.write_gpr32(GprIndex::Rax as u8, 0x1122_3344);
    cpu.regs.write_gpr32(GprIndex::Rdx as u8, 0xAABB_CCDD);
    cpu.regs.write_gpr32(GprIndex::Rbx as u8, 0x5566_7788);
    cpu.regs.write_gpr32(GprIndex::Rcx as u8, 0xEEFF_0011);

    run_one(&mut cpu, &mut mmu, &mut mem, &[0x0F, 0xC7, 0x0E]); // CMPXCHG8B [RSI]

    assert_eq!(mem.read_u64(0x100).unwrap(), 0xEEFF_0011_5566_7788);
    assert_ne!(cpu.regs.rflags & flags::ZF, 0);
}

#[test]
fn cmpxchg16b_updates_memory_on_match() {
    let mut cpu = Cpu::new();
    cpu.mode = Mode::LongMode;
    cpu.decoder = Decoder::new(CpuMode::Long64);

    let mut mmu = Mmu::new();
    let mut mem = GuestMemory::new(0x4000);
    mem.load_at(0x100, &0x1122_3344_5566_7788u64.to_le_bytes());
    mem.load_at(0x108, &0x99AA_BBCC_DDEE_FF00u64.to_le_bytes());

    cpu.regs.write_gpr64(GprIndex::Rsi as u8, 0x100);
    cpu.regs.write_gpr64(GprIndex::Rax as u8, 0x1122_3344_5566_7788);
    cpu.regs.write_gpr64(GprIndex::Rdx as u8, 0x99AA_BBCC_DDEE_FF00);
    cpu.regs.write_gpr64(GprIndex::Rbx as u8, 0x0123_4567_89AB_CDEF);
    cpu.regs.write_gpr64(GprIndex::Rcx as u8, 0x0FED_CBA9_8765_4321);

    run_one(&mut cpu, &mut mmu, &mut mem, &[0x48, 0x0F, 0xC7, 0x0E]); // CMPXCHG16B [RSI]

    assert_eq!(mem.read_u64(0x100).unwrap(), 0x0123_4567_89AB_CDEF);
    assert_eq!(mem.read_u64(0x108).unwrap(), 0x0FED_CBA9_8765_4321);
    assert_ne!(cpu.regs.rflags & flags::ZF, 0);
}

#[test]
fn multibyte_nop_decodes_and_executes() {
    let mut cpu = Cpu::new();
    cpu.mode = Mode::LongMode;
    cpu.decoder = Decoder::new(CpuMode::Long64);
    cpu.regs.write_gpr64(GprIndex::Rax as u8, 0x100);

    let mut mmu = Mmu::new();
    let mut mem = GuestMemory::new(0x4000);
    run_one(&mut cpu, &mut mmu, &mut mem, &[0x0F, 0x1F, 0x00]); // NOP dword ptr [RAX]

    assert_eq!(cpu.regs.rip, 3);
}

#[test]
fn fxsave_fxrstor_roundtrip() {
    let mut cpu = Cpu::new();
    cpu.mode = Mode::LongMode;
    cpu.decoder = Decoder::new(CpuMode::Long64);

    let mut mmu = Mmu::new();
    let mut mem = GuestMemory::new(0x8000);

    cpu.regs.write_gpr64(GprIndex::Rsi as u8, 0x400);
    cpu.fpu.fcw = 0x1234;
    cpu.fpu.fsw = 0x5678;
    cpu.sse.mxcsr = 0x1F80;
    cpu.sse.xmm[0].lo = 0xDEAD_BEEF_CAFE_BABE;
    cpu.sse.xmm[0].hi = 0x0123_4567_89AB_CDEF;

    run_one(&mut cpu, &mut mmu, &mut mem, &[0x0F, 0xAE, 0x06]); // FXSAVE [RSI]

    cpu.fpu.fcw = 0;
    cpu.fpu.fsw = 0;
    cpu.sse.mxcsr = 0;
    cpu.sse.xmm[0].lo = 0;
    cpu.sse.xmm[0].hi = 0;

    run_one(&mut cpu, &mut mmu, &mut mem, &[0x0F, 0xAE, 0x0E]); // FXRSTOR [RSI]

    assert_eq!(cpu.fpu.fcw, 0x1234);
    assert_eq!(cpu.fpu.fsw, 0x5678);
    assert_eq!(cpu.sse.xmm[0].lo, 0xDEAD_BEEF_CAFE_BABE);
    assert_eq!(cpu.sse.xmm[0].hi, 0x0123_4567_89AB_CDEF);
}

#[test]
fn crc32_r32_rm8_executes() {
    let mut cpu = Cpu::new();
    cpu.mode = Mode::ProtectedMode;
    cpu.decoder = Decoder::new(CpuMode::Protected32);
    cpu.regs.write_gpr32(GprIndex::Rax as u8, 0x1234_5678);
    cpu.regs.write_gpr8(GprIndex::Rcx as u8, false, 0xA5);

    let mut mmu = Mmu::new();
    let mut mem = GuestMemory::new(0x4000);
    run_one(&mut cpu, &mut mmu, &mut mem, &[0xF2, 0x0F, 0x38, 0xF0, 0xC1]); // CRC32 EAX, CL

    let expected = crc32_update(0x1234_5678, 0xA5, 1);
    assert_eq!(cpu.regs.read_gpr32(GprIndex::Rax as u8), expected);
}

#[test]
fn crc32_r64_rm64_executes() {
    let mut cpu = Cpu::new();
    cpu.mode = Mode::LongMode;
    cpu.decoder = Decoder::new(CpuMode::Long64);
    cpu.regs.write_gpr64(GprIndex::Rax as u8, 0x89AB_CDEF);
    cpu.regs.write_gpr64(GprIndex::Rcx as u8, 0x1122_3344_5566_7788);

    let mut mmu = Mmu::new();
    let mut mem = GuestMemory::new(0x4000);
    run_one(&mut cpu, &mut mmu, &mut mem, &[0xF2, 0x48, 0x0F, 0x38, 0xF1, 0xC1]); // CRC32 RAX, RCX

    let expected = crc32_update(0x89AB_CDEF, 0x1122_3344_5566_7788, 8);
    assert_eq!(cpu.regs.read_gpr64(GprIndex::Rax as u8), expected as u64);
}

#[test]
fn pshufb_executes() {
    let mut cpu = Cpu::new();
    cpu.mode = Mode::LongMode;
    cpu.decoder = Decoder::new(CpuMode::Long64);

    cpu.sse.xmm[1].lo = lane_from_bytes([10, 11, 12, 13, 14, 15, 16, 17]);
    cpu.sse.xmm[1].hi = lane_from_bytes([20, 21, 22, 23, 24, 25, 26, 27]);
    cpu.sse.xmm[0].lo = lane_from_bytes([0, 1, 2, 3, 4, 5, 6, 7]);
    cpu.sse.xmm[0].hi = lane_from_bytes([7, 6, 5, 4, 3, 2, 1, 0]);

    let mut mmu = Mmu::new();
    let mut mem = GuestMemory::new(0x4000);
    run_one(&mut cpu, &mut mmu, &mut mem, &[0x66, 0x0F, 0x38, 0x00, 0xC1]); // PSHUFB XMM0, XMM1

    assert_eq!(cpu.sse.xmm[0].lo, lane_from_bytes([10, 11, 12, 13, 14, 15, 16, 17]));
    assert_eq!(cpu.sse.xmm[0].hi, lane_from_bytes([27, 26, 25, 24, 23, 22, 21, 20]));
}

#[test]
fn palignr_executes() {
    let mut cpu = Cpu::new();
    cpu.mode = Mode::LongMode;
    cpu.decoder = Decoder::new(CpuMode::Long64);

    cpu.sse.xmm[0].lo = lane_from_bytes([0, 1, 2, 3, 4, 5, 6, 7]);
    cpu.sse.xmm[0].hi = lane_from_bytes([8, 9, 10, 11, 12, 13, 14, 15]);
    cpu.sse.xmm[1].lo = lane_from_bytes([16, 17, 18, 19, 20, 21, 22, 23]);
    cpu.sse.xmm[1].hi = lane_from_bytes([24, 25, 26, 27, 28, 29, 30, 31]);

    let mut mmu = Mmu::new();
    let mut mem = GuestMemory::new(0x4000);
    run_one(
        &mut cpu,
        &mut mmu,
        &mut mem,
        &[0x66, 0x0F, 0x3A, 0x0F, 0xC1, 0x08], // PALIGNR XMM0, XMM1, 8
    );

    assert_eq!(cpu.sse.xmm[0].lo, lane_from_bytes([8, 9, 10, 11, 12, 13, 14, 15]));
    assert_eq!(cpu.sse.xmm[0].hi, lane_from_bytes([16, 17, 18, 19, 20, 21, 22, 23]));
}

#[test]
fn sysenter_32_transitions_to_ring0() {
    let mut cpu = Cpu::new();
    cpu.mode = Mode::ProtectedMode;
    cpu.decoder = Decoder::new(CpuMode::Protected32);
    cpu.regs.cpl = 3;
    cpu.regs.write_msr(MSR_IA32_SYSENTER_CS, 0x10);
    cpu.regs.write_msr(MSR_IA32_SYSENTER_EIP, 0x1234_5678);
    cpu.regs.write_msr(MSR_IA32_SYSENTER_ESP, 0x00AB_CDEF);

    let mut mmu = Mmu::new();
    let mut mem = GuestMemory::new(0x4000);
    run_one(&mut cpu, &mut mmu, &mut mem, &[0x0F, 0x34]); // SYSENTER

    assert_eq!(cpu.regs.cpl, 0);
    assert_eq!(cpu.regs.rip, 0x1234_5678);
    assert_eq!(cpu.regs.sp(), 0x00AB_CDEF);
    assert_eq!(cpu.regs.segment(libcorevm::registers::SegReg::Cs).selector, 0x10);
    assert_eq!(cpu.regs.segment(libcorevm::registers::SegReg::Ss).selector, 0x18);
}

#[test]
fn sysexit_32_returns_to_ring3() {
    let mut cpu = Cpu::new();
    cpu.mode = Mode::ProtectedMode;
    cpu.decoder = Decoder::new(CpuMode::Protected32);
    cpu.regs.cpl = 0;
    cpu.regs.write_msr(MSR_IA32_SYSENTER_CS, 0x10);
    cpu.regs.write_gpr32(GprIndex::Rdx as u8, 0x4000_1000);
    cpu.regs.write_gpr32(GprIndex::Rcx as u8, 0x7000_2000);

    let mut mmu = Mmu::new();
    let mut mem = GuestMemory::new(0x4000);
    run_one(&mut cpu, &mut mmu, &mut mem, &[0x0F, 0x35]); // SYSEXIT

    assert_eq!(cpu.regs.cpl, 3);
    assert_eq!(cpu.regs.rip, 0x4000_1000);
    assert_eq!(cpu.regs.sp(), 0x7000_2000);
    assert_eq!(cpu.regs.segment(libcorevm::registers::SegReg::Cs).selector, 0x23);
    assert_eq!(cpu.regs.segment(libcorevm::registers::SegReg::Ss).selector, 0x2B);
}

#[test]
fn sysexit_64_returns_to_ring3_with_rex_w() {
    let mut cpu = Cpu::new();
    cpu.mode = Mode::LongMode;
    cpu.decoder = Decoder::new(CpuMode::Long64);
    cpu.regs.cpl = 0;
    cpu.regs.write_msr(MSR_IA32_SYSENTER_CS, 0x10);
    cpu.regs.write_gpr64(GprIndex::Rdx as u8, 0xFFFF_8000_0000_1000);
    cpu.regs.write_gpr64(GprIndex::Rcx as u8, 0x0000_7FFF_FFFF_F000);

    let mut mmu = Mmu::new();
    let mut mem = GuestMemory::new(0x4000);
    run_one(&mut cpu, &mut mmu, &mut mem, &[0x48, 0x0F, 0x35]); // REX.W SYSEXIT

    assert_eq!(cpu.regs.cpl, 3);
    assert_eq!(cpu.regs.rip, 0xFFFF_8000_0000_1000);
    assert_eq!(cpu.regs.sp(), 0x0000_7FFF_FFFF_F000);
    assert_eq!(cpu.regs.segment(libcorevm::registers::SegReg::Cs).selector, 0x33);
    assert_eq!(cpu.regs.segment(libcorevm::registers::SegReg::Ss).selector, 0x3B);
}

#[test]
fn popcnt_r32_rm32_updates_flags() {
    let mut cpu = Cpu::new();
    cpu.mode = Mode::ProtectedMode;
    cpu.decoder = Decoder::new(CpuMode::Protected32);
    cpu.regs.write_gpr32(GprIndex::Rax as u8, 0);
    cpu.regs.write_gpr32(GprIndex::Rcx as u8, 0xF0F0_0001);
    cpu.regs.rflags = u64::MAX;

    let mut mmu = Mmu::new();
    let mut mem = GuestMemory::new(0x4000);
    run_one(&mut cpu, &mut mmu, &mut mem, &[0xF3, 0x0F, 0xB8, 0xC1]); // POPCNT EAX, ECX

    assert_eq!(cpu.regs.read_gpr32(GprIndex::Rax as u8), 9);
    assert_eq!(cpu.regs.rflags & flags::ZF, 0);
    assert_eq!(cpu.regs.rflags & flags::CF, 0);
    assert_eq!(cpu.regs.rflags & flags::OF, 0);
}

#[test]
fn popcnt_zero_sets_zf() {
    let mut cpu = Cpu::new();
    cpu.mode = Mode::ProtectedMode;
    cpu.decoder = Decoder::new(CpuMode::Protected32);
    cpu.regs.write_gpr32(GprIndex::Rcx as u8, 0);
    cpu.regs.rflags = 0;

    let mut mmu = Mmu::new();
    let mut mem = GuestMemory::new(0x4000);
    run_one(&mut cpu, &mut mmu, &mut mem, &[0xF3, 0x0F, 0xB8, 0xC1]); // POPCNT EAX, ECX

    assert_eq!(cpu.regs.read_gpr32(GprIndex::Rax as u8), 0);
    assert_ne!(cpu.regs.rflags & flags::ZF, 0);
}

#[test]
fn movbe_load_and_store_work() {
    let mut cpu = Cpu::new();
    cpu.mode = Mode::LongMode;
    cpu.decoder = Decoder::new(CpuMode::Long64);
    cpu.regs.write_gpr64(GprIndex::Rsi as u8, 0x120);
    cpu.regs.write_gpr32(GprIndex::Rbx as u8, 0x1122_3344);

    let mut mmu = Mmu::new();
    let mut mem = GuestMemory::new(0x4000);
    mem.load_at(0x120, &0xA1B2_C3D4u32.to_le_bytes());

    // MOVBE EAX, [RSI]
    run_one(&mut cpu, &mut mmu, &mut mem, &[0x0F, 0x38, 0xF0, 0x06]);
    assert_eq!(cpu.regs.read_gpr32(GprIndex::Rax as u8), 0xD4C3_B2A1);

    // MOVBE [RSI], EBX
    run_one(&mut cpu, &mut mmu, &mut mem, &[0x0F, 0x38, 0xF1, 0x1E]);
    assert_eq!(mem.read_u32(0x120).unwrap(), 0x4433_2211);
}

#[test]
fn protected_interrupt_cpl3_to_cpl0_uses_tss_stack() {
    let mut cpu = Cpu::new();
    cpu.mode = Mode::ProtectedMode;
    cpu.decoder = Decoder::new(CpuMode::Protected32);
    cpu.regs.cr0 = CR0_PE;
    cpu.regs.cpl = 3;
    cpu.regs.rip = 0x0040_0000;
    cpu.regs.rflags = RFLAGS_FIXED | flags::IF;
    cpu.regs.set_sp(0x0000_8000);

    let mut mmu = Mmu::new();
    let mut mem = GuestMemory::new(0x20_000);
    let mut ints = InterruptController::new();

    // GDT @ 0x1000: null, kcode(0x08), kdata(0x10), ucode(0x18), udata(0x20), tss(0x28)
    let gdt_base = 0x1000u64;
    mem.load_at((gdt_base + 0x00) as usize, &0u64.to_le_bytes());
    // 32-bit flat kernel code: access=0x9A, flags=0xC (G=1,D=1)
    mem.load_at((gdt_base + 0x08) as usize, &make_seg_desc(0, 0x000F_FFFF, 0x9A, 0xC).to_le_bytes());
    // 32-bit flat kernel data
    mem.load_at((gdt_base + 0x10) as usize, &make_seg_desc(0, 0x000F_FFFF, 0x92, 0xC).to_le_bytes());
    // 32-bit flat user code (DPL=3)
    mem.load_at((gdt_base + 0x18) as usize, &make_seg_desc(0, 0x000F_FFFF, 0xFA, 0xC).to_le_bytes());
    // 32-bit flat user data (DPL=3)
    mem.load_at((gdt_base + 0x20) as usize, &make_seg_desc(0, 0x000F_FFFF, 0xF2, 0xC).to_le_bytes());
    // 32-bit available TSS at 0x2000, limit 0x67: access=0x89, flags=0
    mem.load_at((gdt_base + 0x28) as usize, &make_seg_desc(0x2000, 0x67, 0x89, 0x0).to_le_bytes());

    cpu.regs.gdtr.base = gdt_base;
    cpu.regs.gdtr.limit = 0x2F;
    cpu.regs.idtr.base = 0x3000;
    cpu.regs.idtr.limit = 0x07FF;
    cpu.regs.tr = 0x28;

    // Current user segments
    cpu.load_segment_from_gdt(SegReg::Cs, 0x1B, &mem, &mmu).unwrap();
    cpu.load_segment_from_gdt(SegReg::Ss, 0x23, &mem, &mmu).unwrap();

    // IDT vector 14 -> kernel handler 0x0010_1000 in CS=0x08
    let idt_entry = make_idt32_interrupt_gate(0x0010_1000, 0x08);
    mem.load_at((0x3000 + 14 * 8) as usize, &idt_entry.to_le_bytes());

    // TSS ring0 stack: ESP0=0x9000, SS0=0x10
    mem.load_at(0x2004, &0x0000_9000u32.to_le_bytes()); // ESP0
    mem.load_at(0x2008, &0x0010u16.to_le_bytes()); // SS0

    cpu.deliver_interrupt(14, true, Some(0xDEAD_BEEFu32), &mut mem, &mut mmu, &mut ints)
        .unwrap();

    assert_eq!(cpu.regs.cpl, 0);
    assert_eq!(cpu.regs.segment(SegReg::Cs).selector, 0x08);
    assert_eq!(cpu.regs.segment(SegReg::Ss).selector, 0x10);
    assert_eq!(cpu.regs.rip, 0x0010_1000);

    // New ESP after pushing old SS, old ESP, EFLAGS, CS, EIP, error code
    let esp = cpu.regs.sp() as u32;
    assert_eq!(esp, 0x9000 - 24);
    assert_eq!(mem.read_u32(esp as u64).unwrap(), 0xDEAD_BEEF);
    assert_eq!(mem.read_u32((esp + 4) as u64).unwrap(), 0x0040_0000); // old EIP
    assert_eq!(mem.read_u32((esp + 8) as u64).unwrap(), 0x001B); // old CS
    assert_eq!(mem.read_u32((esp + 12) as u64).unwrap(), (RFLAGS_FIXED | flags::IF) as u32);
    assert_eq!(mem.read_u32((esp + 16) as u64).unwrap(), 0x0000_8000); // old ESP
    assert_eq!(mem.read_u32((esp + 20) as u64).unwrap(), 0x0023); // old SS
}

#[test]
fn protected_interrupt_16bit_gate_pushes_16bit_frame() {
    let mut cpu = Cpu::new();
    cpu.mode = Mode::ProtectedMode;
    cpu.decoder = Decoder::new(CpuMode::Protected32);
    cpu.regs.cr0 = CR0_PE;
    cpu.regs.cpl = 0;
    cpu.regs.rip = 0x0001_2345;
    cpu.regs.rflags = RFLAGS_FIXED | flags::IF;
    cpu.regs.set_sp(0x0000_8000);

    let mut mmu = Mmu::new();
    let mut mem = GuestMemory::new(0x20_000);
    let mut ints = InterruptController::new();

    // GDT: null, 16-bit code(0x08), 16-bit data(0x10)
    let gdt_base = 0x1000u64;
    mem.load_at((gdt_base + 0x00) as usize, &0u64.to_le_bytes());
    mem.load_at(
        (gdt_base + 0x08) as usize,
        &make_seg_desc(0, 0x0000_FFFF, 0x9A, 0x0).to_le_bytes(),
    );
    mem.load_at(
        (gdt_base + 0x10) as usize,
        &make_seg_desc(0, 0x0000_FFFF, 0x92, 0x0).to_le_bytes(),
    );
    cpu.regs.gdtr.base = gdt_base;
    cpu.regs.gdtr.limit = 0x17;
    cpu.regs.idtr.base = 0x3000;
    cpu.regs.idtr.limit = 0x07FF;
    cpu.load_segment_from_gdt(SegReg::Cs, 0x08, &mem, &mmu).unwrap();
    cpu.load_segment_from_gdt(SegReg::Ss, 0x10, &mem, &mmu).unwrap();

    // IDT vector 0x20 -> 16-bit handler 0x04D0 in CS=0x08.
    let idt_entry = make_idt16_interrupt_gate(0x04D0, 0x08);
    mem.load_at((0x3000 + 0x20 * 8) as usize, &idt_entry.to_le_bytes());

    cpu.deliver_interrupt(0x20, false, None, &mut mem, &mut mmu, &mut ints)
        .unwrap();

    assert_eq!(cpu.regs.rip, 0x04D0);
    assert_eq!(cpu.regs.sp(), 0x8000 - 6);
    let sp = cpu.regs.sp() as u64;
    assert_eq!(mem.read_u16(sp).unwrap(), 0x2345);
    assert_eq!(mem.read_u16(sp + 2).unwrap(), 0x0008);
    assert_eq!(
        mem.read_u16(sp + 4).unwrap(),
        (RFLAGS_FIXED as u16) | (flags::IF as u16)
    );
}

#[test]
fn mmu_translation_updates_when_cr3_changes() {
    let mut mmu = Mmu::new();
    let mut mem = GuestMemory::new(0x40_000);
    mmu.update_from_regs(CR0_PG, 0, 0);

    let linear = 0x0000_4000u64;
    let pte_index = ((linear >> 12) & 0x3FF) as u64;

    // CR3=A: PD@0x1000 -> PT@0x2000 -> linear page -> phys 0xA000
    mem.write_u32(0x1000, 0x2000 | 0x3).unwrap();
    mem.write_u32(0x2000 + pte_index * 4, 0xA000 | 0x3).unwrap();

    // CR3=B: PD@0x3000 -> PT@0x4000 -> linear page -> phys 0xB000
    mem.write_u32(0x3000, 0x4000 | 0x3).unwrap();
    mem.write_u32(0x4000 + pte_index * 4, 0xB000 | 0x3).unwrap();

    let pa_a = mmu
        .translate_linear(linear, 0x1000, libcorevm::memory::AccessType::Read, 0, &mem)
        .unwrap();
    assert_eq!(pa_a, 0xA000);

    let pa_b = mmu
        .translate_linear(linear, 0x3000, libcorevm::memory::AccessType::Read, 0, &mem)
        .unwrap();
    assert_eq!(pa_b, 0xB000);
}

#[test]
fn mmu_flush_tlb_observes_updated_pte() {
    let mut mmu = Mmu::new();
    let mut mem = GuestMemory::new(0x40_000);
    mmu.update_from_regs(CR0_PG, 0, 0);

    let linear = 0x0000_4000u64;
    let pte_index = ((linear >> 12) & 0x3FF) as u64;
    let cr3 = 0x1000u64;

    mem.write_u32(0x1000, 0x2000 | 0x3).unwrap();
    mem.write_u32(0x2000 + pte_index * 4, 0xA000 | 0x3).unwrap();

    let pa1 = mmu
        .translate_linear(linear, cr3, libcorevm::memory::AccessType::Read, 0, &mem)
        .unwrap();
    assert_eq!(pa1, 0xA000);

    // Remap same linear page and flush TLB.
    mem.write_u32(0x2000 + pte_index * 4, 0xB000 | 0x3).unwrap();
    mmu.flush_tlb();

    let pa2 = mmu
        .translate_linear(linear, cr3, libcorevm::memory::AccessType::Read, 0, &mem)
        .unwrap();
    assert_eq!(pa2, 0xB000);
}
