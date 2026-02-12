#![no_std]
#![no_main]

anyos_std::entry!(main);

fn is_leap_year(y: u32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(month: u32, year: u32) -> u32 {
    match month {
        1 => 31,
        2 => if is_leap_year(year) { 29 } else { 28 },
        3 => 31,
        4 => 30,
        5 => 31,
        6 => 30,
        7 => 31,
        8 => 31,
        9 => 30,
        10 => 31,
        11 => 30,
        12 => 31,
        _ => 30,
    }
}

// Zeller's congruence: day of week (0=Sun, 1=Mon, ..., 6=Sat) for the 1st of a given month
fn day_of_week(year: u32, month: u32, day: u32) -> u32 {
    let (y, m) = if month < 3 {
        (year - 1, month + 12)
    } else {
        (year, month)
    };
    let q = day;
    let k = y % 100;
    let j = y / 100;
    let h = (q + (13 * (m + 1)) / 5 + k + k / 4 + j / 4 + 5 * j) % 7;
    // Zeller's gives h=0 for Saturday, h=1 for Sunday...
    ((h + 6) % 7) // Convert: 0=Sun, 1=Mon, ..., 6=Sat
}

const MONTH_NAMES: [&str; 12] = [
    "January", "February", "March", "April", "May", "June",
    "July", "August", "September", "October", "November", "December",
];

fn main() {
    let mut time_buf = [0u8; 8];
    anyos_std::sys::time(&mut time_buf);
    let year = (time_buf[0] as u32) | ((time_buf[1] as u32) << 8);
    let month = time_buf[2] as u32;
    let today = time_buf[3] as u32;

    // Header
    let name = MONTH_NAMES[(month - 1) as usize];
    // Center the header in 20 chars
    let header_len = name.len() + 1 + 4; // "Month YYYY"
    let pad = if header_len < 20 { (20 - header_len) / 2 } else { 0 };
    let mut spaces = [b' '; 20];
    anyos_std::print!("{}", core::str::from_utf8(&spaces[..pad]).unwrap_or(""));
    anyos_std::println!("{} {}", name, year);
    anyos_std::println!("Su Mo Tu We Th Fr Sa");

    let first_dow = day_of_week(year, month, 1);
    let dim = days_in_month(month, year);

    // Leading spaces
    for _ in 0..first_dow {
        anyos_std::print!("   ");
    }

    let mut col = first_dow;
    for d in 1..=dim {
        if d == today {
            // Highlight today with brackets
            if d < 10 {
                anyos_std::print!("[{}]", d);
            } else {
                anyos_std::print!("[{}]", d); // this makes it 4 chars but that's OK visually
            }
        } else {
            anyos_std::print!("{:>2} ", d);
        }
        col += 1;
        if col == 7 {
            anyos_std::println!("");
            col = 0;
        }
    }
    if col != 0 {
        anyos_std::println!("");
    }
}
