//! Sutherland-Hodgman frustum clipping.
//!
//! Clips triangles against the 6 frustum planes in clip space (before
//! perspective divide). A triangle can produce 0 to 2+ output triangles.

use alloc::vec::Vec;
use super::ClipVertex;

/// Clip a triangle against the view frustum.
///
/// Returns a flat list of vertices forming triangles (fan from first vertex).
pub fn clip_triangle(tri: &[ClipVertex; 3]) -> Vec<ClipVertex> {
    let mut polygon: Vec<ClipVertex> = tri.to_vec();

    // Clip against 6 planes: +w, -w for each axis
    // In clip space, a point is inside if: -w <= x,y,z <= w
    for plane in 0..6 {
        if polygon.len() < 3 { return Vec::new(); }
        polygon = clip_polygon_against_plane(&polygon, plane);
    }

    if polygon.len() < 3 { return Vec::new(); }

    // Triangulate the clipped polygon (fan from vertex 0)
    let mut result = Vec::new();
    for i in 1..polygon.len() - 1 {
        result.push(polygon[0].clone());
        result.push(polygon[i].clone());
        result.push(polygon[i + 1].clone());
    }
    result
}

/// Clip a convex polygon against one frustum plane.
fn clip_polygon_against_plane(verts: &[ClipVertex], plane: usize) -> Vec<ClipVertex> {
    let mut out = Vec::new();
    let n = verts.len();

    for i in 0..n {
        let curr = &verts[i];
        let next = &verts[(i + 1) % n];
        let d_curr = signed_distance(curr, plane);
        let d_next = signed_distance(next, plane);

        if d_curr >= 0.0 {
            out.push(curr.clone());
        }
        if (d_curr >= 0.0) != (d_next >= 0.0) {
            // Edge crosses plane — compute intersection
            let denom = d_curr - d_next;
            if denom.abs() > 1e-10 {
                let t = d_curr / denom;
                out.push(interpolate_vertex(curr, next, t));
            }
        }
    }

    out
}

/// Signed distance from a vertex to a frustum plane.
///
/// Planes in clip space:
/// 0: x + w >= 0  (left)
/// 1: -x + w >= 0 (right)
/// 2: y + w >= 0  (bottom)
/// 3: -y + w >= 0 (top)
/// 4: z + w >= 0  (near)
/// 5: -z + w >= 0 (far)
fn signed_distance(v: &ClipVertex, plane: usize) -> f32 {
    let p = &v.position;
    match plane {
        0 => p[0] + p[3],   // left:   x + w
        1 => -p[0] + p[3],  // right: -x + w
        2 => p[1] + p[3],   // bottom: y + w
        3 => -p[1] + p[3],  // top:   -y + w
        4 => p[2] + p[3],   // near:   z + w
        5 => -p[2] + p[3],  // far:   -z + w
        _ => 0.0,
    }
}

/// Linearly interpolate between two vertices.
fn interpolate_vertex(a: &ClipVertex, b: &ClipVertex, t: f32) -> ClipVertex {
    let position = [
        a.position[0] + (b.position[0] - a.position[0]) * t,
        a.position[1] + (b.position[1] - a.position[1]) * t,
        a.position[2] + (b.position[2] - a.position[2]) * t,
        a.position[3] + (b.position[3] - a.position[3]) * t,
    ];

    let num_vary = a.varyings.len().min(b.varyings.len());
    let mut varyings = Vec::with_capacity(num_vary);
    for i in 0..num_vary {
        varyings.push([
            a.varyings[i][0] + (b.varyings[i][0] - a.varyings[i][0]) * t,
            a.varyings[i][1] + (b.varyings[i][1] - a.varyings[i][1]) * t,
            a.varyings[i][2] + (b.varyings[i][2] - a.varyings[i][2]) * t,
            a.varyings[i][3] + (b.varyings[i][3] - a.varyings[i][3]) * t,
        ]);
    }

    ClipVertex { position, varyings }
}
