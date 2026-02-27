//! CPU Benchmark 1 — Prime Sieve (Eratosthenes).
//!
//! Repeatedly sieves primes up to N for [`CPU_TEST_MS`] milliseconds and
//! returns the cumulative number of primes found across all iterations.

use alloc::vec;
use super::CPU_TEST_MS;

/// Runs the Sieve of Eratosthenes up to 100 000, repeated for CPU_TEST_MS.
pub fn bench_prime_sieve() -> u64 {
    const N: usize = 100_000;
    let mut sieve = vec![true; N];
    let mut total: u64 = 0;
    let start = anyos_std::sys::uptime_ms();
    while anyos_std::sys::uptime_ms().wrapping_sub(start) < CPU_TEST_MS {
        for v in sieve.iter_mut() { *v = true; }
        sieve[0] = false;
        if N > 1 { sieve[1] = false; }
        let mut i = 2;
        while i * i < N {
            if sieve[i] {
                let mut j = i * i;
                while j < N {
                    sieve[j] = false;
                    j += i;
                }
            }
            i += 1;
        }
        total += sieve.iter().filter(|&&v| v).count() as u64;
    }
    total
}
