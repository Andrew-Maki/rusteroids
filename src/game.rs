//! Game state: the rules, the wave structure and the screens around them.
//!
//! One `Game` owns everything and is driven by a single `update`/`draw` pair
//! from `main`. The state machine is deliberately flat — attract screen, play,
//! the pause while the ship is in pieces, and game over — because that is all
//! the arcade original had.

use std::f32::consts::TAU;

use macroquad::prelude::*;

use crate::entity::{
    BULLET_LIFE, BULLET_R, BULLET_SPEED, Bullet, Dot, Piece, Rock, RockSize, SHIP_R, Saucer, Ship,
    collide_rocks, hits, random_edge_point, random_point, torus_delta,
};
use crate::render::STROKE;
use crate::sfx::SoundBank;
use crate::{H, PHOSPHOR, W, font};

/// The arcade cabinet allowed four shots in the air at once; it is the single
/// most important constraint on how the game plays.
const MAX_SHOTS: usize = 4;
/// Minimum gap between shots when the fire key is held down.
const FIRE_COOLDOWN: f32 = 0.22;

const START_LIVES: i32 = 3;
const EXTRA_LIFE_EVERY: u32 = 10_000;

/// Hyperspace is a panic button, not a free escape: it occasionally scatters
/// the ship instead of saving it.
const HYPERSPACE_FAILURE: f32 = 0.08;
const HYPERSPACE_COOLDOWN: f32 = 0.8;

/// How clear the middle of the screen has to be before the next ship appears.
const SAFE_RADIUS: f32 = 125.0;
const RESPAWN_INVULN: f32 = 2.0;

/// Seconds between waves, so the last explosion can finish.
const WAVE_GAP: f32 = 1.7;

/// Closing speed at which a rock-vs-rock impact stops being a bounce and
/// becomes a break.
///
/// Worth checking against `RockSize::speed_range`: large rocks travel at
/// 26-62, medium at 46-104 and small at 74-158. So two large rocks can only
/// reach 124 and break each other on a near-flat-out head-on, while the fast
/// fragments you create by shooting will happily demolish them. Lower this to
/// make the playfield more destructive, raise it for more clattering.
const ROCK_BREAK_SPEED: f32 = 115.0;
/// Impacts softer than this are silent and sparkless; without it, rocks that
/// are barely touching produce a constant drizzle of debris.
const ROCK_SPARK_SPEED: f32 = 45.0;
/// Minimum gap between impact sounds, so a pile-up cannot machine-gun them.
const THUD_COOLDOWN: f32 = 0.09;

enum State {
    Attract,
    Playing,
    /// Ship destroyed; debris is flying and we are waiting for room to respawn.
    Dying,
    GameOver,
}

/// Chosen on the title screen, fixed for the run.
#[derive(Copy, Clone, PartialEq, Eq)]
pub enum Mode {
    /// The arcade rules: rocks drift straight through each other.
    Original,
    /// Rocks bounce off each other and shatter on hard impacts, so the
    /// playfield fills with fast fragments you were never paid for.
    Hard,
}

impl Mode {
    /// Index into the per-mode high score table.
    fn index(self) -> usize {
        match self {
            Mode::Original => 0,
            Mode::Hard => 1,
        }
    }

    fn rocks_collide(self) -> bool {
        matches!(self, Mode::Hard)
    }
}

pub struct Game {
    state: State,
    /// Rules for the current run, picked on the title screen.
    mode: Mode,
    /// Countdown used by `Dying` and `GameOver`.
    timer: f32,

    ship: Option<Ship>,
    rocks: Vec<Rock>,
    bullets: Vec<Bullet>,
    saucer: Option<Saucer>,
    dots: Vec<Dot>,
    pieces: Vec<Piece>,

    score: u32,
    /// One high score per mode: hard mode destroys rocks without paying for
    /// them, so the two are not comparable.
    high_scores: [u32; 2],
    lives: i32,
    wave: u32,
    next_extra_life: u32,

