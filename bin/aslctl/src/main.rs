#![cfg_attr(not(target_os = "linux"), no_std)]
#![cfg_attr(not(target_os = "linux"), no_main)]

#[cfg(not(target_os = "linux"))]
anyos_std::entry!(aslctl::run);

fn main() {
    aslctl::run();
}
