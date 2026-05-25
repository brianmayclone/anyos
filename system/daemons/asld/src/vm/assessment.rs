use alloc::format;
use alloc::string::String;

use super::{exit_reason, VmExitInfo, VmRuntimeEvent};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ExitAssessment {
    pub(super) ready: bool,
    pub(super) should_continue: bool,
    pub(super) halted: bool,
    pub(super) summary: String,
}

pub(super) enum RuntimeExitAssessment {
    Continue,
    Record(VmRuntimeEvent),
}

pub(super) fn assess_boot_exit(exit: &VmExitInfo) -> ExitAssessment {
    match exit.reason {
        exit_reason::HLT | exit_reason::HLT_EMULATED => ExitAssessment {
            ready: true,
            should_continue: false,
            halted: true,
            summary: format!("guest bootstrap reached halt ({})", describe_exit(exit)),
        },
        exit_reason::CPUID_EMULATED
        | exit_reason::CR_ACCESS_EMULATED
        | exit_reason::CPUID
        | exit_reason::PAUSE => ExitAssessment {
            ready: false,
            should_continue: true,
            halted: false,
            summary: format!("continuing after transient exit ({})", describe_exit(exit)),
        },
        exit_reason::IO_INSTRUCTION => ExitAssessment {
            ready: false,
            should_continue: false,
            halted: false,
            summary: format!(
                "guest requested unsupported I/O port {:#x} during boot",
                exit.io_port
            ),
        },
        exit_reason::EPT_VIOLATION | exit_reason::EPT_MISCONFIG => ExitAssessment {
            ready: false,
            should_continue: false,
            halted: false,
            summary: format!(
                "guest memory translation failure during boot ({})",
                describe_exit(exit)
            ),
        },
        exit_reason::INVALID_GUEST_STATE | exit_reason::TRIPLE_FAULT | exit_reason::SHUTDOWN => {
            ExitAssessment {
                ready: false,
                should_continue: false,
                halted: false,
                summary: format!(
                    "guest failed to enter stable boot state ({})",
                    describe_exit(exit)
                ),
            }
        }
        _ => ExitAssessment {
            ready: false,
            should_continue: false,
            halted: false,
            summary: format!(
                "unexpected guest exit during boot ({})",
                describe_exit(exit)
            ),
        },
    }
}

pub(super) fn assess_runtime_exit(exit: &VmExitInfo) -> RuntimeExitAssessment {
    match exit.reason {
        exit_reason::CPUID_EMULATED
        | exit_reason::CR_ACCESS_EMULATED
        | exit_reason::CPUID
        | exit_reason::PAUSE => RuntimeExitAssessment::Continue,
        exit_reason::HLT | exit_reason::HLT_EMULATED => RuntimeExitAssessment::Record(
            build_runtime_event(exit, "guest halted after runtime dispatch", false, true),
        ),
        exit_reason::IO_INSTRUCTION => RuntimeExitAssessment::Record(build_runtime_event(
            exit,
            "guest triggered unsupported I/O instruction",
            true,
            false,
        )),
        exit_reason::EPT_VIOLATION | exit_reason::EPT_MISCONFIG => RuntimeExitAssessment::Record(
            build_runtime_event(exit, "guest hit memory translation failure", true, false),
        ),
        exit_reason::INVALID_GUEST_STATE | exit_reason::TRIPLE_FAULT | exit_reason::SHUTDOWN => {
            RuntimeExitAssessment::Record(build_runtime_event(
                exit,
                "guest entered fatal virtualization state",
                true,
                true,
            ))
        }
        _ => RuntimeExitAssessment::Record(build_runtime_event(
            exit,
            "guest exited to host runtime dispatcher",
            false,
            false,
        )),
    }
}

fn build_runtime_event(
    exit: &VmExitInfo,
    prefix: &str,
    fatal: bool,
    halted: bool,
) -> VmRuntimeEvent {
    VmRuntimeEvent {
        reason: String::from(exit_reason_name(exit.reason)),
        summary: format!("{} ({})", prefix, describe_exit(exit)),
        fatal,
        qualification: exit.qualification,
        guest_phys_addr: exit.guest_phys_addr,
        guest_virt_addr: exit.guest_virt_addr,
        halted,
    }
}

pub(super) fn describe_exit(exit: &VmExitInfo) -> String {
    format!(
        "{} hw={} qual={:#x} gpa={:#x} gva={:#x}",
        exit_reason_name(exit.reason),
        exit.hw_reason,
        exit.qualification,
        exit.guest_phys_addr,
        exit.guest_virt_addr
    )
}

fn exit_reason_name(reason: u32) -> &'static str {
    match reason {
        exit_reason::EXTERNAL_INTERRUPT => "external-interrupt",
        exit_reason::TRIPLE_FAULT => "triple-fault",
        exit_reason::INIT_SIGNAL => "init-signal",
        exit_reason::SIPI => "sipi",
        exit_reason::CPUID => "cpuid",
        exit_reason::HLT => "hlt",
        exit_reason::INVD => "invd",
        exit_reason::INVLPG => "invlpg",
        exit_reason::RDPMC => "rdpmc",
        exit_reason::RDTSC => "rdtsc",
        exit_reason::RSM => "rsm",
        exit_reason::VMCALL => "vmcall",
        exit_reason::CR_ACCESS => "cr-access",
        exit_reason::DR_ACCESS => "dr-access",
        exit_reason::IO_INSTRUCTION => "io-instruction",
        exit_reason::RDMSR => "rdmsr",
        exit_reason::WRMSR => "wrmsr",
        exit_reason::INVALID_GUEST_STATE => "invalid-guest-state",
        exit_reason::PAUSE => "pause",
        exit_reason::EPT_VIOLATION => "ept-violation",
        exit_reason::EPT_MISCONFIG => "ept-misconfig",
        exit_reason::RDTSCP => "rdtscp",
        exit_reason::PREEMPTION_TIMER => "preemption-timer",
        exit_reason::WBINVD => "wbinvd",
        exit_reason::XSETBV => "xsetbv",
        exit_reason::RDRAND => "rdrand",
        exit_reason::INVPCID => "invpcid",
        exit_reason::RDSEED => "rdseed",
        exit_reason::SHUTDOWN => "shutdown",
        exit_reason::SMI => "smi",
        exit_reason::NMI_WINDOW => "nmi-window",
        exit_reason::IRQ_WINDOW => "irq-window",
        exit_reason::CPUID_EMULATED => "cpuid-emulated",
        exit_reason::HLT_EMULATED => "hlt-emulated",
        exit_reason::CR_ACCESS_EMULATED => "cr-access-emulated",
        _ => "unknown",
    }
}