    /// Rocks at the start of the wave, which sets the heartbeat tempo.
    wave_rocks: usize,
    beat_timer: f32,
    beat_high: bool,

    saucer_timer: f32,
    wave_gap: f32,
    fire_timer: f32,
    hyperspace_timer: f32,
    thud_timer: f32,

    paused: bool,
    quit: bool,
    sfx: SoundBank,
}

impl Game {
    pub async fn new() -> Self {
        let mut game = Self {
            state: State::Attract,
            mode: Mode::Original,
            timer: 0.0,
            ship: None,
            rocks: Vec::new(),
            bullets: Vec::new(),
            saucer: None,
            dots: Vec::new(),
            pieces: Vec::new(),
            score: 0,
            high_scores: [0; 2],
            lives: 0,
            wave: 0,
            next_extra_life: EXTRA_LIFE_EVERY,
            wave_rocks: 1,
            beat_timer: 0.0,
            beat_high: false,
            saucer_timer: 0.0,
            wave_gap: 0.0,
            fire_timer: 0.0,
            hyperspace_timer: 0.0,
            thud_timer: 0.0,
            paused: false,
            quit: false,
            sfx: SoundBank::load_all().await,
        };
        game.enter_attract();
        game
    }

    pub fn quitting(&self) -> bool {
        self.quit
    }

    // -----------------------------------------------------------------------
    // Top-level update
    // -----------------------------------------------------------------------

    pub fn update(&mut self, dt: f32) {
        self.handle_system_keys();
        if self.paused || self.quit {
            return;
        }
        match self.state {
            State::Attract => self.update_attract(dt),
            State::Playing => self.update_playing(dt),
            State::Dying => self.update_dying(dt),
            State::GameOver => self.update_game_over(dt),
        }
    }

    fn handle_system_keys(&mut self) {
        if is_key_pressed(KeyCode::M) {
            self.sfx.toggle_mute();
        }
        if is_key_pressed(KeyCode::P) && matches!(self.state, State::Playing | State::Dying) {
            self.paused = !self.paused;
            if self.paused {
                self.sfx.silence_loops();
            }
        }
        if is_key_pressed(KeyCode::Escape) {
            // Mid-game, escape abandons the run; on the attract screen it quits.
            match self.state {
                State::Playing | State::Dying | State::GameOver => {
                    self.paused = false;
                    self.enter_attract();
                }
                State::Attract => self.quit = true,
            }
        }
    }

    /// Which mode the player just asked to start, if any.
    fn start_input() -> Option<Mode> {
        if is_key_pressed(KeyCode::Space) || is_key_pressed(KeyCode::Enter) {
            Some(Mode::Original)
        } else if is_key_pressed(KeyCode::H) {
            Some(Mode::Hard)
        } else {
            None
        }
    }

    // -----------------------------------------------------------------------
    // Attract screen
    // -----------------------------------------------------------------------

    fn enter_attract(&mut self) {
        self.state = State::Attract;
        self.ship = None;
        self.saucer = None;
        self.bullets.clear();
        self.pieces.clear();
        self.dots.clear();
        self.rocks.clear();
        self.sfx.silence_loops();
        // A few rocks drifting behind the title, as the cabinet did.
        for _ in 0..7 {
            let size = match rand::gen_range(0, 3) {
                0 => RockSize::Small,
                1 => RockSize::Medium,
                _ => RockSize::Large,
            };
            self.rocks.push(Rock::new(random_point(), size));
        }
    }

    fn update_attract(&mut self, dt: f32) {
        for rock in &mut self.rocks {
            rock.update(dt);
        }
        // The title-screen rocks always bounce, whichever mode is about to be
        // picked: it previews hard mode and keeps the backdrop lively. They
        // never break, so the same handful of shapes drifts for as long as the
        // screen is up.
        self.rocks_vs_rocks(dt, false);
        if let Some(mode) = Self::start_input() {
            self.new_game(mode);
        }
    }

