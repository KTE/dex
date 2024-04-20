#!/bin/bash
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

git submodule update --recursive --init --force packages/pi-gen
quilt pop -af || true
quilt push -av

