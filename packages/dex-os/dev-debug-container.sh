#!/bin/bash
set -euo pipefail
WD="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$WD"

docker start pigen_work || true
docker exec -it pigen_work /bin/bash
