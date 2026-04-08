extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

use crate::block;
use crate::textures::{self, Face};
use crate::world::World;

pub const FLOATS_PER_VERTEX: usize = 10;

const CHUNK_X: usize = 16;
const CHUNK_Y: usize = 256;
const CHUNK_Z: usize = 16;

const FACE_DIRS: [(i32, i32, i32); 6] = [
    (0, 1, 0),
    (0, -1, 0),
    (1, 0, 0),
    (-1, 0, 0),
    (0, 0, 1),
    (0, 0, -1),
];

const FACE_NORMALS: [[f32; 3]; 6] = [
    [0.0, 1.0, 0.0],
    [0.0, -1.0, 0.0],
    [1.0, 0.0, 0.0],
    [-1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0],
    [0.0, 0.0, -1.0],
];

// Kept as a light occlusion / artistic floor, not as the main sun term.
const FACE_LIGHT: [f32; 6] = [1.0, 0.65, 0.95, 0.95, 0.92, 0.92];

#[derive(Clone, Copy, PartialEq, Eq)]
struct MaskCell {
    block_id: u8,
    tex_id: u8,
    face: usize,
}

pub struct ChunkMesh {
    pub vertices: Vec<f32>,
    pub vertex_count: u32,
}

pub fn build_chunk_mesh(world: &World, cx: i32, cz: i32) -> ChunkMesh {
    let mut vertices = Vec::with_capacity(64 * 1024);
    let mut vertex_count: u32 = 0;

    let base_x = cx * CHUNK_X as i32;
    let base_z = cz * CHUNK_Z as i32;
    let max_y = world.chunks.get(&(cx, cz)).map_or(0, |c| c.max_y + 1).min(CHUNK_Y);

    if max_y == 0 {
        return ChunkMesh {
            vertices,
            vertex_count,
        };
    }

    // Top / bottom faces: scan X/Z for each Y layer.
    for y in 0..max_y {
        let wy = y as i32;

        let mut top_mask = vec![None; CHUNK_X * CHUNK_Z];
        let mut bottom_mask = vec![None; CHUNK_X * CHUNK_Z];

        for z in 0..CHUNK_Z {
            for x in 0..CHUNK_X {
                let wx = base_x + x as i32;
                let wz = base_z + z as i32;
                let idx = z * CHUNK_X + x;
                top_mask[idx] = visible_face_cell(world, wx, wy, wz, 0);
                bottom_mask[idx] = visible_face_cell(world, wx, wy, wz, 1);
            }
        }

        greedy_mask(CHUNK_X, CHUNK_Z, &top_mask, |x, z, w, h, cell| {
            emit_top_quad(&mut vertices, &mut vertex_count, base_x + x as i32, wy, base_z + z as i32, w, h, cell);
        });
        greedy_mask(CHUNK_X, CHUNK_Z, &bottom_mask, |x, z, w, h, cell| {
            emit_bottom_quad(&mut vertices, &mut vertex_count, base_x + x as i32, wy, base_z + z as i32, w, h, cell);
        });
    }

    // East / west faces: scan Z/Y for each X slice.
    for x in 0..CHUNK_X {
        let wx = base_x + x as i32;
        let mut east_mask = vec![None; CHUNK_Z * max_y];
        let mut west_mask = vec![None; CHUNK_Z * max_y];

        for y in 0..max_y {
            let wy = y as i32;
            for z in 0..CHUNK_Z {
                let wz = base_z + z as i32;
                let idx = y * CHUNK_Z + z;
                east_mask[idx] = visible_face_cell(world, wx, wy, wz, 2);
                west_mask[idx] = visible_face_cell(world, wx, wy, wz, 3);
            }
        }

        greedy_mask(CHUNK_Z, max_y, &east_mask, |z, y, d, h, cell| {
            emit_east_quad(&mut vertices, &mut vertex_count, wx, y as i32, base_z + z as i32, d, h, cell);
        });
        greedy_mask(CHUNK_Z, max_y, &west_mask, |z, y, d, h, cell| {
            emit_west_quad(&mut vertices, &mut vertex_count, wx, y as i32, base_z + z as i32, d, h, cell);
        });
    }

    // South / north faces: scan X/Y for each Z slice.
    for z in 0..CHUNK_Z {
        let wz = base_z + z as i32;
        let mut south_mask = vec![None; CHUNK_X * max_y];
        let mut north_mask = vec![None; CHUNK_X * max_y];

        for y in 0..max_y {
            let wy = y as i32;
            for x in 0..CHUNK_X {
                let wx = base_x + x as i32;
                let idx = y * CHUNK_X + x;
                south_mask[idx] = visible_face_cell(world, wx, wy, wz, 4);
                north_mask[idx] = visible_face_cell(world, wx, wy, wz, 5);
            }
        }

        greedy_mask(CHUNK_X, max_y, &south_mask, |x, y, w, h, cell| {
            emit_south_quad(&mut vertices, &mut vertex_count, base_x + x as i32, y as i32, wz, w, h, cell);
        });
        greedy_mask(CHUNK_X, max_y, &north_mask, |x, y, w, h, cell| {
            emit_north_quad(&mut vertices, &mut vertex_count, base_x + x as i32, y as i32, wz, w, h, cell);
        });
    }

    ChunkMesh {
        vertices,
        vertex_count,
    }
}

