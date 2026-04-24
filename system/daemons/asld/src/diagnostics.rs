use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use anyos_std::fs;

const STATUS_PATH: &str = "/System/var/asl/asld.status";
const BROKERS: &[&str] = &["aslnetd", "aslfsd", "aslconsoled", "aslobsd"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelfCheckReport {
    pub healthy: bool,
    pub lines: Vec<String>,
}

impl SelfCheckReport {
    pub fn summary(&self) -> &'static str {
        if self.healthy {
            "ok"
        } else {
            "degraded"
        }
    }
}

pub fn run_self_check() -> SelfCheckReport {
    let mut healthy = true;
    let mut lines = alloc::vec![
        String::from("service\tasld"),
        String::from("schema\tregistered"),
        String::from("ipc_pipe\tasld"),
    ];

    let avm_healthy = append_avm_probe(&mut lines);
    healthy &= avm_healthy;

    for broker in BROKERS {
        let available = append_broker_probe(&mut lines, broker);
        healthy &= available;
    }

    lines.push(format!(
        "self_check\t{}",
        if healthy { "pass" } else { "degraded" }
    ));

    SelfCheckReport { healthy, lines }
}

pub fn write_status(report: &SelfCheckReport) {
    let _ = fs::mkdir("/System/var");
    let _ = fs::mkdir("/System/var/asl");
    let mut text = format!("health={}\n", report.summary());
    for line in &report.lines {
        text.push_str(line);
        text.push('\n');
    }
    let _ = fs::write_bytes(STATUS_PATH, text.as_bytes());
}

fn append_broker_probe(lines: &mut Vec<String>, broker: &'static str) -> bool {
    match crate::broker::status(broker) {
        Ok(status_lines) => {
            lines.push(format!("dependency\t{}\tavailable\ttrue", broker));
            for line in status_lines {
                lines.push(format!("dependency_status\t{}\t{}", broker, line));
            }
            true
        }
        Err(err) => {
            lines.push(format!(
                "dependency\t{}\tavailable\tfalse\t{}",
                broker,
                err.message()
            ));
            false
        }
    }
}

#[cfg(target_os = "linux")]
fn append_avm_probe(lines: &mut Vec<String>) -> bool {
    lines.push(String::from("avm_host_mode\ttrue"));
    lines.push(String::from("avm_api_available\tfalse"));
    lines.push(String::from("avm_backend\thost-stub"));
    lines.push(String::from(
        "avm_message\tAVM probing is only live inside anyOS",
    ));
    true
}

#[cfg(not(target_os = "linux"))]
fn append_avm_probe(lines: &mut Vec<String>) -> bool {
    let avm = libavm::Avm::new();
    let mut healthy = true;

    match avm.api_version() {
        Ok(version) => {
            let api_ok = version == libavm::AVM_API_VERSION;
            healthy &= api_ok;
            lines.push(String::from("avm_host_mode\tfalse"));
            lines.push(String::from("avm_api_available\ttrue"));
            lines.push(format!("avm_api_version\t{}", version));
            lines.push(format!("avm_expected_api\t{}", libavm::AVM_API_VERSION));
            lines.push(format!("avm_api_match\t{}", api_ok));
        }
        Err(err) => {
            lines.push(String::from("avm_host_mode\tfalse"));
            lines.push(String::from("avm_api_available\tfalse"));
            lines.push(format!("avm_error\t{:?}", err));
            return false;
        }
    }

    match avm.backend_info() {
        Ok(info) => {
            let backend = backend_name(info.backend_kind);
            let backend_ok = info.backend_kind != 0;
            healthy &= backend_ok;
            lines.push(format!("avm_backend\t{}", backend));
            lines.push(format!("avm_backend_kind\t{}", info.backend_kind));
            lines.push(format!("avm_feature_bits\t0x{:x}", info.feature_bits));
            lines.push(format!("avm_max_vcpus\t{}", info.max_vcpus));
            lines.push(format!("avm_exit_info_size\t{}", info.exit_info_size));
            lines.push(format!("avm_regs_size\t{}", info.regs_size));
            lines.push(format!("avm_sregs_size\t{}", info.sregs_size));
        }
        Err(err) => {
            healthy = false;
            lines.push(format!("avm_backend_error\t{:?}", err));
        }
    }

    for (name, extension, required) in [
        ("dirty_log", libavm::AVM_EXT_DIRTY_LOG, false),
        ("mp_state", libavm::AVM_EXT_MP_STATE, false),
        ("gva_translate", libavm::AVM_EXT_GVA_TRANSLATE, true),
        ("fpu_state", libavm::AVM_EXT_FPU_STATE, false),
        ("irq_injection", libavm::AVM_EXT_IRQ_INJECTION, true),
    ] {
        match avm.check_extension(extension) {
            Ok(enabled) => {
                lines.push(format!(
                    "avm_extension\t{}\tenabled={}\trequired={}",
                    name, enabled, required
                ));
                if required && !enabled {
                    healthy = false;
                }
            }
            Err(err) => {
                lines.push(format!(
                    "avm_extension\t{}\tenabled=false\trequired={}\terror={:?}",
                    name, required, err
                ));
                if required {
                    healthy = false;
                }
            }
        }
    }

    healthy
}

#[cfg(not(target_os = "linux"))]
fn backend_name(kind: u32) -> &'static str {
    match kind {
        1 => "avm-vmx",
        2 => "avm-svm",
        0 => "none",
        _ => "avm-unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::run_self_check;

    #[test]
    fn self_check_reports_service_and_avm_lines() {
        let report = run_self_check();
        assert!(report.lines.iter().any(|line| line == "service\tasld"));
        assert!(report.lines.iter().any(|line| line.starts_with("avm_")));
        assert!(report
            .lines
            .iter()
            .any(|line| line.starts_with("self_check\t")));
    }
}
