#!/bin/bash
set -exuo pipefail


# helpers ######################################################################
WD="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

increment_trailing_number() { # increment last number in string by 1, keeping at least 3 digits (foo-001, foo-002, ...)
  local prefix number
  if [[ $1 =~ ^(.*[^[:digit:]])([[:digit:]]+)$ ]]; then
    prefix=${BASH_REMATCH[1]}; number=${BASH_REMATCH[2]}
    printf '%s%03d\n' "$prefix" "$(( number + 1 ))"
  else
    echo 'invalid input'; return 1;
  fi
}

# config #######################################################################

PI_GEN_DIR="$(pwd)/packages/pi-gen"
PI_GEN_DEPLOY_DIR="${PI_GEN_DIR}/deploy"

BUILD_DATE=$(date -u +'%Y-%m-%d')
DIST_DIR_PREFIX="${WD}/dist/deploy-${BUILD_DATE}-"
LATEST_DAILY_BUILD="$(find "$DIST_DIR_PREFIX"* -type d 2>/dev/null | tail -1 || true)"
if test -n "$LATEST_DAILY_BUILD"; then
  DIST_DIR="$(increment_trailing_number "$LATEST_DAILY_BUILD")"
else
 DIST_DIR="${DIST_DIR_PREFIX}001"
fi
echo "DIST_DIR: $DIST_DIR"


# rpi-gen build container
export CONTAINER_NAME="pigen_work"
# DEBUG: dont delete the container after building)
# export PRESERVE_CONTAINER=1

# work #######################################################################
cd "$WD"

trap 'rmdir "$DIST_DIR" || true' EXIT SIGINT SIGTERM

# clean slate
mkdir -p "$DIST_DIR" || { echo "Failed to create dist directory"; exit 1; }
if [ "${PRESERVE_CONTAINER:-0}" != "1" ]; then
  docker rm -v "${CONTAINER_NAME}" || true
fi
rm -rf "$PI_GEN_DEPLOY_DIR" || { echo "Failed to remove old pi-gen/deploy directory"; exit 1; }

cp ./pi-gen-config.env "${PI_GEN_DIR}/config"

cd "$PI_GEN_DIR"

docker-compose up -d apt-cacher-ng
echo 'APT_PROXY=http://host.docker.internal:3142' >> config

cat ./config

# get git hash from monorepo
GIT_HASH="$(git rev-parse HEAD)"
export GIT_HASH

./build-docker.sh

mv ./deploy/* "${DIST_DIR}/"

echo "🎉 output in ${DIST_DIR}:"
ls -lah "${DIST_DIR}"
