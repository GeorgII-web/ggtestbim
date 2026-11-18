# ggtestbim

Personal 3D game in Rust on [Bevy](https://bevyengine.org) 0.20, target Windows x86_64.

## Build

- The repo lives in WSL; the game is cross-built to a Windows exe with MinGW (no Windows Rust toolchain needed).
- `scripts/cargo-gnu.sh build [--release]` → `target/x86_64-pc-windows-gnu/<profile>/ggtestbim.exe`.
- `scripts/run-win.sh [--release]` builds, copies the exe (and `assets/`) to `C:\dev\ggtestbim` and starts it.
- Plain `cargo build` (Linux) needs `pkg-config`, `libudev-dev`, `libasound2-dev`, `libwayland-dev`; not set up.
- Gates before a commit: `cargo fmt --check`, `scripts/cargo-gnu.sh clippy -- -D warnings`.

## Rules

- Work and temp files go to the git-ignored `var/`, never the repo root.
- One commit per logical change, short imperative subject.
