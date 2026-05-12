use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    run_once();
}

#[test]
fn downloads_debian_bootstrap_closure_into_tmp() {
    run_once();
}

fn run_once() {
    let tmp = temp_root();
    fs::create_dir_all(&tmp).expect("create lxe hosttest tmp dir");
    println!("lxe hosttest: tmp={}", tmp.display());

    let result = liblxecore::hosttest::run_full_download_test(&tmp);
    let cleanup = fs::remove_dir_all(&tmp);

    match result {
        Ok(report) => {
            println!(
                "lxe hosttest: index={} gz={} packages={} deb_bytes={} cache={}",
                report.index_bytes,
                report.index_gz_bytes,
                report.packages,
                report.downloaded_bytes,
                report.cache_dir.display()
            );
            if let Err(err) = cleanup {
                eprintln!("lxe hosttest: cleanup failed: {err}");
            }
        }
        Err(err) => {
            if let Err(cleanup_err) = cleanup {
                eprintln!("lxe hosttest: cleanup failed after error: {cleanup_err}");
            }
            panic!("{err}");
        }
    }
}

fn temp_root() -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch");
    std::env::temp_dir().join(format!(
        "lxe-hosttest-{}-{}",
        std::process::id(),
        now.as_nanos()
    ))
}
