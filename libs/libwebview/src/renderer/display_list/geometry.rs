fn transformed_bounds(x: i32, y: i32, w: i32, h: i32, rotations: &[DrawRotation]) -> (i32, i32, i32, i32) {
    if w <= 0 || h <= 0 || rotations.is_empty() {
        return (x, y, w, h);
    }

    let mut points = [
        (x as f32, y as f32),
        ((x + w) as f32, y as f32),
        ((x + w) as f32, (y + h) as f32),
        (x as f32, (y + h) as f32),
    ];
    for rot in rotations {
        let rad = rot.angle_deg100 as f32 / 100.0 * core::f32::consts::PI / 180.0;
        let sin = sin_approx(rad);
        let cos = cos_approx(rad);
        for pt in &mut points {
            let dx = pt.0 - rot.origin_x as f32;
            let dy = pt.1 - rot.origin_y as f32;
            pt.0 = rot.origin_x as f32 + dx * cos - dy * sin;
            pt.1 = rot.origin_y as f32 + dx * sin + dy * cos;
        }
    }

    let mut min_x = points[0].0;
    let mut max_x = points[0].0;
    let mut min_y = points[0].1;
    let mut max_y = points[0].1;
    for (px, py) in points.iter().skip(1) {
        min_x = min_x.min(*px);
        max_x = max_x.max(*px);
        min_y = min_y.min(*py);
        max_y = max_y.max(*py);
    }

    let bx = floor_f32(min_x);
    let by = floor_f32(min_y);
    let bw = (ceil_f32(max_x) - bx).max(0);
    let bh = (ceil_f32(max_y) - by).max(0);
    (bx, by, bw, bh)
}

#[inline]
fn floor_f32(v: f32) -> i32 {
    let i = v as i32;
    if v < i as f32 { i - 1 } else { i }
}

#[inline]
fn ceil_f32(v: f32) -> i32 {
    let i = v as i32;
    if v > i as f32 { i + 1 } else { i }
}

