//! Rusteroids — a Rust reimplementation of Atari's 1979 Asteroids cabinet.
//!
//! The playfield is a fixed 1024x768 world drawn through a camera, so the
//! window can be any size (or fullscreen) without the game logic having to
//! care: all coordinates, speeds and radii are in world units.
//!
//! * `render` — the fake-phosphor line renderer
//! * `font`   — stroke font for the HUD and screens
//! * `sfx`    — procedurally synthesised audio
//! * `entity` — ship, rocks, saucers, bullets, debris
//! * `game`   — the rules and the screens

mod entity;
mod font;
mod game;
mod render;
mod sfx;

use macroquad::prelude::*;

use game::Game;

/// Playfield size in world units.
pub const W: f32 = 1024.0;
pub const H: f32 = 768.0;

/// Slightly blue-white, like a real XY monitor's P39-ish phosphor read through
/// the cabinet's bezel.
pub const PHOSPHOR: Color = Color::new(0.88, 0.94, 1.0, 1.0);

fn window_conf() -> Conf {
    Conf {
        window_title: "Rusteroids".to_owned(),
        window_width: W as i32,
        window_height: H as i32,
        window_resizable: true,
        high_dpi: true,
        // Multisampling matters here: every single thing on screen is a
        // diagonal line, and aliased lines ruin the glow illusion.
        sample_count: 4,
        ..Default::default()
    }
}

/// Maps the fixed world rect onto whatever the window currently is, with +y
/// pointing down as it does in screen space.
///
/// `Camera2D::from_display_rect` cannot be used here: it supplies a negative
/// `zoom.y`, and macroquad *also* negates `zoom.y` whenever the camera has no
/// render target, so the two cancel and the playfield comes out mirrored
/// vertically.
fn playfield_camera() -> Camera2D {
    Camera2D {
        target: vec2(W * 0.5, H * 0.5),
        zoom: vec2(2.0 / W, 2.0 / H),
        ..Default::default()
    }
}

#[macroquad::main(window_conf)]
async fn main() {
    // Seed from the wall clock so rock shapes and saucer timing differ per run.
    rand::srand(macroquad::miniquad::date::now() as u64);

    let mut game = Game::new().await;

    loop {
        // Clamp the step: a stall (window drag, alt-tab) must not teleport
        // rocks through the ship.
        let dt = get_frame_time().min(1.0 / 30.0);

        game.update(dt);
        if game.quitting() {
            break;
        }

        clear_background(BLACK);
        set_camera(&playfield_camera());
        game.draw();

        next_frame().await;
    }
}