    // -----------------------------------------------------------------------
    // A new game
    // -----------------------------------------------------------------------

    fn new_game(&mut self, mode: Mode) {
        self.mode = mode;
        self.score = 0;
        self.lives = START_LIVES;
        self.wave = 0;
        self.next_extra_life = EXTRA_LIFE_EVERY;
        self.rocks.clear();
        self.bullets.clear();
        self.dots.clear();
        self.pieces.clear();
        self.saucer = None;
        self.fire_timer = 0.0;
        self.hyperspace_timer = 0.0;
        self.paused = false;
        self.state = State::Playing;
        self.ship = Some(Ship::new(vec2(W * 0.5, H * 0.5), RESPAWN_INVULN));
        self.next_wave();
    }

    fn next_wave(&mut self) {
        self.wave += 1;
        // Four rocks to start, two more each wave, capped as on the original
        // board so the screen never becomes unsurvivable.
        let count = (4 + 2 * (self.wave as usize - 1)).min(11);
        for _ in 0..count {
            self.rocks
                .push(Rock::new(self.safe_spawn_point(), RockSize::Large));
        }
        self.wave_rocks = count;
        self.beat_timer = 0.0;
        self.wave_gap = WAVE_GAP;
        self.reset_saucer_timer();
    }

    /// An edge position that is not about to drop a rock onto the player.
    fn safe_spawn_point(&self) -> Vec2 {
        let ship = self.ship.as_ref().map(|s| s.pos);
        for _ in 0..24 {
            let p = random_edge_point();
            match ship {
                Some(sp) if torus_delta(p, sp).length() < SAFE_RADIUS * 1.6 => continue,
                _ => return p,
            }
        }
        random_edge_point()
    }

    // -----------------------------------------------------------------------
    // Playing
    // -----------------------------------------------------------------------

    fn update_playing(&mut self, dt: f32) {
        self.control_ship(dt);
        self.step_world(dt);
        self.collide(dt);
        self.update_saucer(dt, true);
        self.heartbeat(dt);

        if self.rocks.is_empty() {
            self.wave_gap -= dt;
            if self.wave_gap <= 0.0 {
                self.next_wave();
            }
        } else {
            self.wave_gap = WAVE_GAP;
        }
    }

    fn control_ship(&mut self, dt: f32) {
        let turn = (is_key_down(KeyCode::Right) || is_key_down(KeyCode::D)) as i32 as f32
            - (is_key_down(KeyCode::Left) || is_key_down(KeyCode::A)) as i32 as f32;
        let thrust = is_key_down(KeyCode::Up) || is_key_down(KeyCode::W);

        let Some(ship) = &mut self.ship else { return };
        ship.update(dt, turn, thrust);
        let thrusting = ship.thrusting;
        let (nose, heading, vel) = (ship.nose(), ship.heading(), ship.vel);
        self.sfx.set_thrust(thrusting);

        self.fire_timer = (self.fire_timer - dt).max(0.0);
        let want_fire = is_key_pressed(KeyCode::Space)
            || (is_key_down(KeyCode::Space) && self.fire_timer == 0.0);
        let shots = self.bullets.iter().filter(|b| b.from_player).count();
        if want_fire && shots < MAX_SHOTS {
            // Shots inherit the ship's momentum, so you cannot outrun your own
            // bullets — or catch up with them.
            self.bullets.push(Bullet::new(
                nose,
                heading * BULLET_SPEED + vel * 0.5,
                BULLET_LIFE,
                true,
            ));
            self.fire_timer = FIRE_COOLDOWN;
            self.sfx.fire();
        }

        self.hyperspace_timer = (self.hyperspace_timer - dt).max(0.0);
        let jump = is_key_pressed(KeyCode::LeftShift)
            || is_key_pressed(KeyCode::RightShift)
            || is_key_pressed(KeyCode::Down)
            || is_key_pressed(KeyCode::S);
        if jump && self.hyperspace_timer == 0.0 {
            self.hyperspace();
        }
    }

