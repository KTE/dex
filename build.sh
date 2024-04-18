#!/bin/bash
set -euo pipefail


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

BUILD_DATE=$(date -u +'%Y-%m-%dT%H-%M-%S')
DIST_DIR="${WD}/dist/deploy-${BUILD_DATE}"
test -e "$DIST_DIR" && { echo "Error: dist directory already exists: $DIST_DIR"; exit 1; }
echo "DIST_DIR: $DIST_DIR"

# rpi-gen build container
export CONTAINER_NAME="pigen_work"
# pin apt-cacher-ng container image
APT_CACHER_NG_CONTAINER='sameersbn/apt-cacher-ng@sha256:6d612ae08493af17eb5682cf0b29d75c18fd6455e786239fa63fe56ebca552fa'
# DEBUG: dont delete the container after building)
# export PRESERVE_CONTAINER=1
APT_CACHER_NG_CONTAINER='sameersbn/apt-cacher-ng@sha256:6d612ae08493af17eb5682cf0b29d75c18fd6455e786239fa63fe56ebca552fa'

# work #######################################################################
cd "$WD"

# * delete dist dir on any exit, but only if its not empty
# * stop apt-cacher-ng container on exit
trap '{ rmdir "$DIST_DIR" 2>/dev/null; docker stop apt-cacher-ng ;} || true' EXIT SIGINT SIGTERM

# clean slate
mkdir -p "$DIST_DIR" || { echo "Failed to create dist directory"; exit 1; }
if [ "${PRESERVE_CONTAINER:-0}" != "1" ]; then
  docker rm -v "${CONTAINER_NAME}" || true
fi
rm -rf "$PI_GEN_DEPLOY_DIR" || { echo "Failed to remove old pi-gen/deploy directory"; exit 1; }

cp ./pi-gen-config.env "${PI_GEN_DIR}/config"

# apt-cacher-ng: start container if no cache already configured or disabled with "0"
APT_CACHE=${APT_CACHE:-}
if test -z "$APT_CACHE" || test "$APT_CACHE" != 0; then
  if uname -o | grep Darwin; then
    # macOS
    APT_CACHER_URL=http://host.docker.internal:3142
  else
    # linux
    APT_CACHER_URL=http://172.17.0.2:3142
  fi
  if [ -n "${APT_CACHER_URL}" ]; then
    docker run --rm --init -d --name apt-cacher-ng \
    --publish 3142:3142 \
    --volume ./tmp/apt-cacher-ng:/var/cache/apt-cacher-ng \
    "$APT_CACHER_NG_CONTAINER"
    echo "APT_PROXY=\"${APT_CACHER_URL}\"" >> "${PI_GEN_DIR}/config"
  fi
fi

cd "$PI_GEN_DIR"
cat ./config

# get git hash from monorepo
GIT_HASH="$(git rev-parse HEAD)"
export GIT_HASH

./build-docker.sh

mv ./deploy/* "${DIST_DIR}/"

clear
echo "🎉 output in ${DIST_DIR}:"
ls -lah "${DIST_DIR}"
