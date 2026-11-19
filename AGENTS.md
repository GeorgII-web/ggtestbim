# ggtestbim

Personal 3D game in Rust on [Bevy](https://bevyengine.org) 0.20, target Windows x86_64.

## Code

| Module | What |
|---|---|
| `src/main.rs` | app, sun, sky, ambient light, plugin list |
| `src/level.rs` | endless world streamed in 60 m chunks (2×2 blocks): chunks within 3 of the player are generated (2 per frame, nearest first), chunks beyond 4 are dropped, a revisited area is generated anew with a fresh seed. Each chunk has a random theme (downtown, hills, parkland, mixed): terraces with stairs, buildings (rooms, doors, windows, a switchback stairwell to a walkable roof), plazas, parks, corridors between upper floors and bridges between roofs. Static parts are axis-aligned boxes: collider per chunk (`Level::near`) and, merged per material, one mesh with world-space UVs. Trees and benches are props (own entities, `PropBoxes` colliders) that change while behind the player (`haunt`). Materials: 10 fixed plus 4 floor and 16 wall styles random per world. Window spots for watchers. Indoor volumes (inside a building's outer walls up to the roof, covered corridors): faces in them use an indoor copy of their material with a 1×1 occlusion map, so they get only `INDOOR_AMBIENT` (15 %) of the ambient light and rooms are lit by sun through windows and doors. R drops the world |
| `src/textures.rs` | procedural tileable grayscale textures, varied by a seed (sizes, counts, direction), with a mip chain, tinted by the material color; wall and floor kinds for the random styles |
| `src/player.rs` | first-person walker: mouse look, low acceleration, run 15 m/s by default, Shift walk, Ctrl crouch (0.9 m, fits a window), jump with coyote time and buffer, gravity, box collision against chunks, cars and props with step-up (also in the air, onto sills), snap-down on stairs, landing dip, distance fog; respawn below -30 m or on R. Tests: climbing a 6-story stairwell and a terrace stair |
| `src/cars.rs` | 3 cars per new chunk on street lanes, driving straight, braking for the player ahead, removed in unloaded chunks; `CarBoxes` colliders |
| `src/watchers.rs` | rare figures (15 % of chunks) behind upper-floor windows, facing the player; within 16 m they move to a window over 30 m away or hide; unseen, they may move |
| `src/clouds.rs` | 45 random sphere clouds 90–140 m up, drifting with the wind, recycled with a new shape on the far side when out of range |
| `src/hud.rs` | top line: `~ help` at the left (~ unfolds the controls), compass strip with the heading at the center, fps at the right |

Geometry rules the collider relies on: walls, slabs and steps are boxes; a stair step rises 0.25 m (under the 0.55 m
step-up); stair flights never lie inside another flight's head room (switchback lanes); slabs are at least 0.2 m thick (above the 0.15 m substep); no two boxes share a visible face plane
(slabs stop at the inner wall face), or the faces flicker.

## Build

- The repo lives in WSL; the game is cross-built to a Windows exe with MinGW (no Windows Rust toolchain needed).
- `scripts/cargo-gnu.sh build [--release]` → `target/x86_64-pc-windows-gnu/<profile>/ggtestbim.exe`.
- `scripts/run-win.sh [--release]` builds, copies the exe (and `assets/`) to `C:\dev\ggtestbim` and starts it.
- Plain `cargo build` (Linux) needs `pkg-config`, `libudev-dev`, `libasound2-dev`, `libwayland-dev`; not set up.
- Gates before a commit: `cargo fmt --check`, `scripts/cargo-gnu.sh clippy --all-targets -- -D warnings`, and the
  tests: `scripts/cargo-gnu.sh test --no-run`, copy the printed test exe to a Windows folder (`C:\dev\ggtestbim-test`)
  and run it there (Linux can't build Bevy here; the exe needs Windows).

## Performance

The target machine has an integrated Intel Iris Xe GPU (Vulkan): the cost is per pixel, not geometry (a chunk is ~730
boxes). MSAA is off, shadows are 2 cascades to 50 m on a 1024 map with the `Hardware2x2` filter, textures use 4×
anisotropy, 5×5 chunks are loaded with fog at 45–115 m; that measured 50–65 fps (vsync 60), against ~22 fps with
MSAA 4×, 3–4 cascades and 7×7 chunks. G toggles shadows in game; the HUD shows fps. Measure before raising any of these.

## Rules

- Work and temp files go to the git-ignored `var/`, never the repo root.
- One commit per logical change, short imperative subject.
