# Rusteroids

A clone of Atari's 1979 arcade game **Asteroids**, written from scratch in Rust.

It aims at the feel of the original cabinet rather than a modern remake: a
monochrome vector playfield on a toroidal screen, momentum you have to fight,
four shots in the air at a time, and the two-note heartbeat that speeds up as
the rocks thin out.

Everything on screen and everything you hear is generated at runtime — there
are no image, font or audio files anywhere in the project:

- **Vector rendering.** Every object is a line segment. Each segment is stroked
  three times, wide-and-dim to narrow-and-bright, which reads as phosphor glow
  without needing a shader or a render target.
- **Stroke font.** The HUD and screens use a hand-built polyline font, because a
  raster font would look wrong next to the wireframe art.
- **Procedural audio.** The thrust rumble, rock bangs, saucer warble, fire blip
  and hyperspace sweep are synthesised into buffers at startup and encoded as
  in-memory WAVs. Nothing touches the filesystem.

The only dependency is [macroquad](https://github.com/not-fl3/macroquad) for the
window, input and audio playback.

## Requirements

- **Rust 1.88 or newer** (the crate uses edition 2024 and let-chains). Install
  from [rustup.rs](https://rustup.rs).
- A GPU/driver with OpenGL 2.1+ (or DirectX 11 via ANGLE on Windows) — anything
  from the last fifteen years is fine.
- Linux also needs the usual X11/ALSA development packages, e.g. on Debian or
  Ubuntu:
  ```sh
  sudo apt install libx11-dev libxi-dev libgl1-mesa-dev libasound2-dev
  ```

Audio is optional at runtime: if no sound device is available the game detects
it and stays silent instead of failing.

## Build and run

Clone, then from the project root:

```sh
# play it (release is recommended — the glow renderer strokes every line 3x)
cargo run --release
```

To build and launch the executable separately:

```sh
cargo build --release

# Windows
.\target\release\rusteroids.exe

# macOS / Linux
./target/release/rusteroids
```

A debug build (`cargo run`) also works and starts compiling faster, but the
frame times are noticeably worse.

Run the test suite — the playfield maths, collisions, rock splitting and saucer
aiming are all covered headlessly, with no window or audio device needed:

```sh
cargo test
```

## Controls

| Action | Keys |
| --- | --- |
| Rotate | `Left` / `Right`, or `A` / `D` |
| Thrust | `Up` or `W` |
| Fire | `Space` |
| Hyperspace | `Shift`, or `Down` / `S` |
| Start game | `Space` or `Enter` |
| Pause | `P` |
| Mute | `M` |
| Abandon game / quit | `Esc` |

`Esc` drops you from a game back to the title screen; pressing it again on the
title screen exits.

## How it plays

- **Three lives**, plus an extra ship every 10,000 points.
- **Four shots** on screen at once, and they expire after about two thirds of a
  screen width. Shots inherit the ship's momentum, so you can neither outrun
  your own bullets nor catch up with them.
- **Waves** start with four large rocks and add two more each wave, capped at
  eleven. Large rocks break into two medium, medium into two small, and small
  ones vaporise. Fragments fly faster than their parent.
- **Saucers** cross the screen on a staircase path. The large one sprays shots
  at random; the small one aims at you, and its accuracy improves as your score
  climbs. A saucer that flies into a rock destroys both and scores you nothing.
- **Hyperspace** teleports you somewhere random and kills your momentum, but it
  misfires about 8% of the time and scatters the ship instead. It is a panic
  button, not a free escape.
- After you die the next ship is held back until the middle of the screen is
  survivable, then spawns with two seconds of blinking invulnerability.

### Scoring

| Target | Points |
| --- | --- |
| Large rock | 20 |
| Medium rock | 50 |
| Small rock | 100 |
| Large saucer | 200 |
| Small saucer | 1,000 |

Rocks broken by a saucer's shots score nothing — you have to land the hit
yourself.

## Project layout

| File | Contents |
| --- | --- |
| `src/main.rs` | Window setup, the 1024x768 world, camera, frame loop |
| `src/game.rs` | Rules, waves, collisions, scoring, HUD and screens |
| `src/entity.rs` | Ship, rocks, saucers, bullets, debris, toroidal maths |
| `src/render.rs` | The fake-phosphor line renderer |
| `src/font.rs` | Stroke font |
| `src/sfx.rs` | Sound synthesis and the sound bank |

The playfield is a fixed 1024x768 world drawn through a camera, so the window
can be resized freely without the game logic having to care — every coordinate,
speed and radius is in world units.

## Credits

Asteroids was designed by Lyle Rains and Ed Logg and released by Atari in 1979;
the name and the game are their trademark and property. This is an independent
reimplementation written for fun, and it contains no original Atari code,
artwork, ROM data or other assets.