    fn hyperspace(&mut self) {
        self.hyperspace_timer = HYPERSPACE_COOLDOWN;
        self.sfx.hyperspace();
        let Some(ship) = &mut self.ship else { return };
        let from = ship.pos;
        ship.pos = random_point();
        ship.vel = Vec2::ZERO;
        self.spray(from, 8, (30.0, 120.0), 0.35);
        if rand::gen_range(0.0, 1.0) < HYPERSPACE_FAILURE {
            // The jump misfired: the ship arrives in pieces.
            self.kill_ship();
        }
    }

    /// Advances everything that moves on its own.
    fn step_world(&mut self, dt: f32) {
        for rock in &mut self.rocks {
            rock.update(dt);
        }
        self.bullets.retain_mut(|b| b.update(dt));
        self.dots.retain_mut(|d| d.update(dt));
        self.pieces.retain_mut(|p| p.update(dt));
    }

    fn update_saucer(&mut self, dt: f32, may_spawn: bool) {
        if let Some(mut saucer) = self.saucer.take() {
            if saucer.update(dt) {
                // Saucers hold their fire while the player is respawning.
                let target = self.ship.as_ref().map(|s| s.pos);
                if target.is_some()
                    && let Some(shot) = saucer.try_fire(dt, target, self.score)
                {
                    self.bullets.push(shot);
                    self.sfx.fire();
                }
                self.saucer = Some(saucer);
            } else {
                self.reset_saucer_timer();
            }
        } else if may_spawn && !self.rocks.is_empty() {
            self.saucer_timer -= dt;
            if self.saucer_timer <= 0.0 {
                // Small saucers get steadily more likely as the score climbs.
                let p_small = (self.score as f32 / 40_000.0).clamp(0.0, 0.85);
                self.saucer = Some(Saucer::spawn(rand::gen_range(0.0, 1.0) < p_small));
            }
        }

        let loop_kind = self.saucer.as_ref().map(|s| s.small);
        self.sfx.set_saucer(loop_kind);
    }

    fn reset_saucer_timer(&mut self) {
        let base = (10.0 - self.wave as f32 * 0.5).max(4.0);
        self.saucer_timer = base + rand::gen_range(-1.5, 3.0);
    }

    /// The two-note pulse under the whole game, speeding up as the wave thins
    /// out. It is the only music there is, and it does all the tension work.
    fn heartbeat(&mut self, dt: f32) {
        self.beat_timer -= dt;
        if self.beat_timer > 0.0 {
            return;
        }
        let left = self.rocks.len() as f32;
        let total = self.wave_rocks.max(1) as f32;
        let progress = (1.0 - left / total).clamp(0.0, 1.0);
        self.beat_timer = 0.95 - 0.63 * progress;
        self.sfx.beat(self.beat_high);
        self.beat_high = !self.beat_high;
    }

    // -----------------------------------------------------------------------
    // Collisions
    // -----------------------------------------------------------------------

    fn collide(&mut self, dt: f32) {
        // Rocks first, so everything else tests against post-bounce positions.
        if self.mode.rocks_collide() {
            self.rocks_vs_rocks(dt, true);
        }
        self.bullets_vs_rocks();
        self.bullets_vs_saucer();
        self.bullets_vs_ship();
        self.saucer_vs_rocks();
        self.ship_vs_world();
    }

