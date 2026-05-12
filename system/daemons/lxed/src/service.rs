use alloc::format;

use libsvc::ServiceLifecycle;

use crate::state::RuntimeState;

pub(crate) fn publish_details(lifecycle: &mut Option<ServiceLifecycle>, runtime: &RuntimeState) {
    if let Some(lifecycle) = lifecycle.as_mut() {
        let _ = lifecycle.set_detail("root", &runtime.config.root);
        let _ = lifecycle.set_detail("rootfs", &runtime.config.rootfs);
        let _ = lifecycle.set_detail("active_runs", &format!("{}", runtime.active_runs_len()));
        let _ = lifecycle.set_detail(
            "active_processes",
            &format!("{}", runtime.active_processes_len()),
        );
        let _ = lifecycle.set_detail("writer", &format!("{}", runtime.writer_tid()));
        let _ = lifecycle.set_detail("syncs", &format!("{}", runtime.syncs()));
    }
}
