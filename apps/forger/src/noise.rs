#![allow(clippy::many_single_chars)]

// Standard permutation table (Ken Perlin's original)
#[rustfmt::skip]
const PERM: [u8; 256] = [
    151,160,137, 91, 90, 15,131, 13,201, 95, 96, 53,194,233,  7,225,
    140, 36,103, 30, 69,142,  8, 99, 37,240, 21, 10, 23,190,  6,148,
    247,120,234, 75,  0, 26,197, 62, 94,252,219,203,117, 35, 11, 32,
     57,177, 33, 88,237,149, 56, 87,174, 20,125,136,171,168, 68,175,
     74,165, 71,134,139, 48, 27,166, 77,146,158,231, 83,111,229,122,
     60,211,133,230,220,105, 92, 41, 55, 46,245, 40,244,102,143, 54,
     65, 25, 63,161,  1,216, 80, 73,209, 76,132,187,208, 89, 18,169,
    200,196,135,130,116,188,159, 86,164,100,109,198,173,186,  3, 64,
     52,217,226,250,124,123,  5,202, 38,147,118,126,255, 82, 85,212,
    207,206, 59,227, 47, 16, 58, 17,182,189, 28, 42,223,183,170,213,
    119,248,152,  2, 44,154,163, 70,221,153,101,155,167, 43,172,  9,
    129, 22, 39,253, 19, 98,108,110, 79,113,224,232,178,185,112,104,
    218,246, 97,228,251, 34,242,193,238,210,144, 12,191,179,162,241,
     81, 51,145,235,249, 14,239,107, 49,192,214, 31,181,199,106,157,
    184, 84,204,176,115,121, 50, 45,127,  4,150,254,138,236,205, 93,
    222,114, 67, 29, 24, 72,243,141,128,195, 78, 66,215, 61,156,180,
];

// 12 gradient vectors for 3D simplex noise
const GRAD3: [[f32; 3]; 12] = [
    [ 1.0, 1.0, 0.0], [-1.0, 1.0, 0.0], [ 1.0,-1.0, 0.0], [-1.0,-1.0, 0.0],
    [ 1.0, 0.0, 1.0], [-1.0, 0.0, 1.0], [ 1.0, 0.0,-1.0], [-1.0, 0.0,-1.0],
    [ 0.0, 1.0, 1.0], [ 0.0,-1.0, 1.0], [ 0.0, 1.0,-1.0], [ 0.0,-1.0,-1.0],
];

// Doubled permutation table for wrapping without modulo
const PERM512: [u16; 512] = {
    let mut table = [0u16; 512];
    let mut i = 0;
    while i < 512 {
        table[i] = PERM[i & 255] as u16;
        i += 1;
    }
    table
};

const PERM_MOD12: [u8; 512] = {
    let mut table = [0u8; 512];
    let mut i = 0;
    while i < 512 {
        table[i] = (PERM[i & 255] % 12) as u8;
        i += 1;
    }
    table
};

#[inline]
fn floor(x: f32) -> i32 {
    let xi = x as i32;
    if (xi as f32) > x { xi - 1 } else { xi }
}

#[inline]
fn dot2(g: &[f32; 3], x: f32, y: f32) -> f32 {
    g[0] * x + g[1] * y
}

#[inline]
fn dot3(g: &[f32; 3], x: f32, y: f32, z: f32) -> f32 {
    g[0] * x + g[1] * y + g[2] * z
}

/// 2D simplex noise, returns value in [-1, 1].
pub fn noise2d(xin: f32, yin: f32) -> f32 {
    const F2: f32 = 0.36602540378; // (sqrt(3) - 1) / 2
    const G2: f32 = 0.21132486540; // (3 - sqrt(3)) / 6

    // Skew input space to determine which simplex cell we're in
    let s = (xin + yin) * F2;
    let i = floor(xin + s);
    let j = floor(yin + s);

    let t = (i + j) as f32 * G2;
    // Unskew the cell origin back to (x,y) space
    let x0 = xin - (i as f32 - t);
    let y0 = yin - (j as f32 - t);

    // Determine which simplex we are in
    let (i1, j1) = if x0 > y0 { (1, 0) } else { (0, 1) };

    let x1 = x0 - i1 as f32 + G2;
    let y1 = y0 - j1 as f32 + G2;
    let x2 = x0 - 1.0 + 2.0 * G2;
    let y2 = y0 - 1.0 + 2.0 * G2;

    let ii = (i & 255) as usize;
    let jj = (j & 255) as usize;
    let gi0 = PERM_MOD12[ii + PERM512[jj] as usize] as usize;
    let gi1 = PERM_MOD12[ii + i1 + PERM512[jj + j1] as usize] as usize;
    let gi2 = PERM_MOD12[ii + 1 + PERM512[jj + 1] as usize] as usize;

    // Calculate contributions from the three corners
    let mut n0 = 0.0;
    let t0 = 0.5 - x0 * x0 - y0 * y0;
    if t0 >= 0.0 {
        let t0 = t0 * t0;
        n0 = t0 * t0 * dot2(&GRAD3[gi0], x0, y0);
    }

    let mut n1 = 0.0;
    let t1 = 0.5 - x1 * x1 - y1 * y1;
    if t1 >= 0.0 {
        let t1 = t1 * t1;
        n1 = t1 * t1 * dot2(&GRAD3[gi1], x1, y1);
    }

    let mut n2 = 0.0;
    let t2 = 0.5 - x2 * x2 - y2 * y2;
    if t2 >= 0.0 {
        let t2 = t2 * t2;
        n2 = t2 * t2 * dot2(&GRAD3[gi2], x2, y2);
    }

    // Scale to [-1, 1]
    70.0 * (n0 + n1 + n2)
}

