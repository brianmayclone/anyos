use crate::block;

pub const TEX_SIZE: usize = 16;
pub const ATLAS_COLS: usize = 5;
pub const ATLAS_ROWS: usize = 5;
pub const ATLAS_W: usize = ATLAS_COLS * TEX_SIZE;
pub const ATLAS_H: usize = ATLAS_ROWS * TEX_SIZE;

#[derive(Clone, Copy, PartialEq)]
pub enum Face {
    Top,
    Bottom,
    North,
    South,
    East,
    West,
}

fn simple_hash(px: usize, py: usize, id: u8) -> i8 {
    let mut h = (px as u32)
        .wrapping_mul(374761393)
        .wrapping_add((py as u32).wrapping_mul(668265263))
        .wrapping_add((id as u32).wrapping_mul(2654435761));
    h ^= h >> 13;
    h = h.wrapping_mul(1274126177);
    h ^= h >> 16;
    ((h & 31) as i8) - 16
}

fn clamp_u8(v: i16) -> u8 {
    if v < 0 {
        0
    } else if v > 255 {
        255
    } else {
        v as u8
    }
}

pub fn gen_pixel(id: u8, px: usize, py: usize) -> (u8, u8, u8, u8) {
    let v = simple_hash(px, py, id) as i16;

    match id {
        block::AIR => (0, 0, 0, 0),

        block::GRASS => {
            let r = clamp_u8(60 + v / 2);
            let g = clamp_u8(120 + v);
            (r, g, 30, 255)
        }

        block::DIRT => {
            let base = clamp_u8(130 + v) as i16;
            (base as u8, clamp_u8(base * 3 / 4), clamp_u8(base / 2), 255)
        }

        block::STONE => {
            let base = clamp_u8(128 + v);
            (base, base, base, 255)
        }

        block::SAND => {
            let base = clamp_u8(210 + v);
            (
                base,
                clamp_u8(base as i16 - 15),
                clamp_u8(base as i16 - 70),
                255,
            )
        }

        block::GRAVEL => {
            let base = clamp_u8(140 + v * 2);
            (base, base, base, 255)
        }

        block::WOOD_LOG => {
            let grain: i16 = if px % 3 == 0 { 10 } else { 0 };
            let base = clamp_u8(120 + v + grain);
            (
                base,
                clamp_u8(base as i16 * 3 / 4),
                clamp_u8(base as i16 / 2),
                255,
            )
        }

        block::LEAVES => {
            let r = clamp_u8(40 + v / 2);
            let g = clamp_u8(100 + v);
            let b = clamp_u8(30 + v / 2);
            (r, g, b, 200)
        }

        block::WATER => (30, 60, clamp_u8(180 + v), 160),

        block::BEDROCK => {
            let base = clamp_u8(50 + v);
            (base, base, base, 255)
        }

        block::COAL_ORE => {
            let base = clamp_u8(128 + v);
            if (px + py) % 4 == 0 {
                (30, 30, 30, 255)
            } else {
                (base, base, base, 255)
            }
        }

        block::IRON_ORE => {
            let base = clamp_u8(128 + v);
            if (px + py) % 4 == 0 {
                (200, 170, 130, 255)
            } else {
                (base, base, base, 255)
            }
        }

        block::GOLD_ORE => {
            let base = clamp_u8(128 + v);
            if (px + py) % 4 == 0 {
                (255, 215, 0, 255)
            } else {
                (base, base, base, 255)
            }
        }

        block::DIAMOND_ORE => {
            let base = clamp_u8(128 + v);
            if (px + py) % 4 == 0 {
                (0, 230, 230, 255)
            } else {
                (base, base, base, 255)
            }
        }

        block::WOOD_PLANKS => {
            let grain: i16 = if py % 4 == 0 { -10 } else { 0 };
            let base = clamp_u8(180 + v + grain);
            (
                base,
                clamp_u8(base as i16 * 3 / 4),
                clamp_u8(base as i16 / 2),
                255,
            )
        }

        block::BRICK => {
            let mortar = py % 4 == 0 || (px + if (py / 4) % 2 == 0 { 0 } else { 4 }) % 8 == 0;
            if mortar {
                (200, 200, 190, 255)
            } else {
                let base = clamp_u8(160 + v);
                (
                    base,
                    clamp_u8(base as i16 / 2),
                    clamp_u8(base as i16 / 3),
                    255,
                )
            }
        }

        block::COBBLESTONE => {
            let base = clamp_u8(128 + v * 2);
            (base, base, base, 255)
        }

        block::SNOW => {
            let base = clamp_u8(240 + v / 2);
            (base, base, base, 255)
        }

        block::GLASS => {
            let base = clamp_u8(200 + v / 2);
            (base, base, base, 60)
        }

        block::CLAY => {
            let base = clamp_u8(160 + v);
            (
                base,
                clamp_u8(base as i16 - 10),
                clamp_u8(base as i16 - 20),
                255,
            )
        }

        block::TORCH => {
            if px < 6 || px > 9 {
                (0, 0, 0, 0)
            } else if py < 3 {
                let r = clamp_u8(255 + v / 2);
                let g = clamp_u8(200 + v);
                (r, g, 50, 255)
            } else {
                let base = clamp_u8(140 + v);
                (
                    base,
                    clamp_u8(base as i16 * 3 / 4),
                    clamp_u8(base as i16 / 2),
                    255,
                )
            }
        }

        _ => (255, 0, 255, 255),
    }
}

pub fn face_block_id(id: u8, face: Face) -> u8 {
    match id {
        block::GRASS => match face {
            Face::Bottom => block::DIRT,
            _ => block::GRASS,
        },
        _ => id,
    }
}

pub fn block_uv(id: u8) -> (f32, f32, f32, f32) {
    let col = (id as usize) % ATLAS_COLS;
    let row = (id as usize) / ATLAS_COLS;
    let u0 = col as f32 / ATLAS_COLS as f32;
    let v0 = row as f32 / ATLAS_ROWS as f32;
    let u1 = (col + 1) as f32 / ATLAS_COLS as f32;
    let v1 = (row + 1) as f32 / ATLAS_ROWS as f32;
    (u0, v0, u1, v1)
}

pub fn generate_atlas() -> [u8; ATLAS_W * ATLAS_H * 4] {
    let mut atlas = [0u8; ATLAS_W * ATLAS_H * 4];

    for id in 0..block::BLOCK_COUNT {
        let col = id % ATLAS_COLS;
        let row = id / ATLAS_COLS;
        let base_x = col * TEX_SIZE;
        let base_y = row * TEX_SIZE;

        for py in 0..TEX_SIZE {
            for px in 0..TEX_SIZE {
                let (r, g, b, a) = gen_pixel(id as u8, px, py);
                let idx = ((base_y + py) * ATLAS_W + (base_x + px)) * 4;
                atlas[idx] = r;
                atlas[idx + 1] = g;
                atlas[idx + 2] = b;
                atlas[idx + 3] = a;
            }
        }
    }

    atlas
}
