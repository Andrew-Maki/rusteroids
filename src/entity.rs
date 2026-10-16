//! The things that move: ship, bullets, rocks, saucers and debris.
//!
//! Every entity lives on a torus, so position updates go through [`wrap`] and
//! distance tests through [`torus_delta`] — a rock one pixel off the left edge
//! is adjacent to a ship one pixel off the right edge.
//!
//! Shapes are stored in local space and transformed on the way to the
//! renderer. Rocks deliberately do *not* rotate: the arcade original only
//! translated them, and spinning them makes the playfield read as much busier
//! than it should.

use std::f32::consts::{PI, TAU};

use macroquad::prelude::*;

use crate::render::{self, STROKE};
use crate::sfx::BangSize;
use crate::{H, PHOSPHOR, W};

// ---------------------------------------------------------------------------
// Toroidal playfield maths
// ---------------------------------------------------------------------------

/// Brings a position back inside the playfield.
pub fn wrap(p: Vec2) -> Vec2 {
    vec2(p.x.rem_euclid(W), p.y.rem_euclid(H))
}

/// Shortest vector from `a` to `b`, going around an edge when that is closer.
pub fn torus_delta(a: Vec2, b: Vec2) -> Vec2 {
    let mut d = b - a;
    if d.x > W * 0.5 {
        d.x -= W;
    } else if d.x < -W * 0.5 {
        d.x += W;
    }
    if d.y > H * 0.5 {
        d.y -= H;
    } else if d.y < -H * 0.5 {
        d.y += H;
    }
    d
}

/// Circle-vs-circle overlap test across the wrap.
pub fn hits(a: Vec2, ra: f32, b: Vec2, rb: f32) -> bool {
    let r = ra + rb;
    torus_delta(a, b).length_squared() <= r * r
}

/// A uniformly random point on the playfield.
pub fn random_point() -> Vec2 {
    vec2(rand::gen_range(0.0, W), rand::gen_range(0.0, H))
}

/// A random point just inside an edge, used to launch rocks and saucers from
/// somewhere that is never on top of the player.
pub fn random_edge_point() -> Vec2 {
    let inset = 24.0;
    match rand::gen_range(0, 4) {
        0 => vec2(rand::gen_range(0.0, W), inset),
        1 => vec2(rand::gen_range(0.0, W), H - inset),
        2 => vec2(inset, rand::gen_range(0.0, H)),
        _ => vec2(W - inset, rand::gen_range(0.0, H)),
    }
}

fn rotate(p: Vec2, sin_cos: (f32, f32)) -> Vec2 {
    let (sin, cos) = sin_cos;
    vec2(p.x * cos - p.y * sin, p.x * sin + p.y * cos)
}

/// Fills `buf` with `pts` rotated by `angle` and moved to `pos`.
fn transform(buf: &mut Vec<Vec2>, pts: &[Vec2], pos: Vec2, angle: f32) {
    let r = angle.sin_cos();
    buf.clear();
    buf.extend(pts.iter().map(|&p| pos + rotate(p, r)));
}

// ---------------------------------------------------------------------------
// Ship
// ---------------------------------------------------------------------------

/// Collision radius: a little tighter than the hull, which forgives grazes.
pub const SHIP_R: f32 = 8.5;

/// Arrowhead with a notched tail, drawn as one closed polyline.
const HULL: [Vec2; 5] = [
    Vec2::new(15.0, 0.0),
    Vec2::new(-10.0, 9.0),
    Vec2::new(-6.0, 5.0),
    Vec2::new(-6.0, -5.0),
    Vec2::new(-10.0, -9.0),
];

/// Exhaust plume, only drawn while thrusting.
const FLAME: [Vec2; 3] = [
    Vec2::new(-6.0, 4.0),
    Vec2::new(-17.0, 0.0),
    Vec2::new(-6.0, -4.0),
];

const TURN_RATE: f32 = 3.9; // rad/s
const THRUST: f32 = 430.0; // px/s²
const DRAG: f32 = 0.55; // e-folds per second
const MAX_SPEED: f32 = 540.0;