fn visible_face_cell(world: &World, wx: i32, wy: i32, wz: i32, face: usize) -> Option<MaskCell> {
    let id = world.get_block(wx, wy, wz);
    if id == block::AIR {
        return None;
    }

    let (dx, dy, dz) = FACE_DIRS[face];
    let neighbor = world.get_block(wx + dx, wy + dy, wz + dz);
    if neighbor == id || !block::is_transparent(neighbor) {
        return None;
    }

    let tex_face = match face {
        0 => Face::Top,
        1 => Face::Bottom,
        2 => Face::East,
        3 => Face::West,
        4 => Face::South,
        _ => Face::North,
    };

    Some(MaskCell {
        block_id: id,
        tex_id: textures::face_block_id(id, tex_face),
        face,
    })
}

fn greedy_mask<F>(width: usize, height: usize, mask: &[Option<MaskCell>], mut emit: F)
where
    F: FnMut(usize, usize, usize, usize, MaskCell),
{
    let mut used = vec![false; width * height];

    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            let Some(cell) = mask[idx] else { continue };
            if used[idx] {
                continue;
            }

            let mut quad_w = 1usize;
            while x + quad_w < width {
                let next_idx = y * width + (x + quad_w);
                if used[next_idx] || mask[next_idx] != Some(cell) {
                    break;
                }
                quad_w += 1;
            }

            let mut quad_h = 1usize;
            'grow: while y + quad_h < height {
                for dx in 0..quad_w {
                    let next_idx = (y + quad_h) * width + (x + dx);
                    if used[next_idx] || mask[next_idx] != Some(cell) {
                        break 'grow;
                    }
                }
                quad_h += 1;
            }

            for dy in 0..quad_h {
                for dx in 0..quad_w {
                    used[(y + dy) * width + (x + dx)] = true;
                }
            }

            emit(x, y, quad_w, quad_h, cell);
        }
    }
}

fn push_vertex(
    vertices: &mut Vec<f32>,
    pos: [f32; 3],
    uv: (f32, f32),
    light: f32,
    normal: [f32; 3],
    translucency: f32,
) {
    vertices.push(pos[0]);
    vertices.push(pos[1]);
    vertices.push(pos[2]);
    vertices.push(uv.0);
    vertices.push(uv.1);
    vertices.push(light);
    vertices.push(normal[0]);
    vertices.push(normal[1]);
    vertices.push(normal[2]);
    vertices.push(translucency);
}

fn emit_quad(
    vertices: &mut Vec<f32>,
    vertex_count: &mut u32,
    positions: [[f32; 3]; 6],
    cell: MaskCell,
) {
    let (u0, v0, u1, v1) = textures::block_uv(cell.tex_id);
    let uvs = [
        (u0, v0),
        (u1, v1),
        (u1, v0),
        (u0, v0),
        (u0, v1),
        (u1, v1),
    ];
    let light = FACE_LIGHT[cell.face];
    let normal = FACE_NORMALS[cell.face];
    let translucency = block::translucency(cell.block_id);

    for i in 0..6 {
        push_vertex(vertices, positions[i], uvs[i], light, normal, translucency);
    }
    *vertex_count += 6;
}