    /// Rocks bounce off each other, and shatter when the hit is hard enough.
    ///
    /// Only called in hard mode, apart from the attract screen. `may_shatter`
    /// is false there, where the rocks are scenery and should just clatter
    /// around indefinitely.
    ///
    /// The pair loop is O(n²), but n is capped at a few dozen even in a late
    /// wave, so a broad phase would cost more than it saves.
    fn rocks_vs_rocks(&mut self, dt: f32, may_shatter: bool) {
        let mut shatter: Vec<usize> = Vec::new();
        let mut loudest: Option<(Vec2, f32)> = None;

        for i in 0..self.rocks.len() {
            for j in (i + 1)..self.rocks.len() {
                // Disjoint borrows of two elements of the same Vec.
                let (head, tail) = self.rocks.split_at_mut(j);
                let Some(contact) = collide_rocks(&mut head[i], &mut tail[0]) else {
                    continue;
                };

                if contact.speed >= ROCK_SPARK_SPEED
                    && loudest.is_none_or(|(_, s)| contact.speed > s)
                {
                    loudest = Some((contact.point, contact.speed));
                }
                if may_shatter && contact.speed >= ROCK_BREAK_SPEED {
                    // A small rock has nothing to break into, so it only ever
                    // bounces — otherwise small-vs-small would vaporise both
                    // and a wave could finish itself without the player.
                    for k in [i, j] {
                        if self.rocks[k].size.smaller().is_some() && !shatter.contains(&k) {
                            shatter.push(k);
                        }
                    }
                }
            }
        }

        // One sound and one spark per frame, from the biggest impact, so a
        // pile-up does not turn into a wall of noise. A hit hard enough to
        // shatter needs neither: the explosion covers it.
        self.thud_timer = (self.thud_timer - dt).max(0.0);
        if let Some((point, speed)) = loudest {
            let shattered = may_shatter && speed >= ROCK_BREAK_SPEED;
            if !shattered {
                let n = (speed / 60.0).clamp(1.0, 4.0) as usize;
                self.spray(point, n, (25.0, 70.0), 0.25);
                if self.thud_timer == 0.0 {
                    self.sfx.thud();
                    self.thud_timer = THUD_COOLDOWN;
                }
            }
        }

        // Descending order: `swap_remove` only ever moves an element from the
        // tail, which is past every index still waiting to be processed.
        shatter.sort_unstable_by(|a, b| b.cmp(a));
        for index in shatter {
            // Collisions award nothing — the player has to land the hit.
            self.destroy_rock(index, false);
        }
    }

    fn bullets_vs_rocks(&mut self) {
        let mut i = 0;
        while i < self.bullets.len() {
            let bullet = self.bullets[i];
            let hit = self
                .rocks
                .iter()
                .position(|r| hits(bullet.pos, BULLET_R, r.pos, r.radius()));
            match hit {
                Some(j) => {
                    self.bullets.swap_remove(i);
                    // Saucer shots break rocks but earn the player nothing.
                    self.destroy_rock(j, bullet.from_player);
                }
                None => i += 1,
            }
        }
    }

    fn bullets_vs_saucer(&mut self) {
        let Some(saucer) = &self.saucer else { return };
        let r = saucer.radius();
        let pos = saucer.pos;
        let hit = self
            .bullets
            .iter()
            .position(|b| b.from_player && hits(b.pos, BULLET_R, pos, r));
        if let Some(i) = hit {
            self.bullets.swap_remove(i);
            let saucer = self.saucer.take().expect("checked above");
            self.dots.extend(saucer.burst());
            self.sfx.bang(crate::sfx::BangSize::Medium);
            self.add_score(saucer.score());
            self.reset_saucer_timer();
        }
    }

    fn bullets_vs_ship(&mut self) {
        let Some(ship) = &self.ship else { return };
        if ship.invuln > 0.0 {
            return;
        }
        let pos = ship.pos;
        let hit = self
            .bullets
            .iter()
            .position(|b| !b.from_player && hits(b.pos, BULLET_R, pos, SHIP_R));
        if let Some(i) = hit {
            self.bullets.swap_remove(i);
            self.kill_ship();
        }
    }

