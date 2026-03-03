#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    anyos_std::i18n::init();
    let t = anyos_std::i18n::t;
    let ticks = anyos_std::sys::uptime();
    let hz = anyos_std::sys::tick_hz();
    let secs = if hz > 0 { ticks / hz } else { 0 };
    let hours = secs / 3600;
    let mins = (secs % 3600) / 60;
    let s = secs % 60;

    if hours > 0 {
        anyos_std::println!("{} {}h {}m {}s", t("up"), hours, mins, s);
    } else if mins > 0 {
        anyos_std::println!("{} {}m {}s", t("up"), mins, s);
    } else {
        anyos_std::println!("{} {}s", t("up"), s);
    }
}