pub struct Ship {
    pub pos: Vec2,
    pub vel: Vec2,
    pub angle: f32,
    pub thrusting: bool,
    /// Seconds of spawn protection left; the hull blinks while it lasts.
    pub invuln: f32,
    /// Drives the flame flicker, which is time-based rather than frame-based
    /// so it looks the same at any refresh rate.
    flicker: f32,
}

impl Ship {
    pub fn new(pos: Vec2, invuln: f32) -> Self {
        Self {
            pos,
            vel: Vec2::ZERO,
            angle: -PI / 2.0, // nose up
            thrusting: false,
            invuln,
            flicker: 0.0,
        }
    }

    /// `turn` is -1, 0 or 1.
    pub fn update(&mut self, dt: f32, turn: f32, thrust: bool) {
        self.angle += turn * TURN_RATE * dt;
        self.thrusting = thrust;
        if thrust {
            self.vel += self.heading() * THRUST * dt;
        }
        // Exponential decay rather than a per-frame multiplier, so the glide
        // feels identical regardless of framerate.
        self.vel *= (-DRAG * dt).exp();
        let speed = self.vel.length();
        if speed > MAX_SPEED {
            self.vel *= MAX_SPEED / speed;
        }
        self.pos = wrap(self.pos + self.vel * dt);
        self.invuln = (self.invuln - dt).max(0.0);
        self.flicker += dt;
    }

    pub fn heading(&self) -> Vec2 {
        Vec2::from_angle(self.angle)
    }

    /// Muzzle position, so shots leave the nose rather than the middle.
    pub fn nose(&self) -> Vec2 {
        wrap(self.pos + self.heading() * 15.0)
    }

    /// Breaks the hull into tumbling line segments for the death animation.
    pub fn fragments(&self, life: f32) -> Vec<Piece> {
        let r = self.angle.sin_cos();
        let mut out = Vec::with_capacity(HULL.len());
        for i in 0..HULL.len() {
            let a = rotate(HULL[i], r);
            let b = rotate(HULL[(i + 1) % HULL.len()], r);
            let mid = (a + b) * 0.5;
            // Push each piece outward from the hull centre, plus a little jitter.
            let dir = mid.normalize_or_zero();
            out.push(Piece {
                a: a - mid,
                b: b - mid,
                pos: wrap(self.pos + mid),
                vel: self.vel * 0.3
                    + dir * rand::gen_range(18.0, 46.0)
                    + vec2(rand::gen_range(-12.0, 12.0), rand::gen_range(-12.0, 12.0)),
                angle: 0.0,
                spin: rand::gen_range(-2.2, 2.2),
                life,
                max_life: life,
            });
        }
        out
    }

    pub fn draw(&self, buf: &mut Vec<Vec2>) {
        // Blink at 6 Hz while protected, so the player can see it is not solid.
        if self.invuln > 0.0 && (self.invuln * 12.0) as i32 % 2 == 0 {
            return;
        }
        transform(buf, &HULL, self.pos, self.angle);
        render::wrapped_polyline(buf, true, self.pos, 18.0, STROKE, PHOSPHOR);

        if self.thrusting && (self.flicker * 38.0) as i32 % 3 != 0 {
            transform(buf, &FLAME, self.pos, self.angle);
            render::wrapped_polyline(buf, false, self.pos, 20.0, STROKE, PHOSPHOR);
        }
    }

    /// Draws the ship outline as a static icon, for the spare-ships display.
    pub fn draw_icon(buf: &mut Vec<Vec2>, pos: Vec2, scale: f32) {
        buf.clear();
        buf.extend(HULL.iter().map(|&p| pos + rotate(p * scale, (-1.0, 0.0))));
        render::polyline(buf, true, STROKE * 0.8, PHOSPHOR);
    }
}

// ---------------------------------------------------------------------------
// Bullets
// ---------------------------------------------------------------------------

pub const BULLET_R: f32 = 1.5;
pub const BULLET_SPEED: f32 = 620.0;
/// Shots live long enough to cross a bit more than half the playfield, which
/// is what stops the player from simply holding fire and sweeping the screen.
pub const BULLET_LIFE: f32 = 0.95;

#[derive(Copy, Clone)]
pub struct Bullet {
    pub pos: Vec2,
    pub vel: Vec2,
    pub life: f32,
    /// Saucer shots also break rocks, but they score nothing and they kill the
    /// player, so ownership has to travel with the bullet.
    pub from_player: bool,
}

