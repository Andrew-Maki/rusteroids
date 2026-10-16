//! Rendering helpers that imitate Atari's XY ("vector") monitor.
//!
//! Everything in the game is a line segment, so the whole renderer is a
//! polyline drawer plus a cheap fake bloom: each segment is stroked three
//! times, wide-and-dim to narrow-and-bright, which reads convincingly as
//! phosphor glow without needing a shader or render target.

use macroquad::prelude::*;

use crate::{H, W};

/// Width of a "normal" stroke, in world units.
pub const STROKE: f32 = 2.0;

/// (width multiplier, alpha multiplier) for each bloom pass, dimmest first.
const PASSES: [(f32, f32); 3] = [(3.4, 0.07), (2.0, 0.18), (1.0, 1.0)];

fn glow_line(a: Vec2, b: Vec2, thickness: f32, color: Color) {
    for (w, alpha) in PASSES {
        let c = Color::new(color.r, color.g, color.b, color.a * alpha);
        draw_line(a.x, a.y, b.x, b.y, thickness * w, c);
    }
}

/// Draws a connected run of points, translated by `offset`.
pub fn polyline_at(pts: &[Vec2], closed: bool, offset: Vec2, thickness: f32, color: Color) {
    if pts.len() < 2 {
        return;
    }
    for seg in pts.windows(2) {
        glow_line(seg[0] + offset, seg[1] + offset, thickness, color);
    }
    if closed && pts.len() > 2 {
        let (first, last) = (pts[0] + offset, pts[pts.len() - 1] + offset);
        glow_line(last, first, thickness, color);
    }
}

pub fn polyline(pts: &[Vec2], closed: bool, thickness: f32, color: Color) {
    polyline_at(pts, closed, Vec2::ZERO, thickness, color);
}

/// The playfield is a torus, so a shape straddling an edge has to be drawn
/// again on the opposite side. At most four copies are ever needed (one per
/// corner quadrant the shape can bleed into).
fn wrap_offsets(center: Vec2, radius: f32) -> ([Vec2; 4], usize) {
    let dx = if center.x - radius < 0.0 {
        Some(W)
    } else if center.x + radius > W {
        Some(-W)
    } else {
        None
    };
    let dy = if center.y - radius < 0.0 {
        Some(H)
    } else if center.y + radius > H {
        Some(-H)
    } else {
        None
    };

    let mut offs = [Vec2::ZERO; 4];
    let mut n = 1;
    if let Some(dx) = dx {
        offs[n] = vec2(dx, 0.0);
        n += 1;
    }
    if let Some(dy) = dy {
        offs[n] = vec2(0.0, dy);
        n += 1;
    }
    if let (Some(dx), Some(dy)) = (dx, dy) {
        offs[n] = vec2(dx, dy);
        n += 1;
    }
    (offs, n)
}

/// Draws a polyline with wraparound. `center`/`radius` bound the shape so we
/// know which edges it could be crossing.
pub fn wrapped_polyline(
    pts: &[Vec2],
    closed: bool,
    center: Vec2,
    radius: f32,
    thickness: f32,
    color: Color,
) {
    let (offs, n) = wrap_offsets(center, radius);
    for &o in &offs[..n] {
        polyline_at(pts, closed, o, thickness, color);
    }
}

/// A glowing dot, used for explosion debris and the bullet head.
pub fn point(p: Vec2, radius: f32, color: Color) {
    let (offs, n) = wrap_offsets(p, radius * 3.4);
    for &o in &offs[..n] {
        for (w, alpha) in PASSES {
            let c = Color::new(color.r, color.g, color.b, color.a * alpha);
            draw_circle(p.x + o.x, p.y + o.y, radius * w, c);
        }
    }
}
