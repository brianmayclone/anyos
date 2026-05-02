use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

pub const DISTRO_IMAGES_ROOT: &str = "/System/var/asl/distros";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DistroState {
    Created,
    Starting,
    Booting,
    Ready,
    Degraded,
    Stopping,
    Stopped,
    Failed,
}

impl DistroState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Starting => "starting",
            Self::Booting => "booting",
            Self::Ready => "ready",
            Self::Degraded => "degraded",
            Self::Stopping => "stopping",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentState {
    NotPresent,
    Starting,
    Connected,
    Ready,
    Degraded,
    Disconnected,
}

impl AgentState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotPresent => "not_present",
            Self::Starting => "starting",
            Self::Connected => "connected",
            Self::Ready => "ready",
            Self::Degraded => "degraded",
            Self::Disconnected => "disconnected",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionMode {
    Agent,
    FallbackConsole,
}

impl SessionMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::FallbackConsole => "fallback-console",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DistroHealth {
    Ready,
    Degraded,
    Failed,
    Stopped,
}

impl DistroHealth {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Degraded => "degraded",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VmRunState {
    Provisioned,
    BootReady,
    Running,
    Halted,
    Degraded,
}

impl VmRunState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Provisioned => "provisioned",
            Self::BootReady => "boot-ready",
            Self::Running => "running",
            Self::Halted => "halted",
            Self::Degraded => "degraded",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resources {
    pub memory_mb: u32,
    pub vcpu_count: u16,
    pub autostart: bool,
}

pub const DEFAULT_GUEST_MEMORY_MB: u32 = 2048;

impl Default for Resources {
    fn default() -> Self {
        Self {
            memory_mb: DEFAULT_GUEST_MEMORY_MB,
            vcpu_count: 2,
            autostart: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageSpec {
    pub layout: String,
    pub base_image_path: String,
    pub overlay_image_path: String,
    pub state_image_path: String,
    pub seed_image_path: String,
    pub state_image_enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkPolicy {
    pub mode: String,
    pub dns_mode: String,
    pub allow_outbound: bool,
}

impl Default for NetworkPolicy {
    fn default() -> Self {
        Self {
            mode: String::from("nat"),
            dns_mode: String::from("host-broker"),
            allow_outbound: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MountSpec {
    pub id: String,
    pub host_path: String,
    pub guest_path: String,
    pub mode: String,
    pub metadata_mode: String,
    pub case_mode: String,
    pub exec_policy: String,
    pub watch_policy: String,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MountValidation {
    pub id: String,
    pub guest_path: String,
    pub valid: bool,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortForwardSpec {
    pub id: String,
    pub listen_address: String,
    pub listen_port: u16,
    pub guest_port: u16,
    pub protocol: String,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkValidation {
    pub id: String,
    pub listen: String,
    pub valid: bool,
    pub exposure: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageValidation {
    pub role: String,
    pub path: String,
    pub valid: bool,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentPolicy {
    pub enabled: bool,
    pub required_for_rich_integration: bool,
    pub fallback_console_enabled: bool,
}

impl Default for AgentPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            required_for_rich_integration: true,
            fallback_console_enabled: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LifecyclePolicy {
    pub restart_on_failure: bool,
    pub shutdown_timeout_ms: u32,
    pub boot_timeout_ms: u32,
}

impl Default for LifecyclePolicy {
    fn default() -> Self {
        Self {
            restart_on_failure: true,
            shutdown_timeout_ms: 10_000,
            boot_timeout_ms: 30_000,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DistroMetadata {
    pub distro_family: String,
    pub distro_version: String,
    pub notes: String,
}

impl Default for DistroMetadata {
    fn default() -> Self {
        Self {
            distro_family: String::new(),
            distro_version: String::new(),
            notes: String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DistroConfig {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub owner: String,
    pub base_image_ref: String,
    pub kernel_profile: String,
    pub resources: Resources,
    pub storage: StorageSpec,
    pub network: NetworkPolicy,
    pub mounts: Vec<MountSpec>,
    pub port_forwards: Vec<PortForwardSpec>,
    pub agent: AgentPolicy,
    pub lifecycle: LifecyclePolicy,
    pub metadata: DistroMetadata,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DistroStatus {
    pub name: String,
    pub state: DistroState,
    pub health: DistroHealth,
    pub agent_state: AgentState,
    pub uptime_ms: u64,
    pub last_error: Option<String>,
    pub resources: Resources,
    pub network: NetworkPolicy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellSession {
    pub session_id: String,
    pub session_name: String,
    pub mode: SessionMode,
    pub console_pipe_name: String,
    pub stdin_pipe_name: String,
    pub attached_pid: u32,
    pub reused: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecInvocation {
    pub exec_id: String,
    pub mode: SessionMode,
    pub cwd: String,
    pub env_count: usize,
    pub command_line: String,
    pub stdout_pipe_name: String,
    pub stdin_pipe_name: String,
    pub attached_pid: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VmStatusSummary {
    pub backend: String,
    pub run_state: VmRunState,
    pub guest_memory_mb: u32,
    pub boot_summary: String,
    pub last_exit_summary: String,
    pub total_exits: u64,
    pub recent_exit_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VmExitEvent {
    pub seq: u64,
    pub reason: String,
    pub summary: String,
    pub fatal: bool,
    pub qualification: u64,
    pub guest_phys_addr: u64,
    pub guest_virt_addr: u64,
}

pub fn is_valid_distro_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_'))
}

pub fn default_storage_for(name: &str) -> StorageSpec {
    StorageSpec {
        layout: String::from("layered-v1"),
        base_image_path: format!("{DISTRO_IMAGES_ROOT}/{name}/images/base.img"),
        overlay_image_path: format!("{DISTRO_IMAGES_ROOT}/{name}/images/overlay.img"),
        state_image_path: format!("{DISTRO_IMAGES_ROOT}/{name}/images/state.img"),
        seed_image_path: format!("{DISTRO_IMAGES_ROOT}/{name}/images/seed.img"),
        state_image_enabled: true,
    }
}

#[cfg(test)]
mod tests {
    use super::{default_storage_for, is_valid_distro_name};

    #[test]
    fn validates_distro_names_conservatively() {
        assert!(is_valid_distro_name("ubuntu-dev"));
        assert!(is_valid_distro_name("ci_01"));
        assert!(!is_valid_distro_name(""));
        assert!(!is_valid_distro_name("ubuntu/dev"));
        assert!(!is_valid_distro_name("ubuntu dev"));
    }

    #[test]
    fn builds_predictable_storage_paths() {
        let storage = default_storage_for("ubuntu-dev");
        assert!(storage
            .base_image_path
            .ends_with("/ubuntu-dev/images/base.img"));
        assert!(storage
            .overlay_image_path
            .ends_with("/ubuntu-dev/images/overlay.img"));
        assert!(storage
            .seed_image_path
            .ends_with("/ubuntu-dev/images/seed.img"));
        assert!(storage.state_image_enabled);
    }
}
