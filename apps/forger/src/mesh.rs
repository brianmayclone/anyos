extern crate alloc;
use alloc::vec::Vec;

use crate::block;
use crate::textures::{self, Face};
use crate::world::World;

pub const FLOATS_PER_VERTEX: usize = 6;

const FACE_DIRS: [(i32, i32, i32); 6] = [
    (0, 1, 0),
    (0, -1, 0),
    (1, 0, 0),
    (-1, 0, 0),
    (0, 0, 1),
    (0, 0, -1),
];

const FACE_LIGHT: [f32; 6] = [1.0, 0.5, 0.8, 0.8, 0.7, 0.7];

pub struct ChunkMesh {
    pub vertices: Vec<f32>,
    pub vertex_count: u32,
}

pub fn build_chunk_mesh(world: &World, cx: i32, cz: i32) -> ChunkMesh {
    let mut vertices = Vec::new();
    let mut vertex_count: u32 = 0;

    let base_x = cx * 16;
    let base_z = cz * 16;

    let max_y = world.chunks.get(&(cx, cz)).map_or(0, |c| c.max_y + 1).min(256);

    for ly in 0..max_y {
        for lz in 0..16 {
            for lx in 0..16 {
                let wx = base_x + lx;
                let wy = ly as i32;
                let wz = base_z + lz;

                let id = world.get_block(wx, wy, wz);
                if id == block::AIR {
                    continue;
                }

                for face in 0..6usize {
                    let (dx, dy, dz) = FACE_DIRS[face];
                    let nx = wx + dx;
                    let ny = wy + dy;
                    let nz = wz + dz;

                    let neighbor = world.get_block(nx, ny, nz);

                    if neighbor == id {
                        continue;
                    }

                    if !block::is_transparent(neighbor) {
                        continue;
                    }

                    let tex_face = match face {
                        0 => Face::Top,
                        1 => Face::Bottom,
                        2 => Face::East,
                        3 => Face::West,
                        4 => Face::South,
                        _ => Face::North,
                    };

                    let tex_id = textures::face_block_id(id, tex_face);
                    let (u0, v0, u1, v1) = textures::block_uv(tex_id);

                    let positions = face_vertices(face, wx as f32, wy as f32, wz as f32);
                    let uvs = [
                        (u0, v0),
                        (u1, v0),
                        (u1, v1),
                        (u0, v0),
                        (u1, v1),
                        (u0, v1),
                    ];
                    let light = FACE_LIGHT[face];

                    for i in 0..6 {
                        vertices.push(positions[i][0]);
                        vertices.push(positions[i][1]);
                        vertices.push(positions[i][2]);
                        vertices.push(uvs[i].0);
                        vertices.push(uvs[i].1);
                        vertices.push(light);
                    }
                    vertex_count += 6;
                }
            }
        }
    }

    ChunkMesh {
        vertices,
        vertex_count,
    }
}

pub fn face_vertices(face: usize, x: f32, y: f32, z: f32) -> [[f32; 3]; 6] {
    match face {
        // Top (+Y) — quad at y+1
        0 => [
            [x, y + 1.0, z],
            [x + 1.0, y + 1.0, z],
            [x + 1.0, y + 1.0, z + 1.0],
            [x, y + 1.0, z],
            [x + 1.0, y + 1.0, z + 1.0],
            [x, y + 1.0, z + 1.0],
        ],
        // Bottom (-Y) — quad at y
        1 => [
            [x, y, z + 1.0],
            [x + 1.0, y, z + 1.0],
            [x + 1.0, y, z],
            [x, y, z + 1.0],
            [x + 1.0, y, z],
            [x, y, z],
        ],
        // East (+X) — quad at x+1
        2 => [
            [x + 1.0, y, z],
            [x + 1.0, y, z + 1.0],
            [x + 1.0, y + 1.0, z + 1.0],
            [x + 1.0, y, z],
            [x + 1.0, y + 1.0, z + 1.0],
            [x + 1.0, y + 1.0, z],
        ],
        // West (-X) — quad at x
        3 => [
            [x, y, z + 1.0],
            [x, y, z],
            [x, y + 1.0, z],
            [x, y, z + 1.0],
            [x, y + 1.0, z],
            [x, y + 1.0, z + 1.0],
        ],
        // South (+Z) — quad at z+1
        4 => [
            [x + 1.0, y, z + 1.0],
            [x, y, z + 1.0],
            [x, y + 1.0, z + 1.0],
            [x + 1.0, y, z + 1.0],
            [x, y + 1.0, z + 1.0],
            [x + 1.0, y + 1.0, z + 1.0],
        ],
        // North (-Z) — quad at z
        _ => [
            [x, y, z],
            [x + 1.0, y, z],
            [x + 1.0, y + 1.0, z],
            [x, y, z],
            [x + 1.0, y + 1.0, z],
            [x, y + 1.0, z],
        ],
    }
}
