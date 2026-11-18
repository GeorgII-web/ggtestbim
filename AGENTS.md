# ggtestbim

Personal 3D game in Rust on [Bevy](https://bevyengine.org) 0.20, target Windows x86_64.

## Code

| Module | What |
|---|---|
| `src/main.rs` | app, sun, sky, ambient light, help text |
| `src/level.rs` | seeded city generator: street grid, buildings (rooms, doors, windows, stairs, walkable roof), plazas, parks; every part is an axis-aligned box that is both the mesh and the collider (`Level.boxes`); R regenerates with the next seed |
| `src/player.rs` | first-person walker: mouse look, WASD, Shift run, Space jump, gravity, box collision with step-up (max 0.55 m), substeps against tunneling, respawn below -30 m or on R |

Geometry rules the collider relies on: walls, slabs and steps are boxes; a stair step rises 0.25 m (under the step-up
limit); slabs are at least 0.2 m thick (above the 0.15 m substep).

## Build

- The repo lives in WSL; the game is cross-built to a Windows exe with MinGW (no Windows Rust toolchain needed).
- `scripts/cargo-gnu.sh build [--release]` → `target/x86_64-pc-windows-gnu/<profile>/ggtestbim.exe`.
- `scripts/run-win.sh [--release]` builds, copies the exe (and `assets/`) to `C:\dev\ggtestbim` and starts it.
- Plain `cargo build` (Linux) needs `pkg-config`, `libudev-dev`, `libasound2-dev`, `libwayland-dev`; not set up.
- Gates before a commit: `cargo fmt --check`, `scripts/cargo-gnu.sh clippy -- -D warnings`.

## Rules

- Work and temp files go to the git-ignored `var/`, never the repo root.
- One commit per logical change, short imperative subject.
