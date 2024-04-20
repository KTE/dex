#!/bin/bash
set -euo pipefail
WD="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$WD"

./dev-reset.sh
./prepare.sh
./build.sh

echo "[dist] OK"
