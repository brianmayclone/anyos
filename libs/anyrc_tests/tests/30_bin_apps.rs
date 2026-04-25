use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

static CCARGO_LOCK: Mutex<()> = Mutex::new(());
static HOST_CCARGO: OnceLock<PathBuf> = OnceLock::new();

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("anyrc_tests should live below libs/")
        .to_path_buf()
}

fn host_ccargo(root: &Path) -> &'static PathBuf {
    HOST_CCARGO.get_or_init(|| build_host_ccargo(root))
}

fn build_host_ccargo(root: &Path) -> PathBuf {
    let output = Command::new("cargo")
        .current_dir(root)
        .env("CARGO_TERM_COLOR", "never")
        .args([
            "build",
            "--manifest-path",
            "bin/acargo/Cargo.toml",
            "--features",
            "host",
            "--bin",
            "ccargo",
        ])
        .output()
        .expect("failed to build host ccargo");

    assert!(
        output.status.success(),
        "failed to build host ccargo\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        truncate_output(&String::from_utf8_lossy(&output.stdout)),
        truncate_output(&String::from_utf8_lossy(&output.stderr))
    );

    root.join("target/debug/ccargo")
}

fn assert_ccargo_builds(crate_path: &str) {
    let _guard = CCARGO_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let root = repo_root();
    let ccargo = host_ccargo(&root);

    println!("ccargo build {crate_path} --target x86_64-anyos ...");
    let output = Command::new(ccargo)
        .current_dir(&root)
        .env("CARGO_TERM_COLOR", "never")
        .args(["build", crate_path, "--target", "x86_64-anyos"])
        .output()
        .expect("failed to run host ccargo");

    if output.status.success() {
        println!("{crate_path} ... ok");
        return;
    }

    println!("{crate_path} ... not ok");
    panic!(
        "{crate_path} failed to build with ccargo\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        truncate_output(&String::from_utf8_lossy(&output.stdout)),
        truncate_output(&String::from_utf8_lossy(&output.stderr))
    );
}

fn truncate_output(output: &str) -> String {
    const MAX: usize = 8 * 1024;
    if output.len() <= MAX {
        return output.to_string();
    }
    format!("{}... <truncated {} bytes>", &output[..MAX], output.len() - MAX)
}

macro_rules! ccargo_crate_tests {
    ($($name:ident, $crate_path:literal;)*) => {
        $(
            #[test]
            fn $name() {
                assert_ccargo_builds($crate_path);
            }
        )*
    };
}

