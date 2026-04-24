use std::path::Path;
use std::process::{ExitStatus, Command};
use std::time::Duration;

pub fn run_executable(exe_path: &Path) -> ExitStatus {
    let mut attempts = 0u32;
    loop {
        match Command::new(exe_path).status() {
            Ok(status) => return status,
            Err(err) if err.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                attempts += 1;
                if attempts > 20 {
                    panic!("failed to execute compiled binary: {}", err);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(err) => panic!("failed to execute compiled binary: {}", err),
        }
    }
}
