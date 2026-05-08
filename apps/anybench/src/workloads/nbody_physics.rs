//! CPU Benchmark 11 — N-Body Physics.
//!
//! Simulates pairwise particle interactions with fixed-point arithmetic for
//! [`CPU_TEST_MS`] milliseconds. Returns pair interactions evaluated.

use super::CPU_TEST_MS;
use alloc::vec;

#[derive(Clone, Copy)]
struct Body {
    x: i32,
    y: i32,
    z: i32,
    vx: i32,
    vy: i32,
    vz: i32,
    mass: i32,
}

/// Fixed-point N-body interaction benchmark.
pub fn bench_nbody_physics() -> u64 {
    const N: usize = 96;
    let mut bodies = vec![
        Body {
            x: 0,
            y: 0,
            z: 0,
            vx: 0,
            vy: 0,
            vz: 0,
            mass: 1,
        };
        N
    ];

    let mut seed = 0xCAFEBABEu32;
    for b in bodies.iter_mut() {
        seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
        b.x = ((seed >> 12) as i32 & 0x0FFF) - 2048;
        seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
        b.y = ((seed >> 12) as i32 & 0x0FFF) - 2048;
        seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
        b.z = ((seed >> 12) as i32 & 0x0FFF) - 2048;
        b.mass = 16 + ((seed >> 24) as i32 & 31);
    }

    let mut interactions = 0u64;
    let start = anyos_std::sys::uptime_ms();
    while anyos_std::sys::uptime_ms().wrapping_sub(start) < CPU_TEST_MS {
        for i in 0..N {
            for j in i + 1..N {
                let dx = bodies[j].x - bodies[i].x;
                let dy = bodies[j].y - bodies[i].y;
                let dz = bodies[j].z - bodies[i].z;
                let dist2 =
                    (dx as i64 * dx as i64 + dy as i64 * dy as i64 + dz as i64 * dz as i64 + 4096)
                        as i32;
                let inv = 1_048_576 / ((dist2 >> 8) + 1);
                let fx = (dx * inv) >> 12;
                let fy = (dy * inv) >> 12;
                let fz = (dz * inv) >> 12;
                let mi = bodies[i].mass;
                let mj = bodies[j].mass;

                bodies[i].vx = bodies[i].vx.wrapping_add((fx * mj) >> 5);
                bodies[i].vy = bodies[i].vy.wrapping_add((fy * mj) >> 5);
                bodies[i].vz = bodies[i].vz.wrapping_add((fz * mj) >> 5);
                bodies[j].vx = bodies[j].vx.wrapping_sub((fx * mi) >> 5);
                bodies[j].vy = bodies[j].vy.wrapping_sub((fy * mi) >> 5);
                bodies[j].vz = bodies[j].vz.wrapping_sub((fz * mi) >> 5);
                interactions += 1;
            }
        }

        for b in bodies.iter_mut() {
            b.x = b.x.wrapping_add(b.vx >> 8);
            b.y = b.y.wrapping_add(b.vy >> 8);
            b.z = b.z.wrapping_add(b.vz >> 8);
            b.vx -= b.vx >> 7;
            b.vy -= b.vy >> 7;
            b.vz -= b.vz >> 7;
        }
    }

    core::hint::black_box(&bodies);
    interactions
}