impl Bullet {
    pub fn new(pos: Vec2, vel: Vec2, life: f32, from_player: bool) -> Self {
        Self {
            pos,
            vel,
            life,
            from_player,
        }
    }

    /// Returns `false` once the shot has expired.
    pub fn update(&mut self, dt: f32) -> bool {
        self.life -= dt;
        self.pos = wrap(self.pos + self.vel * dt);
        self.life > 0.0
    }

    pub fn draw(&self) {
        render::point(self.pos, if self.from_player { 1.7 } else { 1.5 }, PHOSPHOR);
    }
}

// ---------------------------------------------------------------------------
// Rocks
// ---------------------------------------------------------------------------

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum RockSize {
    Large,
    Medium,
    Small,
}

impl RockSize {
    pub fn radius(self) -> f32 {
        match self {
            RockSize::Large => 42.0,
            RockSize::Medium => 21.0,
            RockSize::Small => 10.5,
        }
    }

    pub fn score(self) -> u32 {
        match self {
            RockSize::Large => 20,
            RockSize::Medium => 50,
            RockSize::Small => 100,
        }
    }

    /// What this breaks into, or `None` if it just vaporises.
    pub fn smaller(self) -> Option<RockSize> {
        match self {
            RockSize::Large => Some(RockSize::Medium),
            RockSize::Medium => Some(RockSize::Small),
            RockSize::Small => None,
        }
    }

    pub fn bang(self) -> BangSize {
        match self {
            RockSize::Large => BangSize::Large,
            RockSize::Medium => BangSize::Medium,
            RockSize::Small => BangSize::Small,
        }
    }

    /// Smaller fragments fly faster, which is what makes late waves tense.
    fn speed_range(self) -> (f32, f32) {
        match self {
            RockSize::Large => (26.0, 62.0),
            RockSize::Medium => (46.0, 104.0),
            RockSize::Small => (74.0, 158.0),
        }
    }

    /// Number of debris dots a kill throws out.
    fn debris(self) -> usize {
        match self {
            RockSize::Large => 14,
            RockSize::Medium => 10,
            RockSize::Small => 7,
        }
    }

    /// Mass for the bounce, proportional to area. The radii are 4:2:1, so the
    /// masses come out 16:4:1 and a small rock barely budges a large one.
    fn mass(self) -> f32 {
        let r = self.radius() / RockSize::Small.radius();
        r * r
    }
}

pub struct Rock {
    pub pos: Vec2,
    pub vel: Vec2,
    pub size: RockSize,
    /// Seconds during which this rock ignores other rocks.
    ///
    /// Fragments are born on top of their siblings (both start at the parent's
    /// position), so without a grace period a single break would re-trigger
    /// every frame and cascade the whole wave down to nothing.
    pub grace: f32,
    /// Outline in local space, generated once at spawn.
    shape: Vec<Vec2>,
}

/// A lumpy closed polygon: evenly spaced angles with jittered radii, plus the
/// occasional deep crater so no two rocks look alike.
fn rock_outline(radius: f32) -> Vec<Vec2> {
    let n = rand::gen_range(9, 13);
    (0..n)
        .map(|i| {
            let a = TAU * i as f32 / n as f32;
            let mut r = radius * rand::gen_range(0.76, 1.1);
            if rand::gen_range(0, 6) == 0 {
                r *= 0.6;
            }
            Vec2::from_angle(a) * r
        })
        .collect()
}

/// How long a freshly split fragment ignores other rocks.
pub const FRAGMENT_GRACE: f32 = 0.45;

impl Rock {
    pub fn new(pos: Vec2, size: RockSize) -> Self {
        let (lo, hi) = size.speed_range();
        let dir = Vec2::from_angle(rand::gen_range(0.0, TAU));
        Self {
            pos,
            vel: dir * rand::gen_range(lo, hi),
            size,
            grace: 0.0,
            shape: rock_outline(size.radius()),
        }
    }

