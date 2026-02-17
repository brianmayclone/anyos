#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut buf = [0u8; 4096];
    let len = anyos_std::users::listgroups(&mut buf);

    if len == u32::MAX {
        anyos_std::println!("listgroups: failed to list groups");
        return;
    }

    if len == 0 {
        anyos_std::println!("No groups found.");
        return;
    }

    let data = core::str::from_utf8(&buf[..len as usize]).unwrap_or("");
    anyos_std::print!("{}", data);
}
