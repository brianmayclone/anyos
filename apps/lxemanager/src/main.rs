#![no_std]
#![no_main]

mod inspect;
mod state;
mod ui;
mod worker;

anyos_std::entry!(main);

fn main() {
    if !libanyui_client::init() {
        anyos_std::println!("lxemanager: failed to load libanyui.so");
        return;
    }
    ui::build();
    libanyui_client::run();
}
