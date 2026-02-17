#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut buf = [0u8; 2048];
    let len = anyos_std::users::listusers(&mut buf);
    if len == u32::MAX {
        anyos_std::println!("listuser: failed to list users");
        return;
    }
    if len == 0 {
        anyos_std::println!("No users found.");
        return;
    }

    let data = match core::str::from_utf8(&buf[..len as usize]) {
        Ok(s) => s,
        Err(_) => {
            anyos_std::println!("listuser: invalid data");
            return;
        }
    };

    anyos_std::println!("UID  Username");
    anyos_std::println!("---  --------");
    for line in data.split('\n') {
        if line.is_empty() {
            continue;
        }
        if let Some(colon) = line.find(':') {
            let uid = &line[..colon];
            let name = &line[colon + 1..];
            anyos_std::println!("{:<5}{}", uid, name);
        }
    }
}