fn emit_top_quad(
    vertices: &mut Vec<f32>,
    vertex_count: &mut u32,
    x: i32,
    y: i32,
    z: i32,
    w: usize,
    h: usize,
    cell: MaskCell,
) {
    let x0 = x as f32;
    let x1 = (x + w as i32) as f32;
    let yy = (y + 1) as f32;
    let z0 = z as f32;
    let z1 = (z + h as i32) as f32;
    emit_quad(
        vertices,
        vertex_count,
        [
            [x0, yy, z0],
            [x1, yy, z1],
            [x1, yy, z0],
            [x0, yy, z0],
            [x0, yy, z1],
            [x1, yy, z1],
        ],
        cell,
    );
}

fn emit_bottom_quad(
    vertices: &mut Vec<f32>,
    vertex_count: &mut u32,
    x: i32,
    y: i32,
    z: i32,
    w: usize,
    h: usize,
    cell: MaskCell,
) {
    let x0 = x as f32;
    let x1 = (x + w as i32) as f32;
    let yy = y as f32;
    let z0 = z as f32;
    let z1 = (z + h as i32) as f32;
    emit_quad(
        vertices,
        vertex_count,
        [
            [x0, yy, z1],
            [x1, yy, z0],
            [x1, yy, z1],
            [x0, yy, z1],
            [x0, yy, z0],
            [x1, yy, z0],
        ],
        cell,
    );
}

fn emit_east_quad(
    vertices: &mut Vec<f32>,
    vertex_count: &mut u32,
    x: i32,
    y: i32,
    z: i32,
    depth: usize,
    height: usize,
    cell: MaskCell,
) {
    let xx = (x + 1) as f32;
    let y0 = y as f32;
    let y1 = (y + height as i32) as f32;
    let z0 = z as f32;
    let z1 = (z + depth as i32) as f32;
    emit_quad(
        vertices,
        vertex_count,
        [
            [xx, y0, z0],
            [xx, y1, z1],
            [xx, y0, z1],
            [xx, y0, z0],
            [xx, y1, z0],
            [xx, y1, z1],
        ],
        cell,
    );
}

fn emit_west_quad(
    vertices: &mut Vec<f32>,
    vertex_count: &mut u32,
    x: i32,
    y: i32,
    z: i32,
    depth: usize,
    height: usize,
    cell: MaskCell,
) {
    let xx = x as f32;
    let y0 = y as f32;
    let y1 = (y + height as i32) as f32;
    let z0 = z as f32;
    let z1 = (z + depth as i32) as f32;
    emit_quad(
        vertices,
        vertex_count,
        [
            [xx, y0, z1],
            [xx, y1, z0],
            [xx, y0, z0],
            [xx, y0, z1],
            [xx, y1, z1],
            [xx, y1, z0],
        ],
        cell,
    );
}

fn emit_south_quad(
    vertices: &mut Vec<f32>,
    vertex_count: &mut u32,
    x: i32,
    y: i32,
    z: i32,
    width: usize,
    height: usize,
    cell: MaskCell,
) {
    let x0 = x as f32;
    let x1 = (x + width as i32) as f32;
    let y0 = y as f32;
    let y1 = (y + height as i32) as f32;
    let zz = (z + 1) as f32;
    emit_quad(
        vertices,
        vertex_count,
        [
            [x1, y0, zz],
            [x0, y1, zz],
            [x0, y0, zz],
            [x1, y0, zz],
            [x1, y1, zz],
            [x0, y1, zz],
        ],
        cell,
    );
}

fn emit_north_quad(
    vertices: &mut Vec<f32>,
    vertex_count: &mut u32,
    x: i32,
    y: i32,
    z: i32,
    width: usize,
    height: usize,
    cell: MaskCell,
) {
    let x0 = x as f32;
    let x1 = (x + width as i32) as f32;
    let y0 = y as f32;
    let y1 = (y + height as i32) as f32;
    let zz = z as f32;
    emit_quad(
        vertices,
        vertex_count,
        [
            [x0, y0, zz],
            [x1, y1, zz],
            [x1, y0, zz],
            [x0, y0, zz],
            [x0, y1, zz],
            [x1, y1, zz],
        ],
        cell,
    );
}