    /// A saucer that flies into a rock takes it with it, and scores nothing.
    fn saucer_vs_rocks(&mut self) {
        let Some(saucer) = &self.saucer else { return };
        let (pos, r) = (saucer.pos, saucer.radius());
        if let Some(j) = self
            .rocks
            .iter()
            .position(|k| hits(pos, r, k.pos, k.radius()))
        {
            let saucer = self.saucer.take().expect("checked above");
            self.dots.extend(saucer.burst());
            self.sfx.bang(crate::sfx::BangSize::Medium);
            self.destroy_rock(j, false);
            self.reset_saucer_timer();
        }
    }

    fn ship_vs_world(&mut self) {
        let Some(ship) = &self.ship else { return };
        if ship.invuln > 0.0 {
            return;
        }
        let pos = ship.pos;

        if let Some(j) = self
            .rocks
            .iter()
            .position(|r| hits(pos, SHIP_R, r.pos, r.radius()))
        {
            self.kill_ship();
            self.destroy_rock(j, false);
            return;
        }
        if let Some(saucer) = &self.saucer
            && hits(pos, SHIP_R, saucer.pos, saucer.radius())
        {
            let saucer = self.saucer.take().expect("checked above");
            self.dots.extend(saucer.burst());
            self.sfx.bang(crate::sfx::BangSize::Medium);
            self.reset_saucer_timer();
            self.kill_ship();
        }
    }

    /// Removes rock `index`, splitting it in two unless it was already small.
    fn destroy_rock(&mut self, index: usize, award: bool) {
        let rock = self.rocks.swap_remove(index);
        self.sfx.bang(rock.size.bang());
        self.dots.extend(rock.burst());
        if award {
            self.add_score(rock.size.score());
        }
        if let Some(next) = rock.size.smaller() {
            for _ in 0..2 {
                self.rocks.push(Rock::fragment(rock.pos, next, rock.vel));
            }
        }
    }

    fn kill_ship(&mut self) {
        if let Some(ship) = self.ship.take() {
            self.pieces.extend(ship.fragments(1.7));
            self.spray(ship.pos, 10, (40.0, 140.0), 0.6);
        }
        self.sfx.silence_loops();
        self.sfx.ship_explode();
        self.lives -= 1;
        self.state = State::Dying;
        self.timer = 1.9;
    }

    fn spray(&mut self, pos: Vec2, n: usize, speed: (f32, f32), max_life: f32) {
        for _ in 0..n {
            let dir = Vec2::from_angle(rand::gen_range(0.0, TAU));
            let life = rand::gen_range(max_life * 0.4, max_life);
            self.dots.push(Dot {
                pos,
                vel: dir * rand::gen_range(speed.0, speed.1),
                life,
                max_life: life,
            });
        }
    }

    fn add_score(&mut self, points: u32) {
        self.score += points;
        self.record_high_score();
        while self.score >= self.next_extra_life {
            self.lives += 1;
            self.next_extra_life += EXTRA_LIFE_EVERY;
            self.sfx.extra_life();
        }
    }

    fn high_score(&self) -> u32 {
        self.high_scores[self.mode.index()]
    }

    fn record_high_score(&mut self) {
        let slot = &mut self.high_scores[self.mode.index()];
        *slot = (*slot).max(self.score);
    }

    // -----------------------------------------------------------------------
    // Dying / game over
    // -----------------------------------------------------------------------

    fn update_dying(&mut self, dt: f32) {
        self.step_world(dt);
        // Rocks and saucers keep knocking each other about while the player
        // is out of the game.
        if self.mode.rocks_collide() {
            self.rocks_vs_rocks(dt, true);
        }
        self.bullets_vs_rocks();
        self.saucer_vs_rocks();
        self.update_saucer(dt, false);
        self.heartbeat(dt);

        if self.rocks.is_empty() {
            self.wave_gap -= dt;
            if self.wave_gap <= 0.0 {
                self.next_wave();
            }
        }

        self.timer -= dt;
        if self.timer > 0.0 {
            return;
        }
        if self.lives > 0 {
            // Hold the respawn until the middle of the screen is survivable.
            if self.area_clear() {
                self.ship = Some(Ship::new(vec2(W * 0.5, H * 0.5), RESPAWN_INVULN));
                self.state = State::Playing;
            }
        } else {
            self.state = State::GameOver;
            self.timer = 6.0;
            self.record_high_score();
            self.sfx.silence_loops();
        }
    }

