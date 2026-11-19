# ggtestbim

Personal 3D game in Rust on [Bevy](https://bevyengine.org) 0.20, target Windows x86_64.

## Code

| Module | What |
|---|---|
| `src/main.rs` | app, sun, sky, ambient light, plugin list |
| `src/level.rs` | endless world streamed in 60 m chunks (2×2 blocks): chunks within 3 of the player are generated (2 per frame, nearest first), chunks beyond 4 are dropped, a revisited area is generated anew with a fresh seed. Each chunk has a random theme (downtown, hills, parkland, mixed): terraces with stairs, buildings (rooms, doors, windows, a switchback stairwell to a walkable roof), plazas, parks, corridors between upper floors and bridges between roofs. Static parts are axis-aligned boxes: collider per chunk (`Level::near`) and, merged per material, one mesh with world-space UVs. Trees and benches are props (own entities, `PropBoxes` colliders) that change while behind the player (`haunt`). Materials: 10 fixed plus 4 floor and 16 wall styles random per world. Watcher spots: behind windows of every floor, at building corners, on roof corners. Indoor volumes (inside a building's outer walls up to the roof, covered corridors): faces in them use an indoor copy of their material with a 1×1 occlusion map, so they get only `INDOOR_AMBIENT` (15 %) of the ambient light and rooms are lit by sun through windows and doors. R drops the world |
| `src/textures.rs` | procedural tileable grayscale textures, varied by a seed (sizes, counts, direction), with a mip chain, tinted by the material color; wall and floor kinds for the random styles |
| `src/player.rs` | first-person walker: mouse look, instant direction on the ground and a 0.25 s glide after release, run 12 m/s by default, Shift walk 3 m/s, Ctrl crouch (0.9 m, fits a window), jump (apex 0.94 m; crouched 1.12 m, onto a 1.5 m terrace ledge) with coyote time and buffer, gravity, box collision against chunks, cars and props with step-up (also in the air, onto sills), snap-down on stairs, landing dip, distance fog; respawn below -30 m or on R. Tests: climbing a 6-story stairwell and a terrace stair, crouch jumps through a window and onto a terrace ledge |
| `src/cars.rs` | 3 cars per new chunk on street lanes, driving straight, braking for the player ahead, removed in unloaded chunks; `CarBoxes` colliders |
| `src/watchers.rs` | tall dark figures, up to 2 per new chunk (80 % each), at its spots: behind windows of every floor, pressed to building corners (peeking), on roof corners; facing the player; within 20 m they jump to a spot over 30 m away or hide; once looked at, they move as soon as they are off screen; unseen, they may wander. Never catchable |
| `src/clouds.rs` | 45 random sphere clouds 90–140 m up, drifting with the wind, recycled with a new shape on the far side when out of range |
| `src/optic.rs` | rifle-scope optic: right click on/off (eases in to the last zoom, first 10x), smooth wheel zoom 2–30x, FOV and mouse look scaled by the zoom, overlay (lens square and black side bars placed in pixels from the window size, no gaps) with a generated reticle texture (black outside the lens, rim shade, crosshair with posts and mil dots) under the HUD, zoom label |
| `src/hud.rs` | top line: `~ help` at the left (~ unfolds the controls), compass strip (S in red) with the heading right of it at the center, fps at the right |

Geometry rules the collider relies on: walls, slabs and steps are boxes; a stair step rises 0.25 m (under the 0.55 m
step-up); stair flights never lie inside another flight's head room (switchback lanes); slabs are at least 0.2 m thick (above the 0.15 m substep); no two boxes share a visible face plane
(slabs stop at the inner wall face, interior walls stop under the slab above, stair railings sit inside the hole's
lane, plaza low walls differ in height, benches keep apart), or the faces flicker; the test
`level::tests::no_coplanar_visible_faces` checks this on 20 generated chunks (props left out).

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
boxes). MSAA is off, shadows are 2 cascades to 50 m (the first to 6 m, against saw-tooth shadow edges up close) on a 1024 map with the `Hardware2x2` filter, textures use 4×
anisotropy, 5×5 chunks are loaded with fog at 45–115 m; that measured 50–65 fps (vsync 60), against ~22 fps with
MSAA 4×, 3–4 cascades and 7×7 chunks. G toggles shadows in game; the HUD shows fps. Measure before raising any of these.

## Rules

- Work and temp files go to the git-ignored `var/`, never the repo root.
- One commit per logical change, short imperative subject.
