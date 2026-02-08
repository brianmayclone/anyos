#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    anyos_std::println!("Hello from Rust on .anyOS!");

    let pid = anyos_std::process::getpid();
    anyos_std::println!("My PID is {}", pid);

    // Test heap allocation
    let mut numbers = anyos_std::Vec::new();
    for i in 0..5 {
        numbers.push(i * 10);
    }
    anyos_std::println!("Vec: {:?}", numbers);

    let greeting = anyos_std::format!("Goodbye from PID {}!", pid);
    anyos_std::println!("{}", greeting);
}