    /// A fragment of a destroyed rock: inherits some of the parent's momentum
    /// so the pieces visibly spray apart along the original path.
    pub fn fragment(pos: Vec2, size: RockSize, parent_vel: Vec2) -> Self {
        let (lo, hi) = size.speed_range();
        let spread = rand::gen_range(-1.1, 1.1);
        let base = if parent_vel.length_squared() > 1.0 {
            parent_vel.normalize()
        } else {
            Vec2::from_angle(rand::gen_range(0.0, TAU))
        };
        let dir = base.rotate(Vec2::from_angle(spread));
        Self {
            pos,
            vel: parent_vel * 0.35 + dir * rand::gen_range(lo, hi),
            size,
            // Long enough for siblings to clear each other before they can
            // collide, at the slowest fragment speed.
            grace: FRAGMENT_GRACE,
            shape: rock_outline(size.radius()),
        }
    }

    pub fn radius(&self) -> f32 {
        self.size.radius()
    }

    pub fn mass(&self) -> f32 {
        self.size.mass()
    }

    /// True once this rock has been clear of its siblings long enough to take
    /// part in rock-vs-rock collisions.
    pub fn collidable(&self) -> bool {
        self.grace <= 0.0
    }

    pub fn update(&mut self, dt: f32) {
        self.pos = wrap(self.pos + self.vel * dt);
        self.grace = (self.grace - dt).max(0.0);
    }

    /// Debris dots for the explosion.
    pub fn burst(&self) -> Vec<Dot> {
        (0..self.size.debris())
            .map(|_| {
                let dir = Vec2::from_angle(rand::gen_range(0.0, TAU));
                let life = rand::gen_range(0.32, 0.75);
                Dot {
                    pos: self.pos,
                    vel: self.vel * 0.4 + dir * rand::gen_range(40.0, 150.0),
                    life,
                    max_life: life,
                }
            })
            .collect()
    }

    pub fn draw(&self, buf: &mut Vec<Vec2>) {
        buf.clear();
        buf.extend(self.shape.iter().map(|&p| self.pos + p));
        render::wrapped_polyline(buf, true, self.pos, self.radius() * 1.15, STROKE, PHOSPHOR);
    }
}

/// Two rocks touching.
pub struct Contact {
    /// Where on the shared surface they met, for sparks or an explosion.
    pub point: Vec2,
    /// Closing speed along the contact normal in px/s, i.e. how hard the hit
    /// was. The caller uses this to decide between a bounce and a break.
    pub speed: f32,
}

/// Energy kept across a bounce. Slightly lossy, so a crowded playfield settles
/// down instead of rattling forever.
const RESTITUTION: f32 = 0.9;

/// Tests two rocks; if they overlap and are closing, resolves the collision
/// elastically and reports the contact.
///
/// Returns `None` when they are apart, when either is still in its spawn grace
/// period, or when they overlap but are already moving apart — that last case
/// matters, because re-resolving a separating pair would trap them together.
pub fn collide_rocks(a: &mut Rock, b: &mut Rock) -> Option<Contact> {
    if !a.collidable() || !b.collidable() {
        return None;
    }

    let d = torus_delta(a.pos, b.pos); // a -> b, around an edge if nearer
    let dist = d.length();
    let min_dist = a.radius() + b.radius();
    if dist >= min_dist {
        return None;
    }

    // Degenerate case: exactly concentric. Any axis will do.
    let normal = if dist > 1e-4 { d / dist } else { Vec2::X };
    let closing = -(b.vel - a.vel).dot(normal);
    if closing <= 0.0 {
        return None;
    }

    let (inv_a, inv_b) = (1.0 / a.mass(), 1.0 / b.mass());
    let impulse = (1.0 + RESTITUTION) * closing / (inv_a + inv_b);
    a.vel -= normal * (impulse * inv_a);
    b.vel += normal * (impulse * inv_b);

    // Push them apart so the next frame does not see the same overlap. The
    // lighter rock gives way, and a little extra clears rounding error.
    let overlap = min_dist - dist;
    let share = overlap / (inv_a + inv_b);
    a.pos = wrap(a.pos - normal * (share * inv_a + 0.25));
    b.pos = wrap(b.pos + normal * (share * inv_b + 0.25));

    Some(Contact {
        point: wrap(a.pos + normal * a.radius()),
        speed: closing,
    })
}

// ---------------------------------------------------------------------------
// Saucers
// ---------------------------------------------------------------------------

