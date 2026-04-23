#![cfg_attr(not(target_os = "linux"), no_std)]
#![cfg_attr(not(target_os = "linux"), no_main)]

#[cfg(not(target_os = "linux"))]
anyos_std::entry!(asld::run);

fn main() {
    asld::run();
}
