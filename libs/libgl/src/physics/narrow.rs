use alloc::vec::Vec;

use crate::rasterizer::math;

use super::body::{Collider, RigidBody};
use super::contact::Contact;
use super::math::{clamp_f32, midpoint, Quat, Vec3};

const PLANE_CONTACT_EPS: f32 = 0.01;
const PLANE_MANIFOLD_SLOP: f32 = 0.03;
const MAX_MANIFOLD_CONTACTS: usize = 4;

pub(crate) fn append_contacts(bodies: &[RigidBody], i: usize, j: usize, out: &mut Vec<Contact>) {
    let col_i = bodies[i].collider;
    let col_j = bodies[j].collider;
    let pos_i = bodies[i].position;
    let pos_j = bodies[j].position;

    match (col_i, col_j) {
        (Collider::Sphere { radius: r1 }, Collider::Sphere { radius: r2 }) => {
            let diff = pos_i.sub(pos_j);
            let dist_sq = diff.length_sq();
            let min_dist = r1 + r2;
            if dist_sq > min_dist * min_dist {
                return;
            }
            let normal = if dist_sq > 1e-12 {
                diff.scale(1.0 / math::sqrt(dist_sq))
            } else {
                Vec3::new(0.0, 1.0, 0.0)
            };
            let point_i = pos_i.sub(normal.scale(r1));
            let point_j = pos_j.add(normal.scale(r2));
            out.push(make_contact(bodies, i, j, normal, min_dist - math::sqrt(dist_sq), midpoint(point_i, point_j)));
        }

        (Collider::Sphere { radius }, Collider::Plane { normal, d }) => {
            let dist = pos_i.dot(normal) - d;
            if dist >= radius {
                return;
            }
            let point = pos_i.sub(normal.scale(radius));
            out.push(make_contact(bodies, i, j, normal, radius - dist, point));
        }

        (Collider::Plane { normal, d }, Collider::Sphere { radius }) => {
            let dist = pos_j.dot(normal) - d;
            if dist >= radius {
                return;
            }
            let point = pos_j.sub(normal.scale(radius));
            out.push(make_contact(bodies, j, i, normal, radius - dist, point));
        }

        (Collider::Box { half_x, half_y, half_z }, Collider::Plane { normal, d }) => {
            append_box_plane_contacts(bodies, i, j, normal, d, half_x, half_y, half_z, out);
        }

        (Collider::Plane { normal, d }, Collider::Box { half_x, half_y, half_z }) => {
            append_box_plane_contacts(bodies, j, i, normal, d, half_x, half_y, half_z, out);
        }

        (Collider::Sphere { radius }, Collider::Box { half_x, half_y, half_z }) => {
            if let Some((normal, penetration, point)) =
                sphere_vs_obb(pos_i, radius, pos_j, bodies[j].orientation, half_x, half_y, half_z)
            {
                out.push(make_contact(bodies, i, j, normal, penetration, point));
            }
        }

        (Collider::Box { half_x, half_y, half_z }, Collider::Sphere { radius }) => {
            if let Some((normal, penetration, point)) =
                sphere_vs_obb(pos_j, radius, pos_i, bodies[i].orientation, half_x, half_y, half_z)
            {
                out.push(make_contact(bodies, j, i, normal, penetration, point));
            }
        }

        (Collider::Box { half_x: hx1, half_y: hy1, half_z: hz1 },
         Collider::Box { half_x: hx2, half_y: hy2, half_z: hz2 }) => {
            if let Some((normal, penetration, point)) = obb_vs_obb_face_axis(
                pos_i,
                bodies[i].orientation,
                hx1,
                hy1,
                hz1,
                pos_j,
                bodies[j].orientation,
                hx2,
                hy2,
                hz2,
            ) {
                out.push(make_contact(bodies, i, j, normal, penetration, point));
            }
        }

        (Collider::Plane { .. }, Collider::Plane { .. }) => {}
    }
}

