# ggtestbim

Personal 3D game in Rust on [Bevy](https://bevyengine.org) 0.20, target Windows x86_64.

## Code

| Module | What |
|---|---|
| `src/main.rs` | app, sun, sky, ambient light, help text |
| `src/level.rs` | endless world streamed in 60 m chunks (2×2 blocks): chunks within 3 of the player are generated (2 per frame, nearest first), chunks beyond 4 are dropped, a revisited area is generated anew with a fresh seed. Each chunk has a random theme (downtown, hills, parkland, mixed): terraces with stairs, buildings (rooms, doors, windows, stairs to a walkable roof), plazas, parks, corridors between upper floors and bridges between roofs. Every part is an axis-aligned box: collider per chunk (`Level::near`) and, merged per material, one mesh with world-space UVs; R drops the world |
| `src/textures.rs` | procedural tileable grayscale textures (plaster, brick, tiles, asphalt, grass, wood, concrete, stone, leaves, water, bark) with a mip chain, tinted by the material color |
| `src/player.rs` | first-person walker: mouse look, acceleration, run by default, Shift walk, Ctrl crouch (0.9 m, fits a window), jump with coyote time and buffer, gravity, box collision with step-up (also in the air, onto sills), snap-down on stairs, head bob, landing dip, distance fog; respawn below -30 m or on R |

Geometry rules the collider relies on: walls, slabs and steps are boxes; a stair step rises 0.25 m (under the 0.55 m
step-up); slabs are at least 0.2 m thick (above the 0.15 m substep); no two boxes share a visible face plane
(slabs stop at the inner wall face), or the faces flicker.

## Build

- The repo lives in WSL; the game is cross-built to a Windows exe with MinGW (no Windows Rust toolchain needed).
- `scripts/cargo-gnu.sh build [--release]` → `target/x86_64-pc-windows-gnu/<profile>/ggtestbim.exe`.
- `scripts/run-win.sh [--release]` builds, copies the exe (and `assets/`) to `C:\dev\ggtestbim` and starts it.
- Plain `cargo build` (Linux) needs `pkg-config`, `libudev-dev`, `libasound2-dev`, `libwayland-dev`; not set up.
- Gates before a commit: `cargo fmt --check`, `scripts/cargo-gnu.sh clippy -- -D warnings`.

## Rules

- Work and temp files go to the git-ignored `var/`, never the repo root.
- One commit per logical change, short imperative subject.