ccargo_crate_tests! {
    bin_ac, "bin/ac";
    bin_acargo, "bin/acargo";
    bin_addgroup, "bin/addgroup";
    bin_adduser, "bin/adduser";
    bin_agit, "bin/agit";
    bin_ami, "bin/ami";
    bin_anyrc, "bin/anyrc";
    bin_apkg, "bin/apkg";
    bin_arp, "bin/arp";
    bin_aslctl, "bin/aslctl";
    bin_awk, "bin/awk";
    bin_banner, "bin/banner";
    bin_base64, "bin/base64";
    bin_bcedit, "bin/bcedit";
    bin_cal, "bin/cal";
    bin_cat, "bin/cat";
    bin_chmod, "bin/chmod";
    bin_chown, "bin/chown";
    bin_clear, "bin/clear";
    bin_corefs_defrag, "bin/corefs-defrag";
    bin_corefs_dump, "bin/corefs-dump";
    bin_corefs_resize, "bin/corefs-resize";
    bin_corefs_scrub, "bin/corefs-scrub";
    bin_corefs_snapshot, "bin/corefs-snapshot";
    bin_corefs_tier, "bin/corefs-tier";
    bin_corefsd, "bin/corefsd";
    bin_cp, "bin/cp";
    bin_crontab, "bin/crontab";
    bin_date, "bin/date";
    bin_dba, "bin/dba";
    bin_dcpdump, "bin/dcpdump";
    bin_dd, "bin/dd";
    bin_delgroup, "bin/delgroup";
    bin_deluser, "bin/deluser";
    bin_devlist, "bin/devlist";
    bin_df, "bin/df";
    bin_dhcp, "bin/dhcp";
    bin_dmesg, "bin/dmesg";
    bin_dns, "bin/dns";
    bin_du, "bin/du";
    bin_echo, "bin/echo";
    bin_echoserver, "bin/echoserver";
    bin_env, "bin/env";
    bin_export, "bin/export";
    bin_false, "bin/false";
    bin_fdisk, "bin/fdisk";
    bin_file, "bin/file";
    bin_find, "bin/find";
    bin_free, "bin/free";
    bin_fsck_corefs, "bin/fsck.corefs";
    bin_ftp, "bin/ftp";
    bin_fusedemo, "bin/fusedemo";
    bin_grep, "bin/grep";
    bin_gzip, "bin/gzip";
    bin_head, "bin/head";
    bin_hexdump, "bin/hexdump";
    bin_hostname, "bin/hostname";
    bin_htop, "bin/htop";
    bin_ifconfig, "bin/ifconfig";
    bin_install, "bin/install";
    bin_jp2a, "bin/jp2a";
    bin_jscript, "bin/jscript";
    bin_kill, "bin/kill";
    bin_killall, "bin/killall";
    bin_kstress, "bin/kstress";
    bin_listgroups, "bin/listgroups";
    bin_listuser, "bin/listuser";
    bin_ln, "bin/ln";
    bin_ls, "bin/ls";
    bin_mkdir, "bin/mkdir";
    bin_mkfs_corefs, "bin/mkfs.corefs";
    bin_mode, "bin/mode";
    bin_more, "bin/more";
    bin_mount, "bin/mount";
    bin_mv, "bin/mv";
    bin_nano, "bin/nano";
    bin_neofetch, "bin/neofetch";
    bin_netstat, "bin/netstat";
    bin_nice, "bin/nice";
    bin_ntp, "bin/ntp";
    bin_nvi, "bin/nvi";
    bin_open, "bin/open";
    bin_passwd, "bin/passwd";
    bin_ping, "bin/ping";
    bin_pipes, "bin/pipes";
    bin_play, "bin/play";
    bin_ps, "bin/ps";
    bin_pwd, "bin/pwd";
    bin_readlink, "bin/readlink";
    bin_reboot, "bin/reboot";
    bin_rev, "bin/rev";
    bin_rm, "bin/rm";
    bin_route, "bin/route";
    bin_scp, "bin/scp";
    bin_sdel, "bin/sdel";
    bin_sed, "bin/sed";
    bin_seq, "bin/seq";
    bin_set, "bin/set";
    bin_sget, "bin/sget";
    bin_sleep, "bin/sleep";
    bin_sort, "bin/sort";
    bin_speedtest, "bin/speedtest";
    bin_sstore, "bin/sstore";
    bin_stat, "bin/stat";
    bin_strings, "bin/strings";
    bin_su, "bin/su";
    bin_sudo, "bin/sudo";
    bin_sync, "bin/sync";
    bin_sysinfo, "bin/sysinfo";
    bin_tail, "bin/tail";
    bin_tar, "bin/tar";
    bin_top, "bin/top";
    bin_touch, "bin/touch";
    bin_true, "bin/true";
    bin_uictl, "bin/uictl";
    bin_umount, "bin/umount";
    bin_uname, "bin/uname";
    bin_uniq, "bin/uniq";
    bin_unzip, "bin/unzip";
    bin_uptime, "bin/uptime";
    bin_vi, "bin/vi";
    bin_vmctl, "bin/vmctl";
    bin_vmd, "bin/vmd";
    bin_wc, "bin/wc";
    bin_wget, "bin/wget";
    bin_which, "bin/which";
    bin_whoami, "bin/whoami";
    bin_wifi, "bin/wifi";
    bin_xargs, "bin/xargs";
    bin_xxd, "bin/xxd";
    bin_yes, "bin/yes";
    bin_zip, "bin/zip";

    apps_anybench, "apps/anybench";
    apps_anycode, "apps/anycode";
    apps_anymail, "apps/anymail";
    apps_anyzilla, "apps/anyzilla";
    apps_button_demo, "apps/button_demo";
    apps_calc, "apps/calc";
    apps_clipman, "apps/clipman";
    apps_clock, "apps/clock";
    apps_demo_anyui, "apps/demo_anyui";
    apps_diagnostics, "apps/diagnostics";
    apps_diff, "apps/diff";
    apps_diskusage, "apps/diskusage";
    apps_fontviewer, "apps/fontviewer";
    apps_forger, "apps/forger";
    apps_ftp_settings, "apps/ftp-settings";
    apps_gldemo, "apps/gldemo";
    apps_iconview, "apps/iconview";
    apps_imgview, "apps/imgview";
    apps_installer, "apps/installer";
    apps_keyboard, "apps/keyboard";
    apps_mdview, "apps/mdview";
    apps_minesweeper, "apps/minesweeper";
    apps_notepad, "apps/notepad";
    apps_notifications, "apps/notifications";
    apps_paint, "apps/paint";
    apps_screenshot, "apps/screenshot";
    apps_store, "apps/store";
    apps_surf, "apps/surf";
    apps_updater, "apps/updater";
    apps_videoplayer, "apps/videoplayer";
    apps_vmmanager, "apps/vmmanager";
    apps_vnc_settings, "apps/vnc-settings";
    apps_webmanager, "apps/webmanager";
}
