#!/bin/bash
set -euo pipefail
WD="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$WD"

docker stop dex-os-apt-cacher-ng || true
docker rm dex-os-apt-cacher-ng || true

docker stop pigen_work || true
docker rm pigen_work || true

echo "[dev-reset] OK"