/// 3D simplex noise, returns value in [-1, 1].
pub fn noise3d(xin: f32, yin: f32, zin: f32) -> f32 {
    const F3: f32 = 1.0 / 3.0;
    const G3: f32 = 1.0 / 6.0;

    let s = (xin + yin + zin) * F3;
    let i = floor(xin + s);
    let j = floor(yin + s);
    let k = floor(zin + s);

    let t = (i + j + k) as f32 * G3;
    let x0 = xin - (i as f32 - t);
    let y0 = yin - (j as f32 - t);
    let z0 = zin - (k as f32 - t);

    // Determine which simplex we are in
    let (i1, j1, k1, i2, j2, k2) = if x0 >= y0 {
        if y0 >= z0 {
            (1, 0, 0, 1, 1, 0)
        } else if x0 >= z0 {
            (1, 0, 0, 1, 0, 1)
        } else {
            (0, 0, 1, 1, 0, 1)
        }
    } else {
        // x0 < y0
        if y0 < z0 {
            (0, 0, 1, 0, 1, 1)
        } else if x0 < z0 {
            (0, 1, 0, 0, 1, 1)
        } else {
            (0, 1, 0, 1, 1, 0)
        }
    };

    let x1 = x0 - i1 as f32 + G3;
    let y1 = y0 - j1 as f32 + G3;
    let z1 = z0 - k1 as f32 + G3;
    let x2 = x0 - i2 as f32 + 2.0 * G3;
    let y2 = y0 - j2 as f32 + 2.0 * G3;
    let z2 = z0 - k2 as f32 + 2.0 * G3;
    let x3 = x0 - 1.0 + 3.0 * G3;
    let y3 = y0 - 1.0 + 3.0 * G3;
    let z3 = z0 - 1.0 + 3.0 * G3;

    let ii = (i & 255) as usize;
    let jj = (j & 255) as usize;
    let kk = (k & 255) as usize;
    let gi0 = PERM_MOD12[ii + PERM512[jj + PERM512[kk] as usize] as usize] as usize;
    let gi1 = PERM_MOD12[ii + i1 + PERM512[jj + j1 + PERM512[kk + k1] as usize] as usize] as usize;
    let gi2 = PERM_MOD12[ii + i2 + PERM512[jj + j2 + PERM512[kk + k2] as usize] as usize] as usize;
    let gi3 = PERM_MOD12[ii + 1 + PERM512[jj + 1 + PERM512[kk + 1] as usize] as usize] as usize;

    let mut n0 = 0.0;
    let t0 = 0.6 - x0 * x0 - y0 * y0 - z0 * z0;
    if t0 >= 0.0 {
        let t0 = t0 * t0;
        n0 = t0 * t0 * dot3(&GRAD3[gi0], x0, y0, z0);
    }

    let mut n1 = 0.0;
    let t1 = 0.6 - x1 * x1 - y1 * y1 - z1 * z1;
    if t1 >= 0.0 {
        let t1 = t1 * t1;
        n1 = t1 * t1 * dot3(&GRAD3[gi1], x1, y1, z1);
    }

    let mut n2 = 0.0;
    let t2 = 0.6 - x2 * x2 - y2 * y2 - z2 * z2;
    if t2 >= 0.0 {
        let t2 = t2 * t2;
        n2 = t2 * t2 * dot3(&GRAD3[gi2], x2, y2, z2);
    }

    let mut n3 = 0.0;
    let t3 = 0.6 - x3 * x3 - y3 * y3 - z3 * z3;
    if t3 >= 0.0 {
        let t3 = t3 * t3;
        n3 = t3 * t3 * dot3(&GRAD3[gi3], x3, y3, z3);
    }

    // Scale to [-1, 1]
    32.0 * (n0 + n1 + n2 + n3)
}

/// Fractal Brownian Motion using 2D simplex noise.
/// Sums multiple octaves with increasing frequency and decreasing amplitude.
pub fn fbm2d(x: f32, y: f32, octaves: u32, persistence: f32) -> f32 {
    let mut total = 0.0f32;
    let mut frequency = 1.0f32;
    let mut amplitude = 1.0f32;
    let mut max_amplitude = 0.0f32;

    let mut i = 0u32;
    while i < octaves {
        total += noise2d(x * frequency, y * frequency) * amplitude;
        max_amplitude += amplitude;
        amplitude *= persistence;
        frequency *= 2.0;
        i += 1;
    }

    total / max_amplitude
}
