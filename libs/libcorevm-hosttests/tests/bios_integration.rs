use std::fs;
use std::path::PathBuf;

use libcorevm::{
    corevm_create, corevm_destroy, corevm_get_instruction_count, corevm_get_last_error,
    corevm_load_rom, corevm_read_phys_u8, corevm_run, corevm_setup_ide,
    corevm_setup_pci_bus, corevm_setup_standard_devices,
};

struct VmHandle(u64);

impl Drop for VmHandle {
    fn drop(&mut self) {
        if self.0 != 0 {
            corevm_destroy(self.0);
        }
    }
}

fn bios_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("libcorevm")
        .join("bios")
        .join("bios")
}

fn last_error(handle: u64) -> String {
    let mut buf = vec![0u8; 512];
    let n = corevm_get_last_error(handle, buf.as_mut_ptr(), buf.len() as u32);
    if n == 0 {
        String::new()
    } else {
        String::from_utf8_lossy(&buf[..n as usize]).to_string()
    }
}

#[test]
fn bios_rom_is_mapped_at_reset_vector_window() {
    let vm = VmHandle(corevm_create(64));
    assert_ne!(vm.0, 0, "corevm_create failed");

    let bios = fs::read(bios_path()).expect("read bios image");
    let rc = corevm_load_rom(vm.0, 0xF0000, bios.as_ptr(), bios.len() as u32);
    assert_eq!(rc, 0, "corevm_load_rom failed");

    let reset_vec_byte = corevm_read_phys_u8(vm.0, 0xFFFF0);
    let expected = bios[0xFFF0];
    assert_eq!(reset_vec_byte, expected, "reset vector byte mismatch");
}

#[test]
fn bios_post_executes_without_unhandled_exception() {
    let vm = VmHandle(corevm_create(128));
    assert_ne!(vm.0, 0, "corevm_create failed");

    let bios = fs::read(bios_path()).expect("read bios image");
    let rc = corevm_load_rom(vm.0, 0xF0000, bios.as_ptr(), bios.len() as u32);
    assert_eq!(rc, 0, "corevm_load_rom failed");

    corevm_setup_standard_devices(vm.0);
    corevm_setup_pci_bus(vm.0);
    corevm_setup_ide(vm.0);

    // Run enough instructions to cover early POST/INT setup paths.
    let exit_code = corevm_run(vm.0, 200_000);
    let icount = corevm_get_instruction_count(vm.0);

    assert_ne!(exit_code, 1, "BIOS run hit exception: {}", last_error(vm.0));
    assert!(icount >= 10_000, "BIOS executed too few instructions: {icount}");
    assert!(exit_code == 2 || exit_code == 0, "unexpected exit code: {exit_code}");
}
