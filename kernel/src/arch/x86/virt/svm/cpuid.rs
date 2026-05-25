//! Conservative CPUID surface for AVM/SVM guests.
//!
//! The host CPUID leaf set can advertise AMD features that require firmware,
//! MSR, APIC, or memory-encryption support that AVM does not virtualize yet.
//! Expose a stable virtual CPU until those facilities are implemented.

const LEAF_EXT_MAX: u32 = 0x8000_0000;
const LEAF_EXT_FEATURES: u32 = 0x8000_0001;
const LEAF_EXT_ADDR_SIZE: u32 = 0x8000_0008;
const LEAF_AMD_MEM_ENCRYPTION: u32 = 0x8000_001f;
const LEAF_AMD_EXT_FEATURES_21: u32 = 0x8000_0021;
const LEAF_AMD_EXT_FEATURES_22: u32 = 0x8000_0022;

pub(super) fn sanitize(
    leaf: u32,
    subleaf: u32,
    mut eax: u32,
    mut ebx: u32,
    mut ecx: u32,
    mut edx: u32,
) -> (u32, u32, u32, u32) {
    match leaf {
        1 => {
            ecx &= !(1 << 5); // VMX
            ecx &= !(1 << 21); // x2APIC
            ecx &= !(1 << 24); // TSC deadline timer
            ecx |= 1 << 31; // hypervisor present
        }
        7 if subleaf == 0 => {
            ebx &= !(1 << 18); // RDSEED
        }
        LEAF_EXT_MAX => {
            if eax > LEAF_EXT_ADDR_SIZE {
                eax = LEAF_EXT_ADDR_SIZE;
            }
        }
        LEAF_EXT_FEATURES => {
            ecx &= !(1 << 2); // SVM
            edx &= !(1 << 27); // RDTSCP
        }
        LEAF_AMD_MEM_ENCRYPTION | LEAF_AMD_EXT_FEATURES_21 | LEAF_AMD_EXT_FEATURES_22 => {
            eax = 0;
            ebx = 0;
            ecx = 0;
            edx = 0;
        }
        _ => {}
    }

    (eax, ebx, ecx, edx)
}