fn make_contact(bodies: &[RigidBody], i: usize, j: usize, normal: Vec3, penetration: f32, point: Vec3) -> Contact {
    Contact {
        i,
        j,
        normal: normal.normalized(),
        penetration,
        point,
        friction: math::sqrt(bodies[i].friction * bodies[j].friction),
        rolling_friction: math::sqrt(bodies[i].rolling_friction * bodies[j].rolling_friction),
        restitution: if bodies[i].restitution < bodies[j].restitution {
            bodies[i].restitution
        } else {
            bodies[j].restitution
        },
    }
}

fn append_box_plane_contacts(
    bodies: &[RigidBody],
    box_idx: usize,
    plane_idx: usize,
    normal: Vec3,
    d: f32,
    hx: f32,
    hy: f32,
    hz: f32,
    out: &mut Vec<Contact>,
) {
    let pos = bodies[box_idx].position;
    let orient = bodies[box_idx].orientation;
    let corners = [
        Vec3::new(-hx, -hy, -hz),
        Vec3::new(-hx, -hy, hz),
        Vec3::new(-hx, hy, -hz),
        Vec3::new(-hx, hy, hz),
        Vec3::new(hx, -hy, -hz),
        Vec3::new(hx, -hy, hz),
        Vec3::new(hx, hy, -hz),
        Vec3::new(hx, hy, hz),
    ];

    let mut world_corners = [Vec3::ZERO; 8];
    let mut distances = [0.0f32; 8];
    let mut min_dist = f32::INFINITY;

    for (idx, local) in corners.iter().copied().enumerate() {
        let world = pos.add(orient.rotate_vec(local));
        let dist = world.dot(normal) - d;
        world_corners[idx] = world;
        distances[idx] = dist;
        if dist < min_dist {
            min_dist = dist;
        }
    }

    if min_dist > PLANE_CONTACT_EPS {
        return;
    }

    let threshold = min_dist + PLANE_MANIFOLD_SLOP;
    let mut added = 0usize;
    for idx in 0..world_corners.len() {
        let dist = distances[idx];
        if dist <= PLANE_CONTACT_EPS && dist <= threshold {
            out.push(make_contact(
                bodies,
                box_idx,
                plane_idx,
                normal,
                -dist,
                world_corners[idx],
            ));
            added += 1;
            if added >= MAX_MANIFOLD_CONTACTS {
                break;
            }
        }
    }

    if added == 0 {
        let deepest_idx = distances
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(core::cmp::Ordering::Equal))
            .map(|(idx, _)| idx)
            .unwrap_or(0);
        let dist = distances[deepest_idx];
        if dist <= PLANE_CONTACT_EPS {
            out.push(make_contact(
                bodies,
                box_idx,
                plane_idx,
                normal,
                -dist,
                world_corners[deepest_idx],
            ));
        }
    }
}

fn sphere_vs_obb(
    sphere_pos: Vec3,
    radius: f32,
    box_pos: Vec3,
    box_orientation: Quat,
    hx: f32,
    hy: f32,
    hz: f32,
) -> Option<(Vec3, f32, Vec3)> {
    let inv = box_orientation.conjugate();
    let local_center = inv.rotate_vec(sphere_pos.sub(box_pos));

    let closest = Vec3::new(
        clamp_f32(local_center.x, -hx, hx),
        clamp_f32(local_center.y, -hy, hy),
        clamp_f32(local_center.z, -hz, hz),
    );
    let delta = local_center.sub(closest);
    let dist_sq = delta.length_sq();

    if dist_sq > 1e-12 {
        if dist_sq >= radius * radius {
            return None;
        }
        let dist = math::sqrt(dist_sq);
        let normal_local = delta.scale(1.0 / dist);
        let normal_world = box_orientation.rotate_vec(normal_local);
        let point_world = box_pos.add(box_orientation.rotate_vec(closest));
        return Some((normal_world, radius - dist, point_world));
    }

    let margin_x = hx - math::abs(local_center.x);
    let margin_y = hy - math::abs(local_center.y);
    let margin_z = hz - math::abs(local_center.z);

    let (normal_local, point_local, penetration) = if margin_x <= margin_y && margin_x <= margin_z {
        let sign = if local_center.x >= 0.0 { 1.0 } else { -1.0 };
        (
            Vec3::new(sign, 0.0, 0.0),
            Vec3::new(sign * hx, local_center.y, local_center.z),
            radius + margin_x,
        )
    } else if margin_y <= margin_z {
        let sign = if local_center.y >= 0.0 { 1.0 } else { -1.0 };
        (
            Vec3::new(0.0, sign, 0.0),
            Vec3::new(local_center.x, sign * hy, local_center.z),
            radius + margin_y,
        )
    } else {
        let sign = if local_center.z >= 0.0 { 1.0 } else { -1.0 };
        (
            Vec3::new(0.0, 0.0, sign),
            Vec3::new(local_center.x, local_center.y, sign * hz),
            radius + margin_z,
        )
    };

    Some((
        box_orientation.rotate_vec(normal_local),
        penetration,
        box_pos.add(box_orientation.rotate_vec(point_local)),
    ))
}

