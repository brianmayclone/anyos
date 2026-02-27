//! CPU Benchmark 4 — Dense Matrix Multiplication.
//!
//! 64×64 integer matrix multiply (C = A × B), repeated for [`CPU_TEST_MS`]
//! milliseconds. Returns total multiply-add operations performed.

use alloc::vec;
use super::CPU_TEST_MS;

/// Dense 64×64 integer matrix multiplication benchmark.
pub fn bench_matrix_multiply() -> u64 {
    const N: usize = 64;
    let mut a = vec![0i32; N * N];
    let mut b = vec![0i32; N * N];
    let mut c = vec![0i32; N * N];

    // Fill with pseudo-random data
    let mut seed: u32 = 12345;
    for i in 0..N * N {
        seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
        a[i] = (seed >> 16) as i32 & 0xFF;
        seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
        b[i] = (seed >> 16) as i32 & 0xFF;
    }

    let mut ops: u64 = 0;
    let start = anyos_std::sys::uptime_ms();
    while anyos_std::sys::uptime_ms().wrapping_sub(start) < CPU_TEST_MS {
        for i in 0..N {
            for j in 0..N {
                let mut sum = 0i32;
                for k in 0..N {
                    sum = sum.wrapping_add(a[i * N + k].wrapping_mul(b[k * N + j]));
                }
                c[i * N + j] = sum;
            }
        }
        ops += (N * N * N * 2) as u64; // multiply + add per element
    }
    core::hint::black_box(&c);
    ops
}
