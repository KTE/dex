#!/bin/bash
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

git submodule update --recursive --init --force --depth=1 packages/pi-gen packages/pi_video_looper packages/example-content

quilt pop -af || true
quilt push -av

echo "[prepare] OK"