fn obb_vs_obb_face_axis(
    pos_a: Vec3,
    orient_a: Quat,
    hx_a: f32,
    hy_a: f32,
    hz_a: f32,
    pos_b: Vec3,
    orient_b: Quat,
    hx_b: f32,
    hy_b: f32,
    hz_b: f32,
) -> Option<(Vec3, f32, Vec3)> {
    let axes = [
        orient_a.rotate_vec(Vec3::new(1.0, 0.0, 0.0)),
        orient_a.rotate_vec(Vec3::new(0.0, 1.0, 0.0)),
        orient_a.rotate_vec(Vec3::new(0.0, 0.0, 1.0)),
        orient_b.rotate_vec(Vec3::new(1.0, 0.0, 0.0)),
        orient_b.rotate_vec(Vec3::new(0.0, 1.0, 0.0)),
        orient_b.rotate_vec(Vec3::new(0.0, 0.0, 1.0)),
    ];

    let center_delta = pos_b.sub(pos_a);
    let mut best_axis = Vec3::ZERO;
    let mut best_overlap = f32::INFINITY;

    for axis in axes.iter().copied() {
        let axis = axis.normalized();
        if axis.length_sq() <= 1e-12 {
            continue;
        }
        let extent_a = obb_support_extent(axis, orient_a, hx_a, hy_a, hz_a);
        let extent_b = obb_support_extent(axis, orient_b, hx_b, hy_b, hz_b);
        let distance = math::abs(center_delta.dot(axis));
        let overlap = extent_a + extent_b - distance;
        if overlap <= 0.0 {
            return None;
        }
        if overlap < best_overlap {
            best_overlap = overlap;
            best_axis = if center_delta.dot(axis) >= 0.0 { axis.neg() } else { axis };
        }
    }

    if best_overlap == f32::INFINITY {
        return None;
    }

    let point_a = pos_a.add(box_support_point(orient_a, hx_a, hy_a, hz_a, best_axis.neg()));
    let point_b = pos_b.add(box_support_point(orient_b, hx_b, hy_b, hz_b, best_axis));
    Some((best_axis, best_overlap, midpoint(point_a, point_b)))
}

fn obb_support_extent(axis: Vec3, orient: Quat, hx: f32, hy: f32, hz: f32) -> f32 {
    let ax = orient.rotate_vec(Vec3::new(hx, 0.0, 0.0));
    let ay = orient.rotate_vec(Vec3::new(0.0, hy, 0.0));
    let az = orient.rotate_vec(Vec3::new(0.0, 0.0, hz));
    math::abs(axis.dot(ax)) + math::abs(axis.dot(ay)) + math::abs(axis.dot(az))
}

fn box_support_point(orient: Quat, hx: f32, hy: f32, hz: f32, dir_world: Vec3) -> Vec3 {
    let local_dir = orient.conjugate().rotate_vec(dir_world);
    let local = Vec3::new(
        if local_dir.x >= 0.0 { hx } else { -hx },
        if local_dir.y >= 0.0 { hy } else { -hy },
        if local_dir.z >= 0.0 { hz } else { -hz },
    );
    orient.rotate_vec(local)
}