    fn area_clear(&self) -> bool {
        let centre = vec2(W * 0.5, H * 0.5);
        let rocks_ok = self
            .rocks
            .iter()
            .all(|r| torus_delta(centre, r.pos).length() > SAFE_RADIUS + r.radius());
        let saucer_ok = self
            .saucer
            .as_ref()
            .is_none_or(|s| torus_delta(centre, s.pos).length() > SAFE_RADIUS + s.radius());
        rocks_ok && saucer_ok
    }

    fn update_game_over(&mut self, dt: f32) {
        self.step_world(dt);
        for rock in &mut self.rocks {
            rock.update(dt);
        }
        self.timer -= dt;
        if self.timer <= 0.0 || Self::start_input().is_some() {
            self.enter_attract();
        }
    }

    // -----------------------------------------------------------------------
    // Drawing
    // -----------------------------------------------------------------------

    pub fn draw(&self) {
        // One scratch buffer for every transformed shape this frame.
        let mut buf: Vec<Vec2> = Vec::with_capacity(16);

        for rock in &self.rocks {
            rock.draw(&mut buf);
        }
        if let Some(saucer) = &self.saucer {
            saucer.draw(&mut buf);
        }
        for bullet in &self.bullets {
            bullet.draw();
        }
        for dot in &self.dots {
            dot.draw();
        }
        for piece in &self.pieces {
            piece.draw(&mut buf);
        }
        if let Some(ship) = &self.ship {
            ship.draw(&mut buf);
        }

        match self.state {
            State::Attract => self.draw_attract(),
            State::Playing | State::Dying => self.draw_hud(&mut buf),
            State::GameOver => {
                self.draw_hud(&mut buf);
                self.draw_game_over();
            }
        }
        if self.paused {
            font::draw_centered("PAUSED", W * 0.5, H * 0.5 - 18.0, 36.0, STROKE, PHOSPHOR);
        }
        if self.sfx.muted() {
            font::draw("MUTED", 24.0, H - 30.0, 16.0, STROKE * 0.7, dim(0.45));
        }
    }

    fn draw_hud(&self, buf: &mut Vec<Vec2>) {
        font::draw(&self.score.to_string(), 26.0, 24.0, 30.0, STROKE, PHOSPHOR);

        // Spare ships, left to right under the score.
        for i in 0..self.lives.saturating_sub(1).min(9) {
            Ship::draw_icon(buf, vec2(38.0 + i as f32 * 26.0, 84.0), 0.8);
        }

        let high = format!("HIGH {}", self.high_score());
        font::draw_centered(&high, W * 0.5, 26.0, 18.0, STROKE * 0.75, dim(0.6));

        let wave = format!("WAVE {}", self.wave);
        let size = 18.0;
        font::draw(
            &wave,
            W - 26.0 - font::text_width(&wave, size),
            26.0,
            size,
            STROKE * 0.75,
            dim(0.6),
        );

        // Badge under the wave counter, so a screenshot says which rules were
        // in play. Original mode is the default and needs no label.
        if self.mode == Mode::Hard {
            let label = "HARD";
            let size = 15.0;
            font::draw(
                label,
                W - 26.0 - font::text_width(label, size),
                52.0,
                size,
                STROKE * 0.7,
                dim(0.75),
            );
        }
    }

