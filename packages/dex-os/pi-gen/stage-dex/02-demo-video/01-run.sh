#!/bin/bash -e

EXAMPLE_FILE="files/dex-test-card-2s-1080p-h264.mp4.h264"

mkdir -p "${ROOTFS_DIR}/dexdata/"

# deference if its a symlink
if test -L "$EXAMPLE_FILE"; then
  cp -L "$EXAMPLE_FILE" "$EXAMPLE_FILE".tmp && mv "$EXAMPLE_FILE".tmp "$EXAMPLE_FILE"
fi

install files/dex-test-card-2s-1080p-h264.mp4.h264 "${ROOTFS_DIR}/dexdata/"