/// Hull: the familiar flattened hexagon.
const SAUCER_HULL: [Vec2; 6] = [
    Vec2::new(-20.0, 0.0),
    Vec2::new(-9.0, -5.0),
    Vec2::new(9.0, -5.0),
    Vec2::new(20.0, 0.0),
    Vec2::new(9.0, 6.0),
    Vec2::new(-9.0, 6.0),
];
const SAUCER_DOME: [Vec2; 4] = [
    Vec2::new(-9.0, -5.0),
    Vec2::new(-4.0, -10.0),
    Vec2::new(4.0, -10.0),
    Vec2::new(9.0, -5.0),
];
const SAUCER_WAIST: [Vec2; 2] = [Vec2::new(-20.0, 0.0), Vec2::new(20.0, 0.0)];

const SAUCER_SPEED: f32 = 128.0;
/// How far the saucer slides vertically while it crosses.
const SAUCER_DRIFT: f32 = 118.0;

pub struct Saucer {
    pub pos: Vec2,
    pub vel: Vec2,
    pub small: bool,
    /// +1 if it entered from the left, -1 from the right.
    dir: f32,
    fire_timer: f32,
    turn_timer: f32,
}

impl Saucer {
    pub fn spawn(small: bool) -> Self {
        let from_left = rand::gen_range(0, 2) == 0;
        let dir = if from_left { 1.0 } else { -1.0 };
        let x = if from_left { -24.0 } else { W + 24.0 };
        let speed = SAUCER_SPEED * if small { 1.22 } else { 1.0 };
        let mut s = Self {
            pos: vec2(x, rand::gen_range(H * 0.12, H * 0.88)),
            vel: vec2(dir * speed, 0.0),
            small,
            dir,
            fire_timer: rand::gen_range(0.4, 1.1),
            turn_timer: 0.0,
        };
        s.pick_drift();
        s
    }

    pub fn radius(&self) -> f32 {
        if self.small { 11.0 } else { 19.0 }
    }

    pub fn score(&self) -> u32 {
        if self.small { 1000 } else { 200 }
    }

    fn scale(&self) -> f32 {
        if self.small { 0.55 } else { 1.0 }
    }

    fn pick_drift(&mut self) {
        // -1, 0 or +1: the saucer tracks straight for a while, then steps up or
        // down, giving the characteristic staircase path.
        let lane = rand::gen_range(0, 3) as f32 - 1.0;
        self.vel.y = lane * SAUCER_DRIFT;
        self.turn_timer = rand::gen_range(0.55, 1.5);
    }

    /// Returns `false` once it has flown off the far side.
    pub fn update(&mut self, dt: f32) -> bool {
        self.turn_timer -= dt;
        if self.turn_timer <= 0.0 {
            self.pick_drift();
        }
        self.pos += self.vel * dt;
        // Only the vertical axis wraps: a saucer makes exactly one pass.
        self.pos.y = self.pos.y.rem_euclid(H);
        if self.dir > 0.0 {
            self.pos.x < W + 36.0
        } else {
            self.pos.x > -36.0
        }
    }

    /// Ticks the gun; returns a shot when it is time to fire.
    ///
    /// The large saucer sprays at random. The small one aims, with an error
    /// that shrinks as the score climbs — the arcade machine's way of making
    /// a good run progressively more dangerous.
    pub fn try_fire(&mut self, dt: f32, target: Option<Vec2>, score: u32) -> Option<Bullet> {
        self.fire_timer -= dt;
        if self.fire_timer > 0.0 {
            return None;
        }
        self.fire_timer = if self.small {
            rand::gen_range(0.75, 1.25)
        } else {
            rand::gen_range(1.0, 1.8)
        };

        let angle = match (self.small, target) {
            (true, Some(t)) => {
                let aim = torus_delta(self.pos, t);
                let spread = (0.55 - score as f32 / 70_000.0).max(0.015);
                aim.y.atan2(aim.x) + rand::gen_range(-spread, spread)
            }
            _ => rand::gen_range(0.0, TAU),
        };
        let vel = Vec2::from_angle(angle) * BULLET_SPEED * 0.72;
        Some(Bullet::new(wrap(self.pos), vel, BULLET_LIFE * 1.3, false))
    }

