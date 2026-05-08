//! CPU Benchmark 7 — Quantized ML Inference.
//!
//! Runs a small int8 multi-layer perceptron with ReLU activation for
//! [`CPU_TEST_MS`] milliseconds. Returns completed inference count.

use super::CPU_TEST_MS;
use alloc::vec;

/// Quantized neural-network inference benchmark.
pub fn bench_ml_inference() -> u64 {
    const IN: usize = 96;
    const HIDDEN: usize = 64;
    const OUT: usize = 16;

    let mut input = vec![0i16; IN];
    let mut hidden = vec![0i16; HIDDEN];
    let mut output = vec![0i32; OUT];
    let mut w1 = vec![0i8; IN * HIDDEN];
    let mut w2 = vec![0i8; HIDDEN * OUT];

    let mut seed = 0x1234_5678u32;
    for v in input.iter_mut() {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        *v = ((seed >> 24) as i8) as i16;
    }
    for v in w1.iter_mut() {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        *v = (seed >> 24) as i8;
    }
    for v in w2.iter_mut() {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        *v = (seed >> 24) as i8;
    }

    let mut inferences = 0u64;
    let mut checksum = 0i32;
    let start = anyos_std::sys::uptime_ms();
    while anyos_std::sys::uptime_ms().wrapping_sub(start) < CPU_TEST_MS {
        for h in 0..HIDDEN {
            let mut acc = 0i32;
            let row = h * IN;
            for i in 0..IN {
                acc = acc.wrapping_add(input[i] as i32 * w1[row + i] as i32);
            }
            acc = (acc >> 7).max(0).min(i16::MAX as i32);
            hidden[h] = acc as i16;
        }

        for o in 0..OUT {
            let mut acc = 0i32;
            let row = o * HIDDEN;
            for h in 0..HIDDEN {
                acc = acc.wrapping_add(hidden[h] as i32 * w2[row + h] as i32);
            }
            output[o] = acc >> 6;
            checksum = checksum.wrapping_add(output[o]);
        }

        let rot = (inferences as usize) & (IN - 1);
        input[rot] = input[rot].wrapping_add((checksum as i16) & 0x7F);
        inferences += 1;
    }

    core::hint::black_box((checksum, &output));
    inferences
}
