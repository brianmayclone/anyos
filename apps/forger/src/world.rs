use alloc::collections::BTreeMap;
use crate::block;
use crate::noise::{noise2d, noise3d, fbm2d};

const CHUNK_X: usize = 16;
const CHUNK_Y: usize = 256;
const CHUNK_Z: usize = 16;
const CHUNK_SIZE: usize = CHUNK_X * CHUNK_Y * CHUNK_Z;
const SEA_LEVEL: i32 = 64;

fn terrain_height(wx: i32, wz: i32, seed: u32) -> i32 {
    let seed_f = seed_offset(seed, 2048.0);
    let seed_fx = seed_offset(seed ^ 0xA511_E9B3, 4096.0);
    let seed_fz = seed_offset(seed ^ 0x63D8_35D7, 4096.0);
    let world_x = wx as f32;
    let world_z = wz as f32;

    let base_x = world_x * 0.0045 + seed_fx * 0.013;
    let base_z = world_z * 0.0045 - seed_fz * 0.011;
    let detail_x = world_x * 0.011 + seed_fx * 0.031;
    let detail_z = world_z * 0.011 - seed_fz * 0.027;
    let ridge_x = world_x * 0.007 + seed_fx * 0.019;
    let ridge_z = world_z * 0.007 + seed_fz * 0.017;
    let mask_x = world_x * 0.0026 - seed_fx * 0.007;
    let mask_z = world_z * 0.0026 + seed_fz * 0.009;

    let base = 60.0 + fbm2d(base_x, base_z, 5, 0.52) * 10.0;
    let rolling = fbm2d(detail_x, detail_z, 4, 0.55).max(-0.15) * 16.0;
    let ridge_base = (1.0 - noise2d(ridge_x, ridge_z).abs()).max(0.0);
    let ridge = ridge_base * ridge_base * ridge_base * 42.0;
    let mask_base = ((fbm2d(mask_x, mask_z, 3, 0.58) + 1.0) * 0.5).max(0.0);
    let mountain_mask = mask_base * mask_base;
    let plateau_base =
        noise2d(world_x * 0.0018 + seed_fx * 0.021, world_z * 0.0018 - seed_fz * 0.025).max(0.0);
    let plateau = plateau_base * plateau_base * 18.0;

    let mut height = base + rolling + ridge * mountain_mask + plateau;
    if height < 34.0 {
        height = 34.0;
    }
    if height > 148.0 {
        height = 148.0;
    }
    (height + 0.5) as i32
}

fn seed_offset(seed: u32, range: f32) -> f32 {
    let mixed = hash(seed);
    let unit = (mixed as f32) / (u32::MAX as f32);
    unit * range - range * 0.5
}

fn hash(mut x: u32) -> u32 {
    x = x.wrapping_mul(0x45d9f3b).wrapping_add(0x12345);
    x = ((x >> 16) ^ x).wrapping_mul(0x45d9f3b);
    x = ((x >> 16) ^ x).wrapping_mul(0x45d9f3b);
    (x >> 16) ^ x
}

fn chunk_coord(v: i32) -> i32 {
    if v < 0 { (v + 1) / 16 - 1 } else { v / 16 }
}

fn local_coord(v: i32) -> usize {
    (((v % 16) + 16) % 16) as usize
}

fn index(x: usize, y: usize, z: usize) -> usize {
    (y * CHUNK_Z + z) * CHUNK_X + x
}

pub struct Chunk {
    blocks: [u8; CHUNK_SIZE],
    pub dirty: bool,
    pub max_y: usize,
}

impl Chunk {
    pub fn new() -> Self {
        Chunk {
            blocks: [block::AIR; CHUNK_SIZE],
            dirty: true,
            max_y: 0,
        }
    }

    pub fn get(&self, x: usize, y: usize, z: usize) -> u8 {
        self.blocks[index(x, y, z)]
    }

    pub fn set(&mut self, x: usize, y: usize, z: usize, id: u8) {
        self.blocks[index(x, y, z)] = id;
        if id != block::AIR && y > self.max_y {
            self.max_y = y;
        }
    }
}

pub struct World {
    pub chunks: BTreeMap<(i32, i32), Chunk>,
    pub seed: u32,
    pub modifications: BTreeMap<(i32, i32, i32), u8>,
}

impl World {
    pub fn new(seed: u32) -> Self {
        World {
            chunks: BTreeMap::new(),
            seed,
            modifications: BTreeMap::new(),
        }
    }