    pub fn burst(&self) -> Vec<Dot> {
        (0..12)
            .map(|_| {
                let dir = Vec2::from_angle(rand::gen_range(0.0, TAU));
                let life = rand::gen_range(0.3, 0.7);
                Dot {
                    pos: self.pos,
                    vel: dir * rand::gen_range(50.0, 170.0),
                    life,
                    max_life: life,
                }
            })
            .collect()
    }

    pub fn draw(&self, buf: &mut Vec<Vec2>) {
        let s = self.scale();
        for (pts, closed) in [
            (&SAUCER_HULL[..], true),
            (&SAUCER_DOME[..], false),
            (&SAUCER_WAIST[..], false),
        ] {
            buf.clear();
            buf.extend(pts.iter().map(|&p| self.pos + p * s));
            render::wrapped_polyline(buf, closed, self.pos, 22.0 * s, STROKE, PHOSPHOR);
        }
    }
}

// ---------------------------------------------------------------------------
// Debris
// ---------------------------------------------------------------------------

/// A single glowing fleck thrown out by an explosion.
pub struct Dot {
    pub pos: Vec2,
    pub vel: Vec2,
    pub life: f32,
    pub max_life: f32,
}

impl Dot {
    pub fn update(&mut self, dt: f32) -> bool {
        self.life -= dt;
        self.vel *= (-1.1 * dt).exp();
        self.pos = wrap(self.pos + self.vel * dt);
        self.life > 0.0
    }

    pub fn draw(&self) {
        let a = (self.life / self.max_life).clamp(0.0, 1.0);
        render::point(
            self.pos,
            1.5,
            Color::new(PHOSPHOR.r, PHOSPHOR.g, PHOSPHOR.b, a),
        );
    }
}

/// A tumbling line segment, used for the ship coming apart.
pub struct Piece {
    /// Endpoints relative to the piece's own centre.
    pub a: Vec2,
    pub b: Vec2,
    pub pos: Vec2,
    pub vel: Vec2,
    pub angle: f32,
    pub spin: f32,
    pub life: f32,
    pub max_life: f32,
}

impl Piece {
    pub fn update(&mut self, dt: f32) -> bool {
        self.life -= dt;
        self.angle += self.spin * dt;
        self.pos = wrap(self.pos + self.vel * dt);
        self.life > 0.0
    }