    fn draw_attract(&self) {
        font::draw_centered(
            "RUSTEROIDS",
            W * 0.5,
            H * 0.17,
            72.0,
            STROKE * 1.3,
            PHOSPHOR,
        );

        // Two ways in. Space blinks because it is the default; hard mode sits
        // underneath it, with a line each explaining the difference.
        if blink(1.0, 0.62) {
            font::draw_centered(
                "PRESS SPACE TO PLAY",
                W * 0.5,
                H * 0.33,
                26.0,
                STROKE,
                PHOSPHOR,
            );
        }
        font::draw_centered(
            "ORIGINAL ARCADE RULES",
            W * 0.5,
            H * 0.385,
            15.0,
            STROKE * 0.7,
            dim(0.5),
        );

        font::draw_centered(
            "PRESS H FOR HARD MODE",
            W * 0.5,
            H * 0.45,
            22.0,
            STROKE * 0.9,
            dim(0.9),
        );
        font::draw_centered(
            "ROCKS COLLIDE AND SMASH EACH OTHER APART",
            W * 0.5,
            H * 0.505,
            15.0,
            STROKE * 0.7,
            dim(0.5),
        );

        let rows = [
            ("ROTATE", "LEFT / RIGHT"),
            ("THRUST", "UP"),
            ("FIRE", "SPACE"),
            ("HYPERSPACE", "SHIFT"),
            ("PAUSE / MUTE", "P / M"),
            ("QUIT", "ESC"),
        ];
        let size = 16.0;
        let line = size * 1.75;
        let top = H * 0.60;
        for (i, (action, keys)) in rows.iter().enumerate() {
            let y = top + i as f32 * line;
            let label_right = W * 0.5 - 18.0;
            font::draw(
                action,
                label_right - font::text_width(action, size),
                y,
                size,
                STROKE * 0.7,
                dim(0.55),
            );
            font::draw(keys, W * 0.5 + 18.0, y, size, STROKE * 0.7, dim(0.8));
        }

        // A high score per mode, since the two are not comparable.
        let mut y = H - 72.0;
        for (label, mode) in [("HIGH SCORE", Mode::Original), ("HARD MODE", Mode::Hard)] {
            let best = self.high_scores[mode.index()];
            if best == 0 {
                continue;
            }
            let s = format!("{label} {best}");
            font::draw_centered(&s, W * 0.5, y, 17.0, STROKE * 0.75, dim(0.6));
            y += 26.0;
        }
    }

    fn draw_game_over(&self) {
        font::draw_centered("GAME OVER", W * 0.5, H * 0.36, 54.0, STROKE * 1.2, PHOSPHOR);
        if self.mode == Mode::Hard {
            font::draw_centered(
                "HARD MODE",
                W * 0.5,
                H * 0.46,
                18.0,
                STROKE * 0.75,
                dim(0.6),
            );
        }
        let s = format!("SCORE {}", self.score);
        font::draw_centered(&s, W * 0.5, H * 0.53, 24.0, STROKE, dim(0.8));
        if self.score >= self.high_score() && self.score > 0 && blink(0.9, 0.55) {
            font::draw_centered(
                "NEW HIGH SCORE",
                W * 0.5,
                H * 0.61,
                20.0,
                STROKE * 0.8,
                PHOSPHOR,
            );
        }
        if blink(1.0, 0.62) {
            font::draw_centered(
                "PRESS SPACE OR H",
                W * 0.5,
                H * 0.72,
                20.0,
                STROKE * 0.8,
                dim(0.7),
            );
        }
    }
}

/// Phosphor at a fraction of full brightness, for secondary HUD text.
fn dim(alpha: f32) -> Color {
    Color::new(PHOSPHOR.r, PHOSPHOR.g, PHOSPHOR.b, alpha)
}

/// True for the first `duty` seconds of each `period`-second cycle.
fn blink(period: f32, duty: f32) -> bool {
    (get_time() as f32).rem_euclid(period) < duty
}