    pub fn get_block(&self, x: i32, y: i32, z: i32) -> u8 {
        if y < 0 || y >= CHUNK_Y as i32 {
            return block::AIR;
        }
        let cx = chunk_coord(x);
        let cz = chunk_coord(z);
        match self.chunks.get(&(cx, cz)) {
            Some(chunk) => chunk.get(local_coord(x), y as usize, local_coord(z)),
            None => block::AIR,
        }
    }

    pub fn set_block(&mut self, x: i32, y: i32, z: i32, id: u8) {
        if y < 0 || y >= CHUNK_Y as i32 {
            return;
        }
        let cx = chunk_coord(x);
        let cz = chunk_coord(z);
        if let Some(chunk) = self.chunks.get_mut(&(cx, cz)) {
            chunk.set(local_coord(x), y as usize, local_coord(z), id);
            chunk.dirty = true;
        }
        self.modifications.insert((x, y, z), id);
        self.mark_neighbor_chunks_dirty(x, z);
    }

    fn mark_neighbor_chunks_dirty(&mut self, x: i32, z: i32) {
        let lx = local_coord(x);
        let lz = local_coord(z);
        let cx = chunk_coord(x);
        let cz = chunk_coord(z);

        if lx == 0 {
            if let Some(chunk) = self.chunks.get_mut(&(cx - 1, cz)) {
                chunk.dirty = true;
            }
        }
        if lx == CHUNK_X - 1 {
            if let Some(chunk) = self.chunks.get_mut(&(cx + 1, cz)) {
                chunk.dirty = true;
            }
        }
        if lz == 0 {
            if let Some(chunk) = self.chunks.get_mut(&(cx, cz - 1)) {
                chunk.dirty = true;
            }
        }
        if lz == CHUNK_Z - 1 {
            if let Some(chunk) = self.chunks.get_mut(&(cx, cz + 1)) {
                chunk.dirty = true;
            }
        }
    }

    pub fn is_solid(&self, x: i32, y: i32, z: i32) -> bool {
        let b = self.get_block(x, y, z);
        b != block::AIR && b != block::WATER
    }

