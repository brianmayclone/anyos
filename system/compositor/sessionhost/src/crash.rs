use anyos_std::prelude::*;

const CRASH_DIALOG_PATH: &str = "/System/CrashDialog.app/CrashDialog";

pub fn launch_crash_dialog(tid: u32) {
    let args = format!("{}", tid);
    anyos_std::process::spawn(CRASH_DIALOG_PATH, &args);
}
