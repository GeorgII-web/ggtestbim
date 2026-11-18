#!/usr/bin/env bash
# Builds the Windows exe in WSL and starts it on Windows from a local folder (C:\dev\ggtestbim),
# because running from \\wsl.localhost triggers the security prompt.
# Usage: scripts/run-win.sh [--release]
set -euo pipefail
cd "$(dirname "$0")/.."
profile=debug; [ "${1:-}" = "--release" ] && profile=release
scripts/cargo-gnu.sh build ${1:+"$1"}
dest=/mnt/c/dev/ggtestbim
mkdir -p "$dest"
cp "target/x86_64-pc-windows-gnu/$profile/ggtestbim.exe" "$dest/"
[ -d assets ] && cp -r assets "$dest/"
cd "$dest" && powershell.exe -NoProfile -Command "Start-Process -FilePath .\\ggtestbim.exe -WorkingDirectory C:\\dev\\ggtestbim"
