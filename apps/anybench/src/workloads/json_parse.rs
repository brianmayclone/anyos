//! CPU Benchmark 10 — JSON Parsing.
//!
//! Scans and validates a JSON-like telemetry document for [`CPU_TEST_MS`]
//! milliseconds. Returns bytes parsed.

use super::CPU_TEST_MS;

const DOC: &[u8] = br#"{
  "app":"anyBench",
  "scores":[1024,998,1410,1205,875,1675,1911,2048],
  "system":{"kernel":"anyOS","arch":"x86_64","cores":8,"smp":true},
  "samples":[
    {"name":"cpu","unit":"points","single":1234,"multi":6543},
    {"name":"memory","unit":"mbps","read":2400,"write":1800},
    {"name":"gpu","unit":"pixels","onscreen":99123,"offscreen":181991},
    {"name":"disk","unit":"ops","read":492,"write":377}
  ],
  "tags":["native","no_std","gui","cli","modern"]
}"#;

/// JSON-like lexical parsing benchmark.
pub fn bench_json_parse() -> u64 {
    let mut bytes = 0u64;
    let mut checksum = 0u32;
    let start = anyos_std::sys::uptime_ms();
    while anyos_std::sys::uptime_ms().wrapping_sub(start) < CPU_TEST_MS {
        let mut depth = 0u32;
        let mut in_string = false;
        let mut escape = false;
        let mut number = 0u32;
        let mut seen_digit = false;

        for &b in DOC {
            if in_string {
                if escape {
                    checksum = checksum.wrapping_add(b as u32);
                    escape = false;
                } else if b == b'\\' {
                    escape = true;
                } else if b == b'"' {
                    in_string = false;
                } else {
                    checksum = checksum.wrapping_mul(31).wrapping_add(b as u32);
                }
                continue;
            }

            match b {
                b'"' => in_string = true,
                b'{' | b'[' => depth = depth.wrapping_add(1),
                b'}' | b']' => depth = depth.wrapping_sub(1),
                b'0'..=b'9' => {
                    number = number.wrapping_mul(10).wrapping_add((b - b'0') as u32);
                    seen_digit = true;
                }
                _ => {
                    if seen_digit {
                        checksum = checksum.wrapping_add(number ^ depth);
                        number = 0;
                        seen_digit = false;
                    }
                }
            }
        }
        if seen_digit {
            checksum = checksum.wrapping_add(number ^ depth);
        }
        bytes += DOC.len() as u64;
    }

    core::hint::black_box(checksum);
    bytes
}
