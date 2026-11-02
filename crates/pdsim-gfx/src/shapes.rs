//! Shape rasterizers. All coordinates are user coordinates.

use crate::painter::Painter;
use crate::types::{Color, FillRule, LineCap, Rect};

/// One-pixel Bresenham line, both ends inclusive.
pub fn line1(p: &mut Painter, x0: i32, y0: i32, x1: i32, y1: i32, c: Color) {
    let (mut x, mut y) = (x0, y0);
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    loop {
        p.plot(x, y, c);
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

/// A line of any width with the given cap style.
pub fn line(p: &mut Painter, x0: i32, y0: i32, x1: i32, y1: i32, width: i32, cap: LineCap, c: Color) {
    if width <= 1 {
        return line1(p, x0, y0, x1, y1, c);
    }
    let (fx0, fy0, fx1, fy1) = (x0 as f32 + 0.5, y0 as f32 + 0.5, x1 as f32 + 0.5, y1 as f32 + 0.5);
    let (dx, dy) = (fx1 - fx0, fy1 - fy0);
    let len = (dx * dx + dy * dy).sqrt();
    let hw = width as f32 / 2.0;
    if len < 1e-6 {
        let r = Rect::new(x0 - width / 2, y0 - width / 2, width, width);
        return p.fill_rect(r, c);
    }
    let (ux, uy) = (dx / len, dy / len);
    let (nx, ny) = (-uy * hw, ux * hw);
    let ext = if cap == LineCap::Square { hw } else { 0.0 };
    let (ax, ay) = (fx0 - ux * ext, fy0 - uy * ext);
    let (bx, by) = (fx1 + ux * ext, fy1 + uy * ext);
    let pts = [
        (ax + nx, ay + ny),
        (bx + nx, by + ny),
        (bx - nx, by - ny),
        (ax - nx, ay - ny),
    ];
    fill_polygon_f(p, &pts, FillRule::NonZero, c);
    if cap == LineCap::Round {
        disc(p, fx0, fy0, hw, c);
        disc(p, fx1, fy1, hw, c);
    }
}

fn disc(p: &mut Painter, cx: f32, cy: f32, r: f32, c: Color) {
    let y0 = (cy - r).floor() as i32;
    let y1 = (cy + r).ceil() as i32;
    for y in y0..=y1 {
        let py = y as f32 + 0.5 - cy;
        let d = r * r - py * py;
        if d < 0.0 {
            continue;
        }
        let w = d.sqrt();
        let a = (cx - w - 0.5).ceil() as i32;
        let b = (cx + w - 0.5).floor() as i32;
        if a <= b {
            p.hspan(a, b, y, c);
        }
    }
}

/// Fills a polygon given in pixel-edge coordinates (a pixel `(x, y)` is
/// covered when its center `(x+0.5, y+0.5)` is inside).
pub fn fill_polygon_f(p: &mut Painter, pts: &[(f32, f32)], rule: FillRule, c: Color) {
    if pts.len() < 3 {
        return;
    }
    let ymin = pts.iter().map(|q| q.1).fold(f32::INFINITY, f32::min).floor() as i32;
    let ymax = pts.iter().map(|q| q.1).fold(f32::NEG_INFINITY, f32::max).ceil() as i32;
    // Clamp to the clip in user space to avoid huge loops.
    let clip = p.clip();
    let (_, oy) = p.offset();
    let ymin = ymin.max(clip.y - oy);
    let ymax = ymax.min(clip.y + clip.h - oy);
    let mut xs: Vec<(f32, i32)> = Vec::with_capacity(pts.len());
    for y in ymin..ymax {
        let sy = y as f32 + 0.5;
        xs.clear();
        for i in 0..pts.len() {
            let (ax, ay) = pts[i];
            let (bx, by) = pts[(i + 1) % pts.len()];
            if (ay <= sy && by > sy) || (by <= sy && ay > sy) {
                let t = (sy - ay) / (by - ay);
                xs.push((ax + t * (bx - ax), if by > ay { 1 } else { -1 }));
            }
        }
        xs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        let mut wind = 0;
        for i in 0..xs.len().saturating_sub(1) {
            wind += xs[i].1;
            let inside = match rule {
                FillRule::EvenOdd => i % 2 == 0,
                FillRule::NonZero => wind != 0,
            };
            if inside {
                let a = (xs[i].0 - 0.5).ceil() as i32;
                let b = (xs[i + 1].0 - 0.5).ceil() as i32 - 1;
                if a <= b {
                    p.hspan(a, b, y, c);
                }
            }
        }
    }
}

/// Fills a polygon with integer vertices (pixel coordinates). The outline
/// pixels are included, like the device.
pub fn fill_polygon(p: &mut Painter, pts: &[(i32, i32)], rule: FillRule, c: Color) {
    let f: Vec<(f32, f32)> = pts
        .iter()
        .map(|&(x, y)| (x as f32 + 0.5, y as f32 + 0.5))
        .collect();
    fill_polygon_f(p, &f, rule, c);
    for i in 0..pts.len() {
        let a = pts[i];
        let b = pts[(i + 1) % pts.len()];
        line1(p, a.0, a.1, b.0, b.1, c);
    }
}

/// Playdate angles are degrees clockwise from 12 o'clock.
fn angle_of(dx: f32, dy: f32) -> f32 {
    let a = dx.atan2(-dy).to_degrees();
    if a < 0.0 {
        a + 360.0
    } else {
        a
    }
}

fn in_arc(a: f32, start: f32, end: f32) -> bool {
    if (start - end).abs() < 1e-6 || (end - start).abs() >= 360.0 {
        return true;
    }
    let s = start.rem_euclid(360.0);
    let e = end.rem_euclid(360.0);
    if s <= e {
        a >= s && a <= e
    } else {
        a >= s || a <= e
    }
}

fn ellipse_inside(px: f32, py: f32, cx: f32, cy: f32, rx: f32, ry: f32) -> bool {
    if rx <= 0.0 || ry <= 0.0 {
        return false;
    }
    let nx = (px - cx) / rx;
    let ny = (py - cy) / ry;
    nx * nx + ny * ny <= 1.0
}

/// Fills the ellipse inscribed in `r`, optionally only the arc between the
/// two angles.
pub fn fill_ellipse(p: &mut Painter, r: Rect, start: f32, end: f32, c: Color) {
    if r.is_empty() {
        return;
    }
    let (cx, cy) = (r.x as f32 + r.w as f32 / 2.0, r.y as f32 + r.h as f32 / 2.0);
    let (rx, ry) = (r.w as f32 / 2.0, r.h as f32 / 2.0);
    let full = in_arc(0.0, start, end) && ((start - end).abs() < 1e-6 || (end - start).abs() >= 360.0);
    for y in r.y..r.y + r.h {
        let py = y as f32 + 0.5;
        if full {
            let ny = (py - cy) / ry;
            let d = 1.0 - ny * ny;
            if d < 0.0 {
                continue;
            }
            let w = rx * d.sqrt();
            let a = (cx - w - 0.5).ceil() as i32;
            let b = (cx + w - 0.5).floor() as i32;
            if a <= b {
                p.hspan(a.max(r.x), b.min(r.x + r.w - 1), y, c);
            }
        } else {
            for x in r.x..r.x + r.w {
                let px = x as f32 + 0.5;
                if ellipse_inside(px, py, cx, cy, rx, ry) && in_arc(angle_of(px - cx, py - cy), start, end) {
                    p.plot(x, y, c);
                }
            }
        }
    }
}

/// Strokes the ellipse inscribed in `r` with a line `lw` pixels wide drawn
/// inside the rect.
pub fn draw_ellipse(p: &mut Painter, r: Rect, lw: i32, start: f32, end: f32, c: Color) {
    if r.is_empty() {
        return;
    }
    let lw = lw.max(1) as f32;
    let (cx, cy) = (r.x as f32 + r.w as f32 / 2.0, r.y as f32 + r.h as f32 / 2.0);
    let (rx, ry) = (r.w as f32 / 2.0, r.h as f32 / 2.0);
    for y in r.y..r.y + r.h {
        let py = y as f32 + 0.5;
        for x in r.x..r.x + r.w {
            let px = x as f32 + 0.5;
            if !ellipse_inside(px, py, cx, cy, rx, ry) {
                continue;
            }
            // On the stroke: inside the outer ellipse and, for 1px lines, next
            // to its edge; for wider lines, outside the inner ellipse.
            let on = if lw <= 1.0 {
                [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)]
                    .iter()
                    .any(|(dx, dy)| !ellipse_inside(px + dx, py + dy, cx, cy, rx, ry))
            } else {
                !ellipse_inside(px, py, cx, cy, rx - lw, ry - lw)
            };
            if on && in_arc(angle_of(px - cx, py - cy), start, end) {
                p.plot(x, y, c);
            }
        }
    }
}

fn in_round_rect(x: i32, y: i32, r: Rect, rad: i32) -> bool {
    if !r.contains(x, y) {
        return false;
    }
    let rad = rad.min(r.w / 2).min(r.h / 2).max(0);
    if rad == 0 {
        return true;
    }
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let left = r.x as f32 + rad as f32;
    let right = (r.x + r.w) as f32 - rad as f32;
    let top = r.y as f32 + rad as f32;
    let bottom = (r.y + r.h) as f32 - rad as f32;
    let cx = px.clamp(left, right);
    let cy = py.clamp(top, bottom);
    let (dx, dy) = (px - cx, py - cy);
    dx * dx + dy * dy <= (rad * rad) as f32
}

/// Fills a rounded rectangle.
pub fn fill_round_rect(p: &mut Painter, r: Rect, radius: i32, c: Color) {
    for y in r.y..r.y + r.h {
        for x in r.x..r.x + r.w {
            if in_round_rect(x, y, r, radius) {
                p.plot(x, y, c);
            }
        }
    }
}

/// Strokes a rounded rectangle inside `r`.
pub fn draw_round_rect(p: &mut Painter, r: Rect, radius: i32, lw: i32, c: Color) {
    let lw = lw.max(1);
    let inner = Rect::new(r.x + lw, r.y + lw, r.w - 2 * lw, r.h - 2 * lw);
    for y in r.y..r.y + r.h {
        for x in r.x..r.x + r.w {
            if in_round_rect(x, y, r, radius) && !in_round_rect(x, y, inner, (radius - lw).max(0)) {
                p.plot(x, y, c);
            }
        }
    }
}