    pub fn draw(&self, buf: &mut Vec<Vec2>) {
        let a = (self.life / self.max_life).clamp(0.0, 1.0);
        let c = Color::new(PHOSPHOR.r, PHOSPHOR.g, PHOSPHOR.b, a);
        transform(buf, &[self.a, self.b], self.pos, self.angle);
        render::wrapped_polyline(buf, false, self.pos, 18.0, STROKE, c);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Everything in here is pure maths — no window, no audio device — so it
    /// can run under `cargo test`.
    #[test]
    fn wrap_keeps_positions_inside_the_playfield() {
        for p in [
            vec2(-1.0, -1.0),
            vec2(W + 5.0, H + 5.0),
            vec2(-3.0 * W, 2.5 * H),
        ] {
            let w = wrap(p);
            assert!((0.0..W).contains(&w.x), "x out of range: {}", w.x);
            assert!((0.0..H).contains(&w.y), "y out of range: {}", w.y);
        }
    }

    #[test]
    fn torus_delta_goes_the_short_way_round() {
        // Two points 10px apart but on opposite edges.
        let a = vec2(5.0, H * 0.5);
        let b = vec2(W - 5.0, H * 0.5);
        assert_eq!(torus_delta(a, b), vec2(-10.0, 0.0));
        assert_eq!(torus_delta(b, a), vec2(10.0, 0.0));
        // And the ordinary case is unchanged.
        assert_eq!(
            torus_delta(vec2(10.0, 10.0), vec2(30.0, 20.0)),
            vec2(20.0, 10.0)
        );
    }

    #[test]
    fn collisions_see_through_the_edges() {
        let ship = vec2(2.0, 10.0);
        let rock = vec2(W - 2.0, 10.0);
        assert!(hits(ship, SHIP_R, rock, RockSize::Small.radius()));
        // Same separation in the middle of the screen must also hit, and a
        // clearly distant rock must not.
        assert!(hits(vec2(500.0, 10.0), SHIP_R, vec2(504.0, 10.0), 10.5));
        assert!(!hits(vec2(500.0, 10.0), SHIP_R, vec2(600.0, 10.0), 10.5));
    }

    #[test]
    fn rocks_break_down_to_nothing() {
        assert_eq!(RockSize::Large.smaller(), Some(RockSize::Medium));
        assert_eq!(RockSize::Medium.smaller(), Some(RockSize::Small));
        assert_eq!(RockSize::Small.smaller(), None);
        // Arcade scoring, smallest is worth the most.
        assert!(RockSize::Small.score() > RockSize::Medium.score());
        assert!(RockSize::Medium.score() > RockSize::Large.score());
    }

    #[test]
    fn fragments_start_where_the_parent_died_and_move_faster() {
        let parent = Rock::new(vec2(100.0, 100.0), RockSize::Large);
        let frag = Rock::fragment(parent.pos, RockSize::Medium, parent.vel);
        assert_eq!(frag.pos, parent.pos);
        assert_eq!(frag.radius(), RockSize::Medium.radius());
        assert!(frag.vel.length() > 1.0);
        // Siblings spawn on top of each other, so they must start inert.
        assert!(!frag.collidable());
    }

    /// Builds a rock with an exact position and velocity, bypassing the random
    /// spawn so collisions can be checked against known numbers.
    fn rock_at(pos: Vec2, vel: Vec2, size: RockSize) -> Rock {
        let mut rock = Rock::new(pos, size);
        rock.vel = vel;
        rock.grace = 0.0;
        rock
    }

    /// Two rocks overlapping and closing head-on along the x axis.
    fn head_on(a_size: RockSize, b_size: RockSize, speed: f32) -> (Rock, Rock) {
        let gap = a_size.radius() + b_size.radius() - 4.0; // 4px of overlap
        let a = rock_at(vec2(400.0, 300.0), vec2(speed, 0.0), a_size);
        let b = rock_at(vec2(400.0 + gap, 300.0), vec2(-speed, 0.0), b_size);
        (a, b)
    }

    #[test]
    fn rocks_bounce_apart_and_report_the_impact() {
        let (mut a, mut b) = head_on(RockSize::Medium, RockSize::Medium, 50.0);
        let contact = collide_rocks(&mut a, &mut b).expect("overlapping and closing");

        // Closing speed is the sum of the two approach speeds.
        assert!(
            (contact.speed - 100.0).abs() < 0.01,
            "speed {}",
            contact.speed
        );
        // Both reversed, and neither gained energy.
        assert!(a.vel.x < 0.0 && b.vel.x > 0.0);
        assert!(a.vel.x.abs() <= 50.0 && b.vel.x.abs() <= 50.0);
        // And they are no longer overlapping, so the next frame is clean.
        let apart = torus_delta(a.pos, b.pos).length();
        assert!(
            apart >= a.radius() + b.radius(),
            "still overlapping: {apart}"
        );
    }

    #[test]
    fn momentum_is_conserved_and_the_heavy_rock_barely_moves() {
        let (mut a, mut b) = head_on(RockSize::Large, RockSize::Small, 90.0);
        let before = a.vel * a.mass() + b.vel * b.mass();
        let (a_vel, b_vel) = (a.vel, b.vel);
        collide_rocks(&mut a, &mut b).expect("overlapping and closing");

        let after = a.vel * a.mass() + b.vel * b.mass();
        assert!(
            (after - before).length() < 0.01,
            "momentum changed: {before} -> {after}"
        );
        // Masses are 16:1, so the small rock takes almost all of the kick.
        let (dv_large, dv_small) = ((a.vel - a_vel).length(), (b.vel - b_vel).length());
        assert!(
            dv_small > dv_large * 10.0,
            "large {dv_large}, small {dv_small}"
        );
    }

    #[test]
    fn overlapping_rocks_that_are_separating_are_left_alone() {
        // Same overlap, but both already heading away from each other.
        let (mut a, mut b) = head_on(RockSize::Medium, RockSize::Medium, -40.0);
        assert!(collide_rocks(&mut a, &mut b).is_none());
        assert_eq!(a.vel.x, -40.0, "velocity must not be touched");
    }

    #[test]
    fn fragment_grace_stops_the_cascade() {
        let (mut a, mut b) = head_on(RockSize::Medium, RockSize::Medium, 50.0);
        a.grace = FRAGMENT_GRACE;
        b.grace = FRAGMENT_GRACE;
        assert!(
            collide_rocks(&mut a, &mut b).is_none(),
            "siblings must pass through"
        );

        // Once the grace has run out they collide normally.
        for _ in 0..((FRAGMENT_GRACE * 60.0) as i32 + 2) {
            a.update(1.0 / 60.0);
            b.update(1.0 / 60.0);
        }
        assert!(a.collidable() && b.collidable());
        // Put them back on a collision course now that they have drifted.
        let (mut a, mut b) = head_on(RockSize::Medium, RockSize::Medium, 50.0);
        assert!(collide_rocks(&mut a, &mut b).is_some());
    }

    #[test]
    fn rocks_collide_across_the_wrap() {
        let a = rock_at(vec2(4.0, 300.0), vec2(-60.0, 0.0), RockSize::Medium);
        let b = rock_at(vec2(W - 8.0, 300.0), vec2(60.0, 0.0), RockSize::Medium);
        let (mut a, mut b) = (a, b);
        let contact = collide_rocks(&mut a, &mut b).expect("touching around the edge");
        assert!((contact.speed - 120.0).abs() < 0.01);
        // `a` is pushed back to the right (+x), `b` to the left.
        assert!(a.vel.x > 0.0 && b.vel.x < 0.0);
    }

    #[test]
    fn distant_rocks_do_not_interact() {
        let mut a = rock_at(vec2(200.0, 300.0), vec2(80.0, 0.0), RockSize::Large);
        let mut b = rock_at(vec2(600.0, 300.0), vec2(-80.0, 0.0), RockSize::Large);
        assert!(collide_rocks(&mut a, &mut b).is_none());
    }

    #[test]
    fn bullets_expire() {
        let mut b = Bullet::new(vec2(0.0, 0.0), vec2(BULLET_SPEED, 0.0), 0.05, true);
        assert!(b.update(0.02));
        assert!(!b.update(0.05));
    }

    #[test]
    fn ship_glides_and_wraps() {
        let mut ship = Ship::new(vec2(W - 4.0, H * 0.5), 0.0);
        ship.angle = 0.0; // facing +x
        for _ in 0..30 {
            ship.update(1.0 / 60.0, 0.0, true);
        }
        assert!(ship.vel.x > 0.0);
        assert!(
            ship.pos.x < 100.0,
            "should have wrapped past the right edge"
        );

        // Coasting bleeds speed off without reversing it.
        let before = ship.vel.x;
        ship.update(0.5, 0.0, false);
        assert!(ship.vel.x < before && ship.vel.x > 0.0);
    }

    #[test]
    fn ship_breaks_into_one_piece_per_hull_segment() {
        let ship = Ship::new(vec2(W * 0.5, H * 0.5), 0.0);
        assert_eq!(ship.fragments(1.0).len(), HULL.len());
    }

    #[test]
    fn saucer_crosses_once_and_then_leaves() {
        let mut saucer = Saucer::spawn(false);
        let mut alive = true;
        // Twenty seconds is far longer than any crossing takes.
        for _ in 0..1200 {
            alive = saucer.update(1.0 / 60.0);
            if !alive {
                break;
            }
        }
        assert!(!alive, "saucer should have flown off the far side");
    }

    #[test]
    fn small_saucer_aims_at_the_player() {
        let mut saucer = Saucer::spawn(true);
        saucer.pos = vec2(W * 0.5, H * 0.5);
        let target = saucer.pos + vec2(200.0, 0.0);
        // A high score shrinks the aiming error to almost nothing.
        let shot = loop {
            if let Some(shot) = saucer.try_fire(0.5, Some(target), 1_000_000) {
                break shot;
            }
        };
        assert!(shot.vel.x > 0.0, "shot should travel toward the target");
        assert!(shot.vel.y.abs() < shot.vel.x * 0.1);
        assert!(!shot.from_player);
    }
}