    pub fn generate_chunk(&mut self, cx: i32, cz: i32) {
        if self.chunks.contains_key(&(cx, cz)) {
            return;
        }

        let mut chunk = Chunk::new();

        for lz in 0..CHUNK_Z {
            for lx in 0..CHUNK_X {
                let wx = cx * 16 + lx as i32;
                let wz = cz * 16 + lz as i32;

                let height = terrain_height(wx, wz, self.seed);

                // Bedrock
                chunk.set(lx, 0, lz, block::BEDROCK);

                // Stone fill
                for y in 1..height {
                    if y >= CHUNK_Y as i32 {
                        break;
                    }
                    chunk.set(lx, y as usize, lz, block::STONE);
                }

                // Top layers
                if height > SEA_LEVEL {
                    // Above sea level: dirt + grass
                    for dy in 1..=3 {
                        let y = height - dy;
                        if y >= 1 {
                            chunk.set(lx, y as usize, lz, block::DIRT);
                        }
                    }
                    if height < CHUNK_Y as i32 {
                        chunk.set(lx, height as usize, lz, block::GRASS);
                    }
                } else {
                    // At/below sea level: sand
                    for dy in 0..3 {
                        let y = height - dy;
                        if y >= 1 && (y as usize) < CHUNK_Y {
                            chunk.set(lx, y as usize, lz, block::SAND);
                        }
                    }
                }

                // Water fill
                for y in 1..=SEA_LEVEL {
                    if y as usize >= CHUNK_Y {
                        break;
                    }
                    if chunk.get(lx, y as usize, lz) == block::AIR {
                        chunk.set(lx, y as usize, lz, block::WATER);
                    }
                }

                // Caves
                for y in 5..height {
                    if y as usize >= CHUNK_Y {
                        break;
                    }
                    let cave_val = noise3d(
                        wx as f32 * 0.05,
                        y as f32 * 0.05,
                        wz as f32 * 0.05,
                    );
                    if cave_val > 0.55 {
                        chunk.set(lx, y as usize, lz, block::AIR);
                    }
                }

                // Ores
                for y in 1..height {
                    if y as usize >= CHUNK_Y {
                        break;
                    }
                    if chunk.get(lx, y as usize, lz) != block::STONE {
                        continue;
                    }
                    let h = hash(
                        (wx as u32)
                            .wrapping_mul(73856093)
                            ^ (y as u32).wrapping_mul(19349663)
                            ^ (wz as u32).wrapping_mul(83492791)
                            ^ self.seed,
                    ) % 1000;

                    if h < 3 && y < 16 {
                        chunk.set(lx, y as usize, lz, block::DIAMOND_ORE);
                    } else if h < 6 && y < 32 {
                        chunk.set(lx, y as usize, lz, block::GOLD_ORE);
                    } else if h < 15 && y < 64 {
                        chunk.set(lx, y as usize, lz, block::IRON_ORE);
                    } else if h < 25 && y < 80 {
                        chunk.set(lx, y as usize, lz, block::COAL_ORE);
                    }
                }
            }
        }

        self.chunks.insert((cx, cz), chunk);

        // Tree pass (after chunk is inserted so we can read block data)
        let mut trees: alloc::vec::Vec<(usize, usize, i32, i32)> = alloc::vec::Vec::new();

        if let Some(chunk) = self.chunks.get(&(cx, cz)) {
            for lz in 2..14usize {
                for lx in 2..14usize {
                    let wx = cx * 16 + lx as i32;
                    let wz = cz * 16 + lz as i32;
                    let h = hash(
                        (wx as u32)
                            .wrapping_mul(48611)
                            ^ (wz as u32).wrapping_mul(95467)
                            ^ self.seed.wrapping_mul(31337),
                    );
                    if h % 100 >= 2 {
                        continue;
                    }

                    // Find grass block above sea level + 2
                    for y in ((SEA_LEVEL + 2) as usize..CHUNK_Y).rev() {
                        if chunk.get(lx, y, lz) == block::GRASS {
                            let trunk_h = 4 + (hash(h ^ y as u32) % 3) as i32;
                            trees.push((lx, lz, y as i32, trunk_h));
                            break;
                        }
                    }
                }
            }
        }

        for (lx, lz, ground_y, trunk_h) in trees {
            if let Some(chunk) = self.chunks.get_mut(&(cx, cz)) {
                // Trunk
                for y in (ground_y + 1)..=(ground_y + trunk_h) {
                    if (y as usize) < CHUNK_Y {
                        chunk.set(lx, y as usize, lz, block::WOOD_LOG);
                    }
                }

                let top = ground_y + trunk_h;

                // 5x5x3 leaf crown
                for dy in -1..=1i32 {
                    let ly = top + dy;
                    if ly < 0 || ly as usize >= CHUNK_Y {
                        continue;
                    }
                    for dz in -2..=2i32 {
                        for dx in -2..=2i32 {
                            let bx = lx as i32 + dx;
                            let bz = lz as i32 + dz;
                            if bx < 0 || bx >= 16 || bz < 0 || bz >= 16 {
                                continue;
                            }
                            if chunk.get(bx as usize, ly as usize, bz as usize) == block::AIR {
                                chunk.set(bx as usize, ly as usize, bz as usize, block::LEAVES);
                            }
                        }
                    }
                }

                // 3x3x1 cap
                let cap_y = top + 2;
                if cap_y >= 0 && (cap_y as usize) < CHUNK_Y {
                    for dz in -1..=1i32 {
                        for dx in -1..=1i32 {
                            let bx = lx as i32 + dx;
                            let bz = lz as i32 + dz;
                            if bx < 0 || bx >= 16 || bz < 0 || bz >= 16 {
                                continue;
                            }
                            if chunk.get(bx as usize, cap_y as usize, bz as usize) == block::AIR {
                                chunk.set(bx as usize, cap_y as usize, bz as usize, block::LEAVES);
                            }
                        }
                    }
                }
            }
        }

        if let Some(chunk) = self.chunks.get_mut(&(cx, cz)) {
            let x0 = cx * 16;
            let z0 = cz * 16;
            let x1 = x0 + 15;
            let z1 = z0 + 15;
            for (&(mx, my, mz), &block_id) in &self.modifications {
                if mx < x0 || mx > x1 || mz < z0 || mz > z1 {
                    continue;
                }
                if my < 0 || my >= CHUNK_Y as i32 {
                    continue;
                }
                chunk.set(local_coord(mx), my as usize, local_coord(mz), block_id);
            }
        }
    }

    pub fn ensure_chunks_around(&mut self, px: i32, pz: i32, radius: i32) {
        let cx = chunk_coord(px);
        let cz = chunk_coord(pz);
        for dz in -radius..=radius {
            for dx in -radius..=radius {
                self.generate_chunk(cx + dx, cz + dz);
            }
        }
    }
}
