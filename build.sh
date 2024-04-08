#!/bin/bash
set -exuo pipefail

PI_GEN_DIR="$(pwd)/packages/pi-gen"

docker rm -v pigen_work || true

cp ./pi-gen-config.env "${PI_GEN_DIR}/config"

cd "$PI_GEN_DIR"

# dont delete the container after build (for debugging and inceremental builds)
export PRESERVE_CONTAINER=1

docker-compose up -d apt-cacher-ng
echo 'APT_PROXY=http://host.docker.internal:3142' >> config

cat ./config

# get git hash from monorepo
GIT_HASH="$(git rev-parse HEAD)"
export GIT_HASH

./build-docker.sh
